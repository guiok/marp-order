//! Slide thumbnails, rendered by marp-cli in the background.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::App;

/// A request to the thumbnail renderer.
pub enum Job {
    /// Render the open file with this number, as it is on disk.
    Disk(u64),
    /// Render a draft (order and edits not saved yet) and keep the images of the slides
    /// listed in `wanted`: (position from 0, key of the text).
    Draft {
        doc: u64,
        generation: u64,
        markdown: String,
        total: usize,
        wanted: Vec<(usize, String)>,
    },
}

pub struct Config {
    /// The marp-cli command, already split into program + arguments.
    pub cmd: Vec<String>,
    /// Extra arguments from the user (e.g. `--theme-set theme.css`).
    pub extra: Vec<String>,
}

/// Picks marp-cli: the given command, `marp` in PATH or, as a last resort, `npx`.
pub fn resolve_cmd(user: Option<&str>) -> Option<Vec<String>> {
    if let Some(cmd) = user {
        let parts: Vec<String> = cmd.split_whitespace().map(String::from).collect();
        return (!parts.is_empty()).then_some(parts);
    }
    // on Windows npm installs commands as `.cmd` scripts
    let (marp, npx) = if cfg!(windows) { ("marp.cmd", "npx.cmd") } else { ("marp", "npx") };
    if in_path(marp) {
        return Some(vec![marp.into()]);
    }
    if in_path(npx) {
        return Some(vec![
            npx.into(),
            "--yes".into(),
            "@marp-team/marp-cli@latest".into(),
        ]);
    }
    None
}

fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

/// Starts the renderer. Each message carries the number of an open file and asks for a
/// new render of what is on disk.
pub fn spawn(app: Arc<Mutex<App>>, config: Config) -> Sender<Job> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || worker(app, config, rx));
    tx
}

fn worker(app: Arc<Mutex<App>>, config: Config, rx: Receiver<Job>) {
    let mut pending: Vec<Job> = Vec::new();
    loop {
        if pending.is_empty() {
            match rx.recv() {
                Ok(job) => pending.push(job),
                Err(_) => return,
            }
        }
        // queued requests for the same file collapse into one; for drafts the last one wins
        for job in rx.try_iter() {
            pending.retain(|old| match (old, &job) {
                (Job::Disk(a), Job::Disk(b)) => a != b,
                (Job::Draft { doc: a, .. }, Job::Draft { doc: b, .. }) => a != b,
                _ => true,
            });
            pending.push(job);
        }
        let id = match pending.remove(0) {
            Job::Disk(id) => id,
            Job::Draft { doc, generation, markdown, total, wanted } => {
                draft(&app, &config, doc, generation, &markdown, total, &wanted);
                continue;
            }
        };
        loop {
            let job = {
                let mut app = app.lock().unwrap();
                app.doc(id).map(|doc| {
                    doc.thumb_state = "rendering";
                    doc.thumb_msg.clear();
                    (
                        doc.generation,
                        doc.save_seq,
                        doc.disk_order.clone(),
                        doc.path.clone(),
                        doc.thumb_dir.clone(),
                        doc.content_hash(),
                    )
                })
            };
            let Some((generation, save_seq, order, path, dir, content)) = job else {
                break; // the file was closed
            };
            let work = dir.join("render");
            let result = if order.is_empty() {
                Ok(Vec::new())
            } else {
                render(&config, &path, &work)
            };

            let mut app = app.lock().unwrap();
            let cache = app.cache.clone();
            let Some(app) = app.doc(id) else {
                break;
            };
            if app.generation != generation || app.save_seq != save_seq {
                continue; // the file changed while rendering
            }
            match result {
                Ok(images) if images.len() == order.len() => {
                    for (image, &id) in images.iter().zip(&order) {
                        let target = dir.join(format!("{id}.jpeg"));
                        if fs::rename(image, &target).is_ok() {
                            app.has_thumb[id] = true;
                        }
                    }
                    app.thumb_state = "ready";
                    if let Some(cache) = cache {
                        // one image per slide in file order, for the next time it is opened
                        let kept: Vec<PathBuf> =
                            order.iter().map(|id| dir.join(format!("{id}.jpeg"))).collect();
                        cache.store(&path, content, &kept);
                    }
                }
                Ok(images) => {
                    // Marp split the deck differently from us: the mapping cannot be trusted.
                    app.has_thumb.iter_mut().for_each(|h| *h = false);
                    app.thumb_state = "mismatch";
                    app.thumb_msg = format!(
                        "Marp rendered {} slides, but marp-order found {}",
                        images.len(),
                        order.len()
                    );
                }
                Err(message) => {
                    app.thumb_state = "error";
                    app.thumb_msg = message;
                }
            }
            app.thumb_version += 1;
            let _ = fs::remove_dir_all(&work);
            break;
        }
    }
}

/// Thumbnails of slides that are not saved yet. The draft is written next to the deck,
/// so images with relative paths show up, and deleted right after.
fn draft(
    app: &Mutex<App>,
    config: &Config,
    id: u64,
    generation: u64,
    markdown: &str,
    total: usize,
    wanted: &[(usize, String)],
) {
    let Some((path, dir)) = app
        .lock()
        .unwrap()
        .doc(id)
        .filter(|doc| doc.generation == generation)
        .map(|doc| (doc.path.clone(), doc.thumb_dir.clone()))
    else {
        return;
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let file = path.with_file_name(format!(".{name}.marp-order.draft.md"));
    if fs::write(&file, markdown).is_err() {
        return;
    }
    let work = dir.join("draft");
    let images = render(config, &file, &work);
    let _ = fs::remove_file(&file);

    let mut app = app.lock().unwrap();
    if let (Ok(images), Some(doc)) = (images, app.doc(id)) {
        // if Marp splits the draft differently, the positions cannot be trusted
        if doc.generation == generation && images.len() == total {
            for (at, key) in wanted {
                let target = dir.join(format!("d-{key}.jpeg"));
                if fs::rename(&images[*at], &target).is_ok() && !doc.drafts.contains(key) {
                    doc.drafts.push(key.clone());
                }
            }
        }
    }
    let _ = fs::remove_dir_all(&work);
}

/// Renders the deck and returns the images in slide order.
fn render(config: &Config, deck: &Path, work: &Path) -> Result<Vec<PathBuf>, String> {
    let _ = fs::remove_dir_all(work);
    fs::create_dir_all(work).map_err(|e| e.to_string())?;

    let output = Command::new(&config.cmd[0])
        .args(&config.cmd[1..])
        .args(["--no-stdin", "--allow-local-files"])
        .args(["--images", "jpeg", "--image-scale", "0.5", "--jpeg-quality", "80"])
        .arg("-o")
        .arg(work.join("s.jpeg"))
        .args(&config.extra)
        .arg(deck)
        .current_dir(deck.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run `{}`: {e}", config.cmd.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(3)..].join(" | ");
        return Err(format!("marp-cli failed: {tail}"));
    }

    let mut images: Vec<PathBuf> = fs::read_dir(work)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "jpeg" || ext == "jpg"))
        .collect();
    images.sort();
    Ok(images)
}

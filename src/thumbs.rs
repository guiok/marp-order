//! Miniaturas das lâminas, geradas pelo marp-cli em segundo plano.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::App;

pub struct Config {
    /// Comando do marp-cli já dividido em programa + argumentos.
    pub cmd: Vec<String>,
    /// Argumentos extras do usuário (ex.: `--theme-set tema.css`).
    pub extra: Vec<String>,
}

/// Escolhe o marp-cli: o comando informado, `marp` no PATH ou, por último, `npx`.
pub fn resolve_cmd(user: Option<&str>) -> Option<Vec<String>> {
    if let Some(cmd) = user {
        let parts: Vec<String> = cmd.split_whitespace().map(String::from).collect();
        return (!parts.is_empty()).then_some(parts);
    }
    if in_path("marp") {
        return Some(vec!["marp".into()]);
    }
    if in_path("npx") {
        return Some(vec![
            "npx".into(),
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

/// Inicia o gerador. Cada mensagem traz o número de um arquivo aberto e pede uma nova
/// renderização do que está em disco.
pub fn spawn(app: Arc<Mutex<App>>, config: Config) -> Sender<u64> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || worker(app, config, rx));
    tx
}

fn worker(app: Arc<Mutex<App>>, config: Config, rx: Receiver<u64>) {
    let mut pending: Vec<u64> = Vec::new();
    loop {
        if pending.is_empty() {
            match rx.recv() {
                Ok(id) => pending.push(id),
                Err(_) => return,
            }
        }
        // pedidos acumulados para o mesmo arquivo viram um só
        for id in rx.try_iter() {
            if !pending.contains(&id) {
                pending.push(id);
            }
        }
        let id = pending.remove(0);
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
                    )
                })
            };
            let Some((generation, save_seq, order, path, dir)) = job else {
                break; // o arquivo foi fechado
            };
            let work = dir.join("render");
            let result = if order.is_empty() {
                Ok(Vec::new())
            } else {
                render(&config, &path, &work)
            };

            let mut app = app.lock().unwrap();
            let Some(app) = app.doc(id) else {
                break;
            };
            if app.generation != generation || app.save_seq != save_seq {
                continue; // o arquivo mudou durante a renderização
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
                }
                Ok(images) => {
                    // A divisão do Marp não bate com a nossa: não dá para confiar no mapeamento.
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

/// Renderiza o deck e devolve as imagens na ordem das lâminas.
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

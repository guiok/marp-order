//! Live preview for the slide editor.
//!
//! A `marp --watch` keeps running on a draft of the deck in a temporary folder; on every
//! change the draft is rewritten and marp-cli reconverts the HTML in a fraction of the
//! time of a one-off run.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Runs the given command and stops it when standard input closes, that is, when
/// marp-order exits for any reason. Without it `marp --watch` would be left orphaned.
#[cfg(unix)]
const SUPERVISE: &str =
    r#"exec 3<&0; "$@" & pid=$!; (cat <&3 >/dev/null; kill $pid 2>/dev/null) & wait $pid"#;

pub struct Preview {
    cmd: Vec<String>,
    extra: Vec<String>,
    dir: PathBuf,
    watcher: Option<Child>,
    /// Folder of the deck being previewed; marp-cli runs in it.
    root: Option<PathBuf>,
    nonce: u64,
}

impl Preview {
    pub fn new(cmd: Vec<String>, extra: Vec<String>, dir: PathBuf) -> Preview {
        Preview {
            cmd,
            extra,
            dir,
            watcher: None,
            root: None,
            nonce: 0,
        }
    }

    /// Folder the images and fonts of the current draft are served from.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.watcher.take() {
            drop(child.stdin.take());
            // without the supervisor (outside Unix) marp-cli does not exit by itself
            #[cfg(not(unix))]
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Command that starts marp-cli. On Unix it runs under the `sh` supervisor; elsewhere
    /// it runs directly and is stopped by `stop`.
    #[cfg(unix)]
    fn watcher_command(&self) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", SUPERVISE, "marp-order"]).args(&self.cmd);
        command
    }

    #[cfg(not(unix))]
    fn watcher_command(&self) -> Command {
        let mut command = Command::new(&self.cmd[0]);
        command.args(&self.cmd[1..]);
        command
    }

    fn html_path(&self) -> PathBuf {
        self.dir.join("draft.html")
    }

    /// Converts `markdown` and returns only when the matching HTML is ready.
    pub fn render(&mut self, deck: &Path, markdown: &str) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let draft = self.dir.join("draft.md");
        let html = self.html_path();
        let root = deck.parent().unwrap_or(Path::new(".")).to_path_buf();
        if self.root.as_ref() != Some(&root) {
            // another file: relative paths of the theme and images have a new base
            self.stop();
            self.root = Some(root.clone());
        }

        // marp-cli may write the HTML of an earlier version right after our write; the
        // marker below, which ends up in the page's CSS, identifies the right version.
        self.nonce += 1;
        let token = format!("mo-draft-{}", self.nonce);
        let content = format!("{markdown}\n\n<style>section.{token}{{--mo:0}}</style>\n");

        let running = matches!(self.watcher.as_mut().map(Child::try_wait), Some(Ok(None)));
        let limit = if running {
            fs::write(&draft, &content).map_err(|e| e.to_string())?;
            Duration::from_secs(6)
        } else {
            let _ = fs::remove_file(&html);
            fs::write(&draft, &content).map_err(|e| e.to_string())?;
            let child = self
                .watcher_command()
                .args(["--no-stdin", "--watch", "--template", "bare", "-o"])
                .arg(&html)
                .args(&self.extra)
                .arg(&draft)
                .current_dir(&root)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("could not run `{}`: {e}", self.cmd.join(" ")))?;
            self.watcher = Some(child);
            Duration::from_secs(30)
        };

        let started = Instant::now();
        let mut written = started;
        loop {
            if let Ok(page) = fs::read_to_string(&html) {
                if page.contains(&token) && page.trim_end().ends_with("</html>") {
                    return Ok(());
                }
            }
            if let Some(Ok(Some(_))) = self.watcher.as_mut().map(Child::try_wait) {
                self.watcher = None;
                return Err("marp-cli exited without producing the preview".into());
            }
            if started.elapsed() > limit {
                return Err("marp-cli did not respond in time".into());
            }
            if written.elapsed() > Duration::from_millis(700) {
                // marp-cli misses changes made while it is still starting or converting;
                // rewriting the draft triggers the conversion again.
                let _ = fs::write(&draft, &content);
                written = Instant::now();
            }
            thread::sleep(Duration::from_millis(15));
        }
    }

    /// Page of the draft showing only slide `page` (from 1).
    pub fn page(&self, page: usize) -> Option<String> {
        let mut html = fs::read_to_string(self.html_path()).ok()?;
        // --watch mode embeds a script that reloads the page over a websocket; the editor
        // swaps versions itself, so the script is removed
        if let Some(socket) = html.find("ws://") {
            let open = html[..socket].rfind("<script");
            let close = html[socket..].find("</script>").map(|i| socket + i + "</script>".len());
            if let (Some(open), Some(close)) = (open, close) {
                html.replace_range(open..close, "");
            }
        }
        let style = format!(
            "<style>html,body{{margin:0;height:100%;overflow:hidden;background:#fff}}\
             body>svg[data-marpit-svg]{{display:none;width:100vw;height:100vh}}\
             body>svg[data-marpit-svg]:nth-of-type({page}){{display:block}}</style></head>"
        );
        Some(html.replacen("</head>", &style, 1))
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.stop();
    }
}

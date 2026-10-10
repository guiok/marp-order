//! Pré-visualização ao vivo do editor de lâminas.
//!
//! Um `marp --watch` fica rodando sobre um rascunho do deck em uma pasta temporária;
//! a cada alteração o rascunho é regravado e o marp-cli reconverte o HTML em uma fração
//! do tempo de uma execução avulsa.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Roda o comando recebido e o encerra quando a entrada padrão fechar, isto é, quando o
/// marp-order terminar por qualquer motivo. Sem isso o `marp --watch` ficaria órfão.
const SUPERVISE: &str =
    r#"exec 3<&0; "$@" & pid=$!; (cat <&3 >/dev/null; kill $pid 2>/dev/null) & wait $pid"#;

pub struct Preview {
    cmd: Vec<String>,
    extra: Vec<String>,
    dir: PathBuf,
    watcher: Option<Child>,
    nonce: u64,
}

impl Preview {
    pub fn new(cmd: Vec<String>, extra: Vec<String>, dir: PathBuf) -> Preview {
        Preview {
            cmd,
            extra,
            dir,
            watcher: None,
            nonce: 0,
        }
    }

    fn html_path(&self) -> PathBuf {
        self.dir.join("draft.html")
    }

    /// Converte `markdown` e só retorna quando o HTML correspondente estiver pronto.
    pub fn render(&mut self, deck: &Path, markdown: &str) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let draft = self.dir.join("draft.md");
        let html = self.html_path();

        // O marp-cli pode gravar um HTML de uma versão anterior logo depois da nossa
        // escrita; a marca abaixo, que sai no CSS da página, identifica a versão certa.
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
            let child = Command::new("sh")
                .args(["-c", SUPERVISE, "marp-order"])
                .args(&self.cmd)
                .args(["--no-stdin", "--watch", "--template", "bare", "-o"])
                .arg(&html)
                .args(&self.extra)
                .arg(&draft)
                .current_dir(deck.parent().unwrap_or(Path::new(".")))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("não foi possível executar `{}`: {e}", self.cmd.join(" ")))?;
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
                return Err("o marp-cli encerrou sem gerar a pré-visualização".into());
            }
            if started.elapsed() > limit {
                return Err("o marp-cli não respondeu a tempo".into());
            }
            if written.elapsed() > Duration::from_millis(700) {
                // O marp-cli perde alterações feitas enquanto ainda está iniciando ou
                // convertendo; regravar o rascunho dispara a conversão de novo.
                let _ = fs::write(&draft, &content);
                written = Instant::now();
            }
            thread::sleep(Duration::from_millis(15));
        }
    }

    /// Página do rascunho mostrando apenas a lâmina `page` (a partir de 1).
    pub fn page(&self, page: usize) -> Option<String> {
        let mut html = fs::read_to_string(self.html_path()).ok()?;
        // o modo --watch embute um script que recarrega a página por websocket; a troca
        // de versões é feita pelo editor, então ele sai
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
        if let Some(mut child) = self.watcher.take() {
            drop(child.stdin.take());
            let _ = child.wait();
        }
    }
}

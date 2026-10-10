mod deck;
mod preview;
mod thumbs;

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use deck::{Deck, Entry};
use preview::Preview;

const UI: &str = include_str!("ui.html");

const USAGE: &str = "\
Usage: marp-order [options] <presentation.md> [-- marp-cli arguments]

Opens a grid with the slides of a Marp presentation in the browser so you
can rearrange them by dragging. Saving rewrites the .md file in the new order.

Options:
  --port <n>         local server port (default: chosen by the system)
  --no-open          do not open the browser automatically
  --no-thumbs        do not render thumbnails (show only the slide text)
  --marp-cmd <cmd>   marp-cli command (default: `marp` or `npx @marp-team/marp-cli`)
  -h, --help         show this help
  -V, --version      show the version

Example with a custom theme:
  marp-order talk.md -- --theme-set theme.css";

struct Args {
    file: PathBuf,
    port: u16,
    open: bool,
    thumbs: bool,
    marp_cmd: Option<String>,
    marp_extra: Vec<String>,
}

pub struct App {
    pub path: PathBuf,
    deck: Deck,
    /// Conteúdo que lemos ou gravamos por último; se o disco diferir, alguém mexeu no arquivo.
    last_disk: Vec<u8>,
    last_meta: Option<(u64, SystemTime)>,
    /// Ordem das lâminas (índices de `deck.slides`) atualmente gravada no arquivo.
    pub disk_order: Vec<usize>,
    /// Muda a cada recarga do arquivo; invalida os índices conhecidos pelo navegador.
    pub generation: u64,
    pub save_seq: u64,
    last_backup: Option<PathBuf>,
    thumb_dir: PathBuf,
    pub has_thumb: Vec<bool>,
    pub thumb_version: u64,
    pub thumb_state: &'static str,
    pub thumb_msg: String,
    /// Pré-visualização do editor; ausente quando não há marp-cli.
    preview: Option<Preview>,
}

enum SaveError {
    ExternalChange,
    Other(String),
}

impl App {
    fn external_change(&mut self) -> bool {
        let meta = file_signature(&self.path);
        if meta.is_some() && meta == self.last_meta {
            return false;
        }
        match fs::read(&self.path) {
            Ok(bytes) if bytes == self.last_disk => {
                self.last_meta = meta;
                false
            }
            _ => true,
        }
    }

    fn save(&mut self, entries: Vec<Entry>, force: bool) -> Result<(), SaveError> {
        let changed = self.external_change();
        if changed && !force {
            return Err(SaveError::ExternalChange);
        }
        // O backup guarda o que está prestes a ser sobrescrito pela primeira vez.
        if self.last_backup.is_none() || changed {
            let backup = free_backup_path(&self.path);
            fs::copy(&self.path, &backup)
                .map_err(|e| SaveError::Other(format!("could not create the backup: {e}")))?;
            self.last_backup = Some(backup);
        }
        let content = self.deck.compose(&entries);
        write_atomic(&self.path, content.as_bytes())
            .map_err(|e| SaveError::Other(format!("could not write the file: {e}")))?;
        self.save_seq += 1;
        if entries.iter().any(|(_, edit)| edit.is_some()) {
            // Uma edição pode criar ou fundir lâminas: o arquivo gravado vira a nova base.
            self.adopt(Deck::parse(content), &entries);
        } else {
            self.last_disk = content.into_bytes();
            self.last_meta = file_signature(&self.path);
            self.disk_order = entries.into_iter().map(|(id, _)| id).collect();
        }
        Ok(())
    }

    fn reload(&mut self) -> Result<(), String> {
        let deck = load(&self.path)?;
        self.adopt(deck, &[]);
        Ok(())
    }

    /// Troca o deck em memória pelo conteúdo que está no disco. `origin` diz de quais
    /// lâminas antigas as novas vieram, para reaproveitar as miniaturas das intactas.
    fn adopt(&mut self, deck: Deck, origin: &[Entry]) {
        let count = deck.slides.len();
        let mut has_thumb = vec![false; count];
        if origin.len() == count {
            let file = |name: String| self.thumb_dir.join(name);
            // em duas etapas, porque os números novos e antigos se sobrepõem
            for (new, (old, edit)) in origin.iter().enumerate() {
                has_thumb[new] = edit.is_none()
                    && self.has_thumb[*old]
                    && fs::copy(file(format!("{old}.jpeg")), file(format!("new-{new}.jpeg"))).is_ok();
            }
            for (new, has) in has_thumb.iter_mut().enumerate() {
                *has = *has
                    && fs::rename(file(format!("new-{new}.jpeg")), file(format!("{new}.jpeg"))).is_ok();
            }
        }
        self.has_thumb = has_thumb;
        self.disk_order = (0..count).collect();
        self.last_disk = deck.source.clone().into_bytes();
        self.last_meta = file_signature(&self.path);
        self.deck = deck;
        self.generation += 1;
        self.thumb_version += 1;
    }

    /// Converte o rascunho para a pré-visualização e devolve a página da lâmina `index`.
    fn preview(&mut self, entries: &[Entry], index: usize) -> Result<usize, String> {
        let markdown = self.deck.compose(entries);
        let page = match index.min(entries.len()) {
            0 => 1,
            index => self.deck.count(&entries[..index]) + 1,
        };
        let preview = self
            .preview
            .as_mut()
            .ok_or("preview unavailable: marp-cli not found")?;
        preview.render(&self.path, &markdown)?;
        Ok(page)
    }

    fn status_json(&mut self) -> Value {
        json!({
            "generation": self.generation,
            "external_change": self.external_change(),
            "disk_order": self.disk_order,
            "backup": self.last_backup.as_ref().map(|p| file_name(p)),
            "thumbs": {
                "state": self.thumb_state,
                "message": self.thumb_msg,
                "version": self.thumb_version,
                "have": self.has_thumb,
            },
        })
    }

    fn state_json(&mut self) -> Value {
        let slides: Vec<Value> = self
            .deck
            .slides
            .iter()
            .enumerate()
            .map(|(id, slide)| {
                let mut warn = Vec::new();
                let directives = deck::forward_directives(&slide.text);
                if !directives.is_empty() {
                    warn.push(format!(
                        "Directive that applies from this slide onwards: {}. Moving this slide changes which slides are affected.",
                        directives.join(", ")
                    ));
                }
                if slide.lead_comment {
                    warn.push(
                        "With headingDivider, the comment before the heading moves to the previous slide if this one stops being the first."
                            .to_string(),
                    );
                }
                json!({
                    "id": id,
                    "title": deck::title(&slide.text),
                    "text": slide.text.trim(),
                    "warn": warn,
                })
            })
            .collect();
        let mut state = self.status_json();
        state["file"] = json!(file_name(&self.path));
        state["path"] = json!(self.path.display().to_string());
        state["slides"] = json!(slides);
        state
    }
}

fn load(path: &Path) -> Result<Deck, String> {
    let bytes = fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("{} is not a valid UTF-8 file", path.display()))?;
    Ok(Deck::parse(text))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn file_signature(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Primeiro nome livre entre `arquivo.md.bak`, `arquivo.md.bak1`, ...
fn free_backup_path(path: &Path) -> PathBuf {
    let base = format!("{}.bak", path.display());
    (0..)
        .map(|n| match n {
            0 => PathBuf::from(&base),
            n => PathBuf::from(format!("{base}{n}")),
        })
        .find(|candidate| !candidate.exists())
        .unwrap()
}

/// Grava em um temporário ao lado e renomeia; se o sistema de arquivos recusar, grava direto.
fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_file_name(format!(".{}.marp-order.tmp", file_name(path)));
    let attempt = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, path)
    })();
    if attempt.is_err() {
        let _ = fs::remove_file(&tmp);
        fs::write(path, data)?;
    }
    Ok(())
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        file: PathBuf::new(),
        port: 0,
        open: true,
        thumbs: true,
        marp_cmd: std::env::var("MARP_ORDER_MARP_CMD").ok(),
        marp_extra: Vec::new(),
    };
    let mut file = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("marp-order {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--no-open" => args.open = false,
            "--no-thumbs" => args.thumbs = false,
            "--port" => {
                args.port = it
                    .next()
                    .and_then(|p| p.parse().ok())
                    .ok_or("--port needs a port number")?;
            }
            "--marp-cmd" => {
                args.marp_cmd = Some(it.next().ok_or("--marp-cmd needs a command")?);
            }
            "--" => {
                args.marp_extra = it.by_ref().collect();
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            _ if file.is_some() => return Err("give only one file".into()),
            _ => file = Some(PathBuf::from(arg)),
        }
    }
    args.file = file.ok_or("give the presentation .md file")?;
    Ok(args)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("marp-order: {message}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    // Caminho real: gravar por rename não pode trocar um link simbólico por um arquivo.
    let path = fs::canonicalize(&args.file)
        .map_err(|e| format!("could not open {}: {e}", args.file.display()))?;
    let deck = load(&path)?;
    let count = deck.slides.len();

    let thumb_dir = std::env::temp_dir().join(format!("marp-order-{}", std::process::id()));
    // o marp-cli serve às miniaturas e à pré-visualização do editor
    let marp_cmd = thumbs::resolve_cmd(args.marp_cmd.as_deref());
    let (thumb_state, thumb_msg) = match (&marp_cmd, args.thumbs) {
        (_, false) => ("off", String::new()),
        (Some(_), true) => ("rendering", String::new()),
        (None, true) => ("error", "marp-cli not found (install `marp` or `npx`)".into()),
    };

    let preview = marp_cmd.as_ref().map(|cmd| {
        Preview::new(cmd.clone(), args.marp_extra.clone(), thumb_dir.join("preview"))
    });

    let app = Arc::new(Mutex::new(App {
        last_meta: file_signature(&path),
        path: path.clone(),
        last_disk: deck.source.clone().into_bytes(),
        deck,
        disk_order: (0..count).collect(),
        generation: 1,
        save_seq: 0,
        last_backup: None,
        thumb_dir: thumb_dir.clone(),
        has_thumb: vec![false; count],
        thumb_version: 0,
        thumb_state,
        thumb_msg,
        preview,
    }));

    let render = marp_cmd.filter(|_| args.thumbs).map(|cmd| {
        let config = thumbs::Config {
            cmd,
            extra: args.marp_extra,
            dir: thumb_dir,
        };
        let tx = thumbs::spawn(app.clone(), config);
        let _ = tx.send(());
        tx
    });

    let server = Server::http(("127.0.0.1", args.port))
        .map_err(|e| format!("could not start the server: {e}"))?;
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(args.port);
    let url = format!("http://127.0.0.1:{port}/");

    println!("marp-order: {} — {count} slides", path.display());
    println!("Open {url}  (Ctrl+C to stop)");
    if args.open {
        let _ = Command::new("xdg-open")
            .arg(&url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    for request in server.incoming_requests() {
        handle(request, &app, render.as_ref(), port);
    }
    Ok(())
}

fn respond_json(request: Request, status: u16, body: Value) {
    let header = Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap();
    let response = Response::from_string(body.to_string())
        .with_status_code(status)
        .with_header(header);
    let _ = request.respond(response);
}

fn respond_error(request: Request, status: u16, message: &str) {
    respond_json(request, status, json!({ "error": message }));
}

fn header_value(request: &Request, name: &'static str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.to_string())
}

/// Lê `[{"id": n, "text": "..."}]`; `text` só vem nas lâminas editadas.
fn parse_entries(value: &Value, slide_count: usize) -> Option<Vec<Entry>> {
    value
        .as_array()?
        .iter()
        .map(|item| {
            let id = item["id"].as_u64()? as usize;
            (id < slide_count).then(|| (id, item["text"].as_str().map(String::from)))
        })
        .collect()
}

/// Arquivo da pasta do deck (ou abaixo dela) pedido pela pré-visualização.
fn deck_asset(deck: &Path, url_path: &str) -> Option<(Vec<u8>, &'static str)> {
    let root = deck.parent()?;
    let file = fs::canonicalize(root.join(percent_decode(url_path))).ok()?;
    if !file.starts_with(root) {
        return None;
    }
    let extension = file.extension()?.to_str()?.to_ascii_lowercase();
    let kind = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "css" => "text/css",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => return None,
    };
    Some((fs::read(file).ok()?, kind))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn handle(mut request: Request, app: &Mutex<App>, render: Option<&Sender<()>>, port: u16) {
    // Só o próprio navegador local fala com o servidor: barra DNS rebinding e POSTs de outros sites.
    let host = header_value(&request, "Host").unwrap_or_default();
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return respond_error(request, 403, "invalid host");
    }
    let is_post = *request.method() == Method::Post;
    if is_post && header_value(&request, "X-Marp-Order").is_none() {
        return respond_error(request, 403, "unauthorized request");
    }
    if !is_post && *request.method() != Method::Get {
        return respond_error(request, 405, "method not supported");
    }

    let url = request.url().to_string();
    let route = url.split('?').next().unwrap_or("");
    let mut app = app.lock().unwrap();

    match (is_post, route) {
        (false, "/") => {
            let header = Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
            let _ = request.respond(Response::from_string(UI).with_header(header));
        }
        (false, "/api/state") => {
            let body = app.state_json();
            respond_json(request, 200, body);
        }
        (false, "/api/status") => {
            let body = app.status_json();
            respond_json(request, 200, body);
        }
        (false, _) if route.starts_with("/thumb/") => {
            let id = route["/thumb/".len()..].parse::<usize>().ok();
            let image = id
                .filter(|&id| app.has_thumb.get(id) == Some(&true))
                .and_then(|id| fs::read(app.thumb_dir.join(format!("{id}.jpeg"))).ok());
            match image {
                Some(bytes) => {
                    let kind = Header::from_bytes("Content-Type", "image/jpeg").unwrap();
                    let cache = Header::from_bytes("Cache-Control", "max-age=3600").unwrap();
                    let response = Response::from_data(bytes).with_header(kind).with_header(cache);
                    let _ = request.respond(response);
                }
                None => respond_error(request, 404, "thumbnail unavailable"),
            }
        }
        (false, "/preview/slide.html") => {
            let page = url
                .split_once("page=")
                .and_then(|(_, rest)| rest.split('&').next()?.parse().ok())
                .unwrap_or(1);
            match app.preview.as_ref().and_then(|p| p.page(page)) {
                Some(html) => {
                    let kind = Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
                    // o HTML do modo --watch tenta recarregar a si mesmo por websocket
                    let policy =
                        Header::from_bytes("Content-Security-Policy", "connect-src 'none'").unwrap();
                    let response = Response::from_string(html).with_header(kind).with_header(policy);
                    let _ = request.respond(response);
                }
                None => respond_error(request, 404, "preview unavailable"),
            }
        }
        (false, _) if route.starts_with("/preview/") => {
            // imagens, fontes e CSS que o deck referencia por caminho relativo
            match deck_asset(&app.path, &route["/preview/".len()..]) {
                Some((bytes, kind)) => {
                    let kind = Header::from_bytes("Content-Type", kind).unwrap();
                    let _ = request.respond(Response::from_data(bytes).with_header(kind));
                }
                None => respond_error(request, 404, "file not found"),
            }
        }
        (true, "/api/save") | (true, "/api/preview") => {
            let mut body = String::new();
            if request.as_reader().take(16 << 20).read_to_string(&mut body).is_err() {
                return respond_error(request, 400, "invalid body");
            }
            let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let entries = parse_entries(&parsed["slides"], app.deck.slides.len());
            let (Some(entries), Some(generation)) = (entries, parsed["generation"].as_u64()) else {
                return respond_error(request, 400, "invalid request");
            };
            if generation != app.generation {
                return respond_error(request, 409, "the file was reloaded; refresh the page");
            }
            if route == "/api/preview" {
                let index = parsed["index"].as_u64().unwrap_or(0) as usize;
                return match app.preview(&entries, index) {
                    Ok(page) => respond_json(request, 200, json!({ "page": page })),
                    Err(message) => respond_error(request, 503, &message),
                };
            }
            match app.save(entries, parsed["force"].as_bool().unwrap_or(false)) {
                Ok(()) => {
                    if let Some(tx) = render {
                        let _ = tx.send(());
                    }
                    let body = app.status_json();
                    respond_json(request, 200, body);
                }
                Err(SaveError::ExternalChange) => {
                    respond_json(request, 409, json!({ "error": "external_change" }))
                }
                Err(SaveError::Other(message)) => respond_error(request, 500, &message),
            }
        }
        (true, "/api/reload") => match app.reload() {
            Ok(()) => {
                if let Some(tx) = render {
                    let _ = tx.send(());
                }
                let body = app.state_json();
                respond_json(request, 200, body);
            }
            Err(message) => respond_error(request, 500, &message),
        },
        (true, "/api/rethumb") => {
            if let Some(tx) = render {
                app.thumb_state = "rendering";
                let _ = tx.send(());
            }
            let body = app.status_json();
            respond_json(request, 200, body);
        }
        _ => respond_error(request, 404, "not found"),
    }
}

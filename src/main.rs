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
Usage: marp-order [options] [presentation.md ...] [-- marp-cli arguments]

Opens a grid with the slides of a Marp presentation in the browser so you
can rearrange them by dragging. Saving rewrites the .md file in the new order.
Each file opens in its own tab; more can be opened from the browser.
Without files, the ones that were open last time are opened again.

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
    files: Vec<PathBuf>,
    port: u16,
    open: bool,
    thumbs: bool,
    marp_cmd: Option<String>,
    marp_extra: Vec<String>,
}

/// Um arquivo aberto (uma aba da interface).
pub struct Doc {
    pub id: u64,
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
    /// Pasta das miniaturas deste arquivo, uma por lâmina (`<id>.jpeg`).
    pub thumb_dir: PathBuf,
    pub has_thumb: Vec<bool>,
    pub thumb_version: u64,
    pub thumb_state: &'static str,
    pub thumb_msg: String,
}

pub struct App {
    /// Arquivos abertos, na ordem das abas.
    docs: Vec<Doc>,
    next_id: u64,
    tmp: PathBuf,
    /// Estado inicial das miniaturas de um arquivo recém-aberto.
    thumb_start: (&'static str, String),
    /// Fila do gerador de miniaturas; ausente sem marp-cli ou com `--no-thumbs`.
    render: Option<Sender<u64>>,
    /// Pré-visualização do editor; ausente quando não há marp-cli.
    preview: Option<Preview>,
}

impl App {
    pub fn doc(&mut self, id: u64) -> Option<&mut Doc> {
        self.docs.iter_mut().find(|doc| doc.id == id)
    }

    /// Abre o arquivo em uma aba nova, ou devolve a aba em que ele já está.
    fn open(&mut self, file: &Path) -> Result<u64, String> {
        // Caminho real: gravar por rename não pode trocar um link simbólico por um arquivo.
        let path = fs::canonicalize(file)
            .map_err(|e| format!("could not open {}: {e}", file.display()))?;
        if let Some(doc) = self.docs.iter().find(|doc| doc.path == path) {
            return Ok(doc.id);
        }
        let deck = load(&path)?;
        let count = deck.slides.len();
        let id = self.next_id;
        self.next_id += 1;
        self.docs.push(Doc {
            id,
            last_meta: file_signature(&path),
            last_disk: deck.source.clone().into_bytes(),
            deck,
            disk_order: (0..count).collect(),
            generation: 1,
            save_seq: 0,
            last_backup: None,
            thumb_dir: self.tmp.join(format!("doc-{id}")),
            has_thumb: vec![false; count],
            thumb_version: 0,
            thumb_state: self.thumb_start.0,
            thumb_msg: self.thumb_start.1.clone(),
            path: path.clone(),
        });
        self.rethumb(id);
        remember_recent(&path);
        self.remember_session();
        Ok(id)
    }

    fn close(&mut self, id: u64) {
        if let Some(at) = self.docs.iter().position(|doc| doc.id == id) {
            let doc = self.docs.remove(at);
            let _ = fs::remove_dir_all(doc.thumb_dir);
            self.remember_session();
        }
    }

    /// Guarda quais arquivos estão abertos, para reabri-los na próxima execução sem argumentos.
    fn remember_session(&self) {
        let files: Vec<String> = self.docs.iter().map(|doc| doc.path.display().to_string()).collect();
        write_list("session.json", &files);
    }

    fn rethumb(&self, id: u64) {
        if let Some(tx) = &self.render {
            let _ = tx.send(id);
        }
    }

    /// Converte o rascunho para a pré-visualização e devolve a página da lâmina `index`.
    fn preview(&mut self, id: u64, entries: &[Entry], index: usize) -> Result<usize, String> {
        let doc = self.doc(id).ok_or("file not open")?;
        let markdown = doc.deck.compose(entries);
        let page = match index.min(entries.len()) {
            0 => 1,
            index => doc.deck.count(&entries[..index]) + 1,
        };
        let path = doc.path.clone();
        let preview = self
            .preview
            .as_mut()
            .ok_or("preview unavailable: marp-cli not found")?;
        preview.render(&path, &markdown)?;
        Ok(page)
    }

    fn docs_json(&self) -> Value {
        let docs: Vec<Value> = self.docs.iter().map(Doc::tab_json).collect();
        json!({ "docs": docs, "recent": recent_files() })
    }
}

enum SaveError {
    ExternalChange,
    Other(String),
}

impl Doc {
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

    fn status_json(&mut self) -> Value {
        json!({
            "doc": self.id,
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

    fn tab_json(&self) -> Value {
        json!({
            "doc": self.id,
            "file": file_name(&self.path),
            "path": self.path.display().to_string(),
        })
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

/// Conteúdo de uma pasta para o diálogo "Open": subpastas e arquivos Markdown.
/// Se `target` for um arquivo, devolve só o caminho dele.
fn browse(target: &Path) -> Result<Value, String> {
    let dir = fs::canonicalize(target)
        .map_err(|e| format!("could not open {}: {e}", target.display()))?;
    if dir.is_file() {
        return Ok(json!({ "file": dir.display().to_string() }));
    }
    let mut entries: Vec<(bool, String)> = fs::read_dir(&dir)
        .map_err(|e| format!("could not read {}: {e}", dir.display()))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let is_dir = path.is_dir();
            (!name.starts_with('.') && (is_dir || is_markdown(&path))).then_some((is_dir, name))
        })
        .collect();
    entries.sort_by_key(|(is_dir, name)| (!is_dir, name.to_lowercase()));
    let entries: Vec<Value> = entries
        .into_iter()
        .map(|(is_dir, name)| json!({ "name": name, "dir": is_dir }))
        .collect();
    Ok(json!({
        "dir": dir.display().to_string(),
        "parent": dir.parent().map(|p| p.display().to_string()),
        "entries": entries,
    }))
}

const RECENT_MAX: usize = 12;

/// Arquivo de configuração do usuário (`~/.config/marp-order/<name>`).
fn config_file(name: &str) -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("marp-order").join(name))
}

/// Lista de caminhos guardada em `name`; só os arquivos que ainda existem.
fn read_list(name: &str) -> Vec<String> {
    config_file(name)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|file| Path::new(file).is_file())
        .collect()
}

fn write_list(name: &str, list: &[String]) {
    let Some(path) = config_file(name) else { return };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(path, json!(list).to_string());
}

/// Arquivos abertos por último, do mais recente para o mais antigo.
fn recent_files() -> Vec<String> {
    read_list("recent.json")
}

fn remember_recent(file: &Path) {
    let file = file.display().to_string();
    let mut list = recent_files();
    list.retain(|other| *other != file);
    list.insert(0, file);
    list.truncate(RECENT_MAX);
    write_list("recent.json", &list);
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
        files: Vec::new(),
        port: 0,
        open: true,
        thumbs: true,
        marp_cmd: std::env::var("MARP_ORDER_MARP_CMD").ok(),
        marp_extra: Vec::new(),
    };
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
            _ => args.files.push(PathBuf::from(arg)),
        }
    }
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
    let tmp = std::env::temp_dir().join(format!("marp-order-{}", std::process::id()));
    // o marp-cli serve às miniaturas e à pré-visualização do editor
    let marp_cmd = thumbs::resolve_cmd(args.marp_cmd.as_deref());
    let thumb_start = match (&marp_cmd, args.thumbs) {
        (_, false) => ("off", String::new()),
        (Some(_), true) => ("rendering", String::new()),
        (None, true) => ("error", "marp-cli not found (install `marp` or `npx`)".into()),
    };
    let preview = marp_cmd
        .as_ref()
        .map(|cmd| Preview::new(cmd.clone(), args.marp_extra.clone(), tmp.join("preview")));

    let app = Arc::new(Mutex::new(App {
        docs: Vec::new(),
        next_id: 1,
        tmp,
        thumb_start,
        render: None,
        preview,
    }));
    if let Some(cmd) = marp_cmd.filter(|_| args.thumbs) {
        let config = thumbs::Config {
            cmd,
            extra: args.marp_extra,
        };
        let tx = thumbs::spawn(app.clone(), config);
        app.lock().unwrap().render = Some(tx);
    }
    {
        let mut app = app.lock().unwrap();
        if args.files.is_empty() {
            // sem argumentos: reabre o que estava aberto da última vez
            for file in read_list("session.json") {
                let _ = app.open(Path::new(&file));
            }
        }
        for file in &args.files {
            app.open(file)?;
        }
        for doc in &app.docs {
            println!("marp-order: {} — {} slides", doc.path.display(), doc.deck.slides.len());
        }
        if app.docs.is_empty() {
            println!("marp-order: no file open — choose one in the browser");
        }
    }

    let server = Server::http(("127.0.0.1", args.port))
        .map_err(|e| format!("could not start the server: {e}"))?;
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(args.port);
    let url = format!("http://127.0.0.1:{port}/");

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
        handle(request, &app, port);
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

/// Arquivo da pasta `root` do deck (ou abaixo dela) pedido pela pré-visualização.
fn deck_asset(root: &Path, url_path: &str) -> Option<(Vec<u8>, &'static str)> {
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

/// Valor de `name=` na query string.
fn query<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    url.split_once('?')?
        .1
        .split('&')
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

fn handle(mut request: Request, app: &Mutex<App>, port: u16) {
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

    if is_post {
        let mut body = String::new();
        if request.as_reader().take(16 << 20).read_to_string(&mut body).is_err() {
            return respond_error(request, 400, "invalid body");
        }
        let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        return handle_post(request, &mut app, route, &parsed);
    }

    let doc_id = query(&url, "doc").and_then(|id| id.parse::<u64>().ok());
    match route {
        "/" => {
            let header = Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
            let _ = request.respond(Response::from_string(UI).with_header(header));
        }
        "/api/docs" => respond_json(request, 200, app.docs_json()),
        "/api/state" | "/api/status" => match doc_id.and_then(|id| app.doc(id)) {
            Some(doc) if route == "/api/state" => respond_json(request, 200, doc.state_json()),
            Some(doc) => respond_json(request, 200, doc.status_json()),
            None => respond_error(request, 404, "file not open"),
        },
        _ if route.starts_with("/thumb/") => {
            // /thumb/<arquivo>/<lâmina>
            let mut parts = route["/thumb/".len()..].split('/');
            let doc = parts.next().and_then(|id| id.parse().ok()).and_then(|id| app.doc(id));
            let slide = parts.next().and_then(|id| id.parse::<usize>().ok());
            let image = match (doc, slide) {
                (Some(doc), Some(id)) if doc.has_thumb.get(id) == Some(&true) => {
                    fs::read(doc.thumb_dir.join(format!("{id}.jpeg"))).ok()
                }
                _ => None,
            };
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
        "/preview/slide.html" => {
            let page = query(&url, "page").and_then(|p| p.parse().ok()).unwrap_or(1);
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
        _ if route.starts_with("/preview/") => {
            // imagens, fontes e CSS que o deck em pré-visualização referencia por caminho relativo
            let asset = app
                .preview
                .as_ref()
                .and_then(Preview::root)
                .and_then(|root| deck_asset(root, &route["/preview/".len()..]));
            match asset {
                Some((bytes, kind)) => {
                    let kind = Header::from_bytes("Content-Type", kind).unwrap();
                    let _ = request.respond(Response::from_data(bytes).with_header(kind));
                }
                None => respond_error(request, 404, "file not found"),
            }
        }
        _ => respond_error(request, 404, "not found"),
    }
}

fn handle_post(request: Request, app: &mut App, route: &str, body: &Value) {
    match route {
        "/api/open" => {
            let opened = match body["path"].as_str() {
                Some(path) => app.open(Path::new(path)),
                None => Err("invalid request".into()),
            };
            return match opened {
                Ok(id) => respond_json(request, 200, app.doc(id).unwrap().tab_json()),
                Err(message) => respond_error(request, 400, &message),
            };
        }
        "/api/browse" => {
            let dir = match body["dir"].as_str().filter(|dir| !dir.is_empty()) {
                Some(dir) => PathBuf::from(dir),
                None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            };
            return match browse(&dir) {
                Ok(listing) => respond_json(request, 200, listing),
                Err(message) => respond_error(request, 400, &message),
            };
        }
        _ => {}
    }

    // as demais rotas agem sobre um arquivo aberto
    let Some(id) = body["doc"].as_u64().filter(|&id| app.doc(id).is_some()) else {
        return respond_error(request, 404, "file not open");
    };
    match route {
        "/api/close" => {
            app.close(id);
            respond_json(request, 200, app.docs_json());
        }
        "/api/save" | "/api/preview" => {
            let doc = app.doc(id).unwrap();
            let entries = parse_entries(&body["slides"], doc.deck.slides.len());
            let (Some(entries), Some(generation)) = (entries, body["generation"].as_u64()) else {
                return respond_error(request, 400, "invalid request");
            };
            if generation != doc.generation {
                return respond_error(request, 409, "the file was reloaded; refresh the page");
            }
            if route == "/api/preview" {
                let index = body["index"].as_u64().unwrap_or(0) as usize;
                return match app.preview(id, &entries, index) {
                    Ok(page) => respond_json(request, 200, json!({ "page": page })),
                    Err(message) => respond_error(request, 503, &message),
                };
            }
            match doc.save(entries, body["force"].as_bool().unwrap_or(false)) {
                Ok(()) => {
                    let status = doc.status_json();
                    app.rethumb(id);
                    respond_json(request, 200, status);
                }
                Err(SaveError::ExternalChange) => {
                    respond_json(request, 409, json!({ "error": "external_change" }))
                }
                Err(SaveError::Other(message)) => respond_error(request, 500, &message),
            }
        }
        "/api/reload" => {
            let doc = app.doc(id).unwrap();
            match doc.reload() {
                Ok(()) => {
                    let state = doc.state_json();
                    app.rethumb(id);
                    respond_json(request, 200, state);
                }
                Err(message) => respond_error(request, 500, &message),
            }
        }
        "/api/rethumb" => {
            let rendering = app.render.is_some();
            let doc = app.doc(id).unwrap();
            if rendering {
                doc.thumb_state = "rendering";
            }
            let status = doc.status_json();
            app.rethumb(id);
            respond_json(request, 200, status);
        }
        _ => respond_error(request, 404, "not found"),
    }
}

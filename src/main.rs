mod deck;
mod links;
mod preview;
mod thumbs;

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use deck::{Deck, Entry};
use preview::Preview;
use thumbs::Job;

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
    /// Miniaturas de lâminas ainda não salvas (`d-<chave>.jpeg`), pela chave do texto.
    pub drafts: Vec<String>,
}

pub struct App {
    /// Arquivos abertos, na ordem das abas.
    docs: Vec<Doc>,
    next_id: u64,
    tmp: PathBuf,
    /// Pasta de configuração do usuário (recentes e sessão); ausente desliga os dois.
    config: Option<PathBuf>,
    /// Estado inicial das miniaturas de um arquivo recém-aberto.
    thumb_start: (&'static str, String),
    /// Fila do gerador de miniaturas; ausente sem marp-cli ou com `--no-thumbs`.
    render: Option<Sender<Job>>,
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
            drafts: Vec::new(),
            path: path.clone(),
        });
        self.rethumb(id);
        self.remember_recent(&path);
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
        self.write_list("session.json", &files);
    }

    fn config_file(&self, name: &str) -> Option<PathBuf> {
        Some(self.config.as_ref()?.join(name))
    }

    /// Lista de caminhos guardada em `name`; só os arquivos que ainda existem.
    fn read_list(&self, name: &str) -> Vec<String> {
        self.config_file(name)
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|file| Path::new(file).is_file())
            .collect()
    }

    fn write_list(&self, name: &str, list: &[String]) {
        let Some(path) = self.config_file(name) else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(path, json!(list).to_string());
    }

    /// Arquivos abertos por último, do mais recente para o mais antigo.
    fn recent_files(&self) -> Vec<String> {
        self.read_list("recent.json")
    }

    fn remember_recent(&self, file: &Path) {
        let file = file.display().to_string();
        let mut list = self.recent_files();
        list.retain(|other| *other != file);
        list.insert(0, file);
        list.truncate(RECENT_MAX);
        self.write_list("recent.json", &list);
    }

    /// Pede as miniaturas das lâminas editadas ou acrescentadas que ainda não existem e
    /// devolve, para cada entrada com texto novo, a chave da miniatura dela.
    fn draft_thumbs(&mut self, id: u64, entries: &[Entry]) -> Vec<Option<String>> {
        let rendering = self.render.is_some();
        let Some(doc) = self.doc(id) else { return Vec::new() };
        let mut wanted = Vec::new();
        let keys: Vec<Option<String>> = entries
            .iter()
            .enumerate()
            .map(|(at, (_, edit))| {
                let key = text_key(edit.as_ref()?);
                if !doc.drafts.contains(&key) && !wanted.iter().any(|(_, k)| *k == key) {
                    wanted.push((doc.deck.count(&entries[..at]), key.clone()));
                }
                Some(key)
            })
            .collect();
        if rendering && !wanted.is_empty() {
            let job = Job::Draft {
                doc: id,
                generation: doc.generation,
                markdown: doc.deck.compose(entries),
                total: doc.deck.count(entries),
                wanted,
            };
            if let Some(tx) = &self.render {
                let _ = tx.send(job);
            }
        }
        keys
    }

    fn rethumb(&self, id: u64) {
        if let Some(tx) = &self.render {
            let _ = tx.send(Job::Disk(id));
        }
    }

    /// Converte o rascunho para a pré-visualização e devolve a página da lâmina `index`.
    /// `head`, se vier, substitui o front-matter (edição do front-matter).
    fn preview(
        &mut self,
        id: u64,
        entries: &[Entry],
        index: usize,
        head: Option<&str>,
    ) -> Result<usize, String> {
        let doc = self.doc(id).ok_or("file not open")?;
        let mut markdown = doc.deck.compose(entries);
        if let Some(head) = head {
            markdown.replace_range(..doc.deck.head.len(), head);
        }
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
        json!({ "docs": docs, "recent": self.recent_files() })
    }
}

enum SaveError {
    ExternalChange,
    /// Outra janela salvou o arquivo depois do estado que esta conhecia.
    Stale,
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

    /// Guarda uma cópia do que está no disco antes da primeira gravação desta sessão
    /// (ou de novo, se alguém mexeu no arquivo por fora).
    fn backup_before_write(&mut self, changed: bool) -> Result<(), SaveError> {
        if self.last_backup.is_none() || changed {
            let backup = backup(&self.path)
                .map_err(|e| SaveError::Other(format!("could not create the backup: {e}")))?;
            self.last_backup = Some(backup);
        }
        Ok(())
    }

    /// `seen` é o `save_seq` que o navegador conhecia; se outro salvamento aconteceu
    /// depois, a gravação só segue com `force`.
    fn save(&mut self, entries: Vec<Entry>, force: bool, seen: Option<u64>) -> Result<(), SaveError> {
        if !force && seen.is_some_and(|seen| seen != self.save_seq) {
            return Err(SaveError::Stale);
        }
        let changed = self.external_change();
        if changed && !force {
            return Err(SaveError::ExternalChange);
        }
        self.backup_before_write(changed)?;
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

    /// Troca o front-matter do arquivo em disco, mantendo as lâminas como estão gravadas.
    fn save_head(&mut self, head: &str, force: bool) -> Result<(), SaveError> {
        let changed = self.external_change();
        if changed && !force {
            return Err(SaveError::ExternalChange);
        }
        let order: Vec<Entry> = self.disk_order.iter().map(|&id| (id, None)).collect();
        let mut content = self.deck.compose(&order);
        let mut head = head.trim().replace("\r\n", "\n");
        if !head.is_empty() {
            head.push('\n');
        }
        if self.deck.source.contains("\r\n") {
            head = head.replace('\n', "\r\n");
        }
        content.replace_range(..self.deck.head.len(), &head);
        let deck = Deck::parse(content);
        if deck.head != head {
            return Err(SaveError::Other(
                "the front matter must start and end with a `---` line".into(),
            ));
        }
        self.backup_before_write(changed)?;
        write_atomic(&self.path, deck.source.as_bytes())
            .map_err(|e| SaveError::Other(format!("could not write the file: {e}")))?;
        self.save_seq += 1;
        self.adopt(deck, &[]);
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
        self.drafts.clear();
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
            "save_seq": self.save_seq,
            "external_change": self.external_change(),
            "disk_order": self.disk_order,
            "backup": self.last_backup.as_ref().map(|p| p.display().to_string()),
            "thumbs": {
                "state": self.thumb_state,
                "message": self.thumb_msg,
                "version": self.thumb_version,
                "have": self.has_thumb,
                "drafts": self.drafts,
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
        state["head"] = json!(self.deck.head);
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

/// Pasta de configuração do usuário (`~/.config/marp-order`).
fn config_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("marp-order"))
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

const BACKUP_DIR: &str = ".marp-order-backups";
const BACKUP_KEEP: usize = 10;

/// `AAAAMMDD-HHMMSS` (UTC) a partir de segundos desde 1970.
fn timestamp(seconds: u64) -> String {
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // dias -> data civil (algoritmo de Howard Hinnant)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// Copia o arquivo para `.marp-order-backups/<nome>.<data>.bak`, ao lado dele, e apaga
/// as cópias mais antigas além das `BACKUP_KEEP` mais recentes.
fn backup(path: &Path) -> io::Result<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new(".")).join(BACKUP_DIR);
    fs::create_dir_all(&dir)?;
    let prefix = format!("{}.", file_name(path));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stamp = timestamp(now);
    let target = (0..)
        .map(|n| match n {
            0 => dir.join(format!("{prefix}{stamp}.bak")),
            n => dir.join(format!("{prefix}{stamp}-{n}.bak")),
        })
        .find(|candidate| !candidate.exists())
        .unwrap();
    fs::copy(path, &target)?;

    let mut old: Vec<PathBuf> = fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            let name = file_name(p);
            name.starts_with(&prefix) && name.ends_with(".bak")
        })
        .collect();
    old.sort();
    for stale in old.iter().rev().skip(BACKUP_KEEP) {
        let _ = fs::remove_file(stale);
    }
    Ok(target)
}

/// Chave da miniatura de um texto de lâmina ainda não salvo.
fn text_key(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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
        config: config_dir(),
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
            for file in app.read_list("session.json") {
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

    serve(&server, &app, port);
    Ok(())
}

fn serve(server: &Server, app: &Mutex<App>, port: u16) {
    for request in server.incoming_requests() {
        handle(request, app, port);
    }
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
            let image: Option<Vec<u8>> = match (doc, slide) {
                (Some(doc), Some(id)) if doc.has_thumb.get(id) == Some(&true) => {
                    fs::read(doc.thumb_dir.join(format!("{id}.jpeg"))).ok()
                }
                _ => None,
            };
            // /thumb/<arquivo>/d-<chave>: lâmina ainda não salva
            let draft = route["/thumb/".len()..].split_once("/d-").and_then(|(doc, key)| {
                let doc = app.doc(doc.parse().ok()?)?;
                let known = doc.drafts.iter().any(|k| k == key);
                known.then(|| fs::read(doc.thumb_dir.join(format!("d-{key}.jpeg"))).ok())?
            });
            let image = image.or(draft);
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
        "/api/import" => {
            // texto de lâminas copiadas de um arquivo aberto para outro, em outra pasta
            let mut dir = |key: &str| {
                let doc = app.doc(body[key].as_u64()?)?;
                Some(doc.path.parent()?.to_path_buf())
            };
            let (Some(from), Some(to), Some(texts)) = (dir("from"), dir("to"), body["texts"].as_array())
            else {
                return respond_error(request, 400, "invalid request");
            };
            let texts: Vec<String> = texts
                .iter()
                .map(|text| links::rebase(text.as_str().unwrap_or(""), &from, &to))
                .collect();
            return respond_json(request, 200, json!({ "texts": texts }));
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
        "/api/head" => {
            let doc = app.doc(id).unwrap();
            let Some(head) = body["head"].as_str() else {
                return respond_error(request, 400, "invalid request");
            };
            match doc.save_head(head, body["force"].as_bool().unwrap_or(false)) {
                Ok(()) => {
                    let state = doc.state_json();
                    app.rethumb(id);
                    respond_json(request, 200, state);
                }
                Err(SaveError::ExternalChange) => {
                    respond_json(request, 409, json!({ "error": "external_change" }))
                }
                Err(SaveError::Stale) => respond_json(request, 409, json!({ "error": "stale" })),
                Err(SaveError::Other(message)) => respond_error(request, 400, &message),
            }
        }
        "/api/save" | "/api/preview" | "/api/draft" => {
            let doc = app.doc(id).unwrap();
            let entries = parse_entries(&body["slides"], doc.deck.slides.len());
            let (Some(entries), Some(generation)) = (entries, body["generation"].as_u64()) else {
                return respond_error(request, 400, "invalid request");
            };
            if generation != doc.generation {
                return respond_error(request, 409, "the file changed in another window; reload it (Ctrl+R)");
            }
            if route == "/api/preview" {
                let index = body["index"].as_u64().unwrap_or(0) as usize;
                return match app.preview(id, &entries, index, body["head"].as_str()) {
                    Ok(page) => respond_json(request, 200, json!({ "page": page })),
                    Err(message) => respond_error(request, 503, &message),
                };
            }
            if route == "/api/draft" {
                let keys = app.draft_thumbs(id, &entries);
                return respond_json(request, 200, json!({ "keys": keys }));
            }
            let seen = body["save_seq"].as_u64();
            match doc.save(entries, body["force"].as_bool().unwrap_or(false), seen) {
                Ok(()) => {
                    let status = doc.status_json();
                    app.rethumb(id);
                    respond_json(request, 200, status);
                }
                Err(SaveError::ExternalChange) => {
                    respond_json(request, 409, json!({ "error": "external_change" }))
                }
                Err(SaveError::Stale) => respond_json(request, 409, json!({ "error": "stale" })),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicU32, Ordering};

    const DECK: &str = "---\nmarp: true\n---\n\n# One\n\n---\n\n# Two\n\n---\n\n# Three\n";

    /// Pasta temporária própria de cada teste, apagada ao final.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new() -> Sandbox {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let name = format!("marp-order-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst));
            let dir = std::env::temp_dir().join(name);
            fs::create_dir_all(&dir).unwrap();
            Sandbox(fs::canonicalize(dir).unwrap())
        }

        fn file(&self, name: &str, content: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            path
        }

        fn app(&self) -> App {
            App {
                docs: Vec::new(),
                next_id: 1,
                tmp: self.0.join("tmp"),
                config: Some(self.0.join("config")),
                thumb_start: ("off", String::new()),
                render: None,
                preview: None,
            }
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn backups(dir: &Path) -> Vec<PathBuf> {
        let mut list: Vec<PathBuf> = fs::read_dir(dir.join(BACKUP_DIR))
            .map(|entries| entries.map(|e| e.unwrap().path()).collect())
            .unwrap_or_default();
        list.sort();
        list
    }

    #[test]
    fn saving_a_new_order_rewrites_the_file_and_keeps_one_backup() {
        let sandbox = Sandbox::new();
        let path = sandbox.file("deck.md", DECK);
        let mut app = sandbox.app();
        let id = app.open(&path).unwrap();
        let doc = app.doc(id).unwrap();

        assert!(doc.save(vec![(2, None), (0, None), (1, None)], false, Some(0)).is_ok());
        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(saved, "---\nmarp: true\n---\n\n# Three\n\n---\n\n# One\n\n---\n\n# Two\n");
        assert_eq!(doc.disk_order, [2, 0, 1]);
        assert_eq!((doc.generation, doc.save_seq), (1, 1));
        assert!(!doc.external_change());

        let copies = backups(&sandbox.0);
        assert_eq!(copies.len(), 1);
        assert_eq!(fs::read_to_string(&copies[0]).unwrap(), DECK);

        // a segunda gravação da sessão não cria outra cópia
        assert!(doc.save(vec![(0, None), (1, None), (2, None)], false, Some(1)).is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), DECK);
        assert_eq!(backups(&sandbox.0).len(), 1);
    }

    #[test]
    fn saving_is_refused_when_another_window_or_program_changed_the_file() {
        let sandbox = Sandbox::new();
        let path = sandbox.file("deck.md", DECK);
        let mut app = sandbox.app();
        let id = app.open(&path).unwrap();
        let doc = app.doc(id).unwrap();
        let order = || vec![(1, None), (0, None), (2, None)];

        assert!(doc.save(order(), false, Some(0)).is_ok());
        // outra janela ainda conhece o estado anterior
        assert!(matches!(doc.save(order(), false, Some(0)), Err(SaveError::Stale)));
        assert!(doc.save(order(), true, Some(0)).is_ok());

        fs::write(&path, format!("{DECK}\n---\n\n# Four\n")).unwrap();
        assert!(doc.external_change());
        assert!(matches!(doc.save(order(), false, Some(2)), Err(SaveError::ExternalChange)));
        assert!(doc.save(order(), true, Some(2)).is_ok());
        // o que estava no disco foi guardado antes de ser sobrescrito
        let copies = backups(&sandbox.0);
        assert_eq!(copies.len(), 2);
        assert!(copies.iter().any(|copy| fs::read_to_string(copy).unwrap().contains("# Four")));
    }

    #[test]
    fn saving_an_edit_makes_the_written_file_the_new_base() {
        let sandbox = Sandbox::new();
        let path = sandbox.file("deck.md", DECK);
        let mut app = sandbox.app();
        let id = app.open(&path).unwrap();
        let doc = app.doc(id).unwrap();

        let entries = vec![(0, None), (1, Some("# Two\n\n---\n\n# Two and a half".to_string())), (2, None)];
        assert!(doc.save(entries, false, Some(0)).is_ok());
        assert_eq!(doc.generation, 2);
        assert_eq!(doc.disk_order, [0, 1, 2, 3]);
        let state = doc.state_json();
        let titles: Vec<&str> = state["slides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|slide| slide["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, ["One", "Two", "Two and a half", "Three"]);
    }

    #[test]
    fn front_matter_can_be_replaced_but_must_stay_front_matter() {
        let sandbox = Sandbox::new();
        let path = sandbox.file("deck.md", DECK);
        let mut app = sandbox.app();
        let id = app.open(&path).unwrap();
        let doc = app.doc(id).unwrap();

        assert!(doc.save_head("---\nmarp: true\ntheme: gaia\n---", false).is_ok());
        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(saved, DECK.replace("marp: true\n", "marp: true\ntheme: gaia\n"));
        assert_eq!(doc.generation, 2);

        assert!(matches!(doc.save_head("theme: gaia", false), Err(SaveError::Other(_))));
        assert_eq!(fs::read_to_string(&path).unwrap(), saved);

        // sem front-matter, as lâminas continuam as mesmas
        assert!(doc.save_head("", false).is_ok());
        assert_eq!(doc.deck.slides.len(), 3);
        assert!(fs::read_to_string(&path).unwrap().starts_with("\n# One"));
    }

    #[test]
    fn session_and_recent_files_survive_a_restart() {
        let sandbox = Sandbox::new();
        let (a, b) = (sandbox.file("a.md", DECK), sandbox.file("b.md", DECK));
        let mut app = sandbox.app();
        let first = app.open(&a).unwrap();
        let second = app.open(&b).unwrap();
        assert_eq!(app.open(&a).unwrap(), first, "an open file is not opened twice");
        app.close(second);

        let mut again = sandbox.app();
        assert_eq!(again.read_list("session.json"), [a.display().to_string()]);
        assert_eq!(again.recent_files(), [b.display().to_string(), a.display().to_string()]);
        fs::remove_file(&b).unwrap();
        assert_eq!(again.recent_files(), [a.display().to_string()]);
        assert!(again.open(&b).is_err());
    }

    #[test]
    fn browsing_lists_folders_and_markdown_files_only() {
        let sandbox = Sandbox::new();
        sandbox.file("talk.md", DECK);
        sandbox.file("Notes.MARKDOWN", "");
        sandbox.file("image.png", "");
        sandbox.file(".hidden.md", "");
        sandbox.file("sub/inner.md", "");

        let listing = browse(&sandbox.0).unwrap();
        let names: Vec<String> = listing["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| format!("{}{}", e["name"].as_str().unwrap(), if e["dir"] == true { "/" } else { "" }))
            .collect();
        assert_eq!(names, ["sub/", "Notes.MARKDOWN", "talk.md"]);
        let file = browse(&sandbox.0.join("talk.md")).unwrap();
        assert_eq!(file["file"], sandbox.0.join("talk.md").display().to_string());
        assert!(browse(&sandbox.0.join("missing")).is_err());
    }

    #[test]
    fn old_backups_are_pruned() {
        let sandbox = Sandbox::new();
        let path = sandbox.file("deck.md", DECK);
        let other = sandbox.file("other.md", DECK);
        backup(&other).unwrap();
        for _ in 0..BACKUP_KEEP + 3 {
            backup(&path).unwrap();
        }
        let copies = backups(&sandbox.0);
        let of = |name: &str| copies.iter().filter(|p| file_name(p).starts_with(name)).count();
        assert_eq!((of("deck.md."), of("other.md.")), (BACKUP_KEEP, 1));
    }

    #[test]
    fn timestamps_are_calendar_dates() {
        assert_eq!(timestamp(0), "19700101-000000");
        assert_eq!(timestamp(951_782_400), "20000229-000000");
        assert_eq!(timestamp(1_760_123_456), "20251010-191056");
    }

    #[test]
    fn preview_assets_stay_inside_the_deck_folder() {
        let sandbox = Sandbox::new();
        sandbox.file("deck/img/a b.png", "png");
        sandbox.file("deck/notes.txt", "");
        sandbox.file("secret.png", "");
        let root = sandbox.0.join("deck");

        assert_eq!(deck_asset(&root, "img/a%20b.png"), Some((b"png".to_vec(), "image/png")));
        assert_eq!(deck_asset(&root, "../secret.png"), None);
        assert_eq!(deck_asset(&root, "notes.txt"), None);
        assert_eq!(deck_asset(&root, "missing.png"), None);
        assert_eq!(query("/x?doc=3&page=12", "page"), Some("12"));
        assert_eq!(query("/x?pages=1", "page"), None);
    }

    /// Faz um pedido HTTP cru e devolve (status, corpo).
    fn http(port: u16, method: &str, path: &str, headers: &[&str], body: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut request = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
        for header in headers {
            request += &format!("{header}\r\n");
        }
        stream.write_all(format!("{request}\r\n{body}").as_bytes()).unwrap();
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status).unwrap();
        let mut rest = String::new();
        reader.read_to_string(&mut rest).unwrap();
        let body = rest.split_once("\r\n\r\n").map(|(_, body)| body.to_string()).unwrap_or_default();
        (status.split(' ').nth(1).unwrap().parse().unwrap(), body)
    }

    #[test]
    fn http_api_opens_saves_and_closes_a_file() {
        let sandbox = Sandbox::new();
        let path = sandbox.file("deck.md", DECK);
        let app = Arc::new(Mutex::new(sandbox.app()));
        let server = Arc::new(Server::http(("127.0.0.1", 0)).unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        {
            let (server, app) = (server.clone(), app.clone());
            std::thread::spawn(move || serve(&server, &app, port));
        }
        let host = format!("Host: 127.0.0.1:{port}");
        let get = |route: &str| http(port, "GET", route, &[&host], "");
        let post = |route: &str, body: Value| {
            let (status, text) = http(port, "POST", route, &[&host, "X-Marp-Order: 1"], &body.to_string());
            (status, serde_json::from_str::<Value>(&text).unwrap())
        };

        // só o navegador local, e só páginas do próprio marp-order
        assert_eq!(http(port, "GET", "/api/docs", &["Host: evil.example"], "").0, 403);
        assert_eq!(http(port, "POST", "/api/open", &[&host], "{}").0, 403);
        assert_eq!(http(port, "PUT", "/", &[&host], "").0, 405);
        assert_eq!(get("/").0, 200);
        assert_eq!(get("/api/state?doc=1").0, 404);

        let (status, tab) = post("/api/open", json!({ "path": path }));
        assert_eq!((status, &tab["file"]), (200, &json!("deck.md")));
        let doc = tab["doc"].as_u64().unwrap();
        let state: Value = serde_json::from_str(&get(&format!("/api/state?doc={doc}")).1).unwrap();
        assert_eq!(state["slides"].as_array().unwrap().len(), 3);
        assert_eq!(state["head"], "---\nmarp: true\n---\n");

        let slides = json!([{ "id": 1 }, { "id": 0 }, { "id": 2, "text": "# Third" }]);
        let save = json!({ "doc": doc, "generation": 1, "save_seq": 0, "slides": slides });
        let (status, saved) = post("/api/save", save.clone());
        assert_eq!((status, &saved["generation"], &saved["save_seq"]), (200, &json!(2), &json!(1)));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "---\nmarp: true\n---\n\n# Two\n\n---\n\n# One\n\n---\n\n# Third\n"
        );
        // o mesmo pedido de novo fala de lâminas que não existem mais
        assert_eq!(post("/api/save", save).0, 409);
        let bad = json!({ "doc": doc, "generation": 2, "slides": [{ "id": 9 }] });
        assert_eq!(post("/api/save", bad).0, 400);

        let listing = post("/api/browse", json!({ "dir": sandbox.0 })).1;
        let names: Vec<&str> = listing["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["config", "deck.md"]);
        assert_eq!(get("/preview/deck.md").0, 404);
        assert_eq!(get(&format!("/thumb/{doc}/0")).0, 404);

        assert_eq!(post("/api/close", json!({ "doc": doc })).1["docs"], json!([]));
        assert_eq!(post("/api/save", json!({ "doc": doc })).0, 404);
        server.unblock();
    }
}

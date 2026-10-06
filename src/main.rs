mod deck;
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

use deck::Deck;

const UI: &str = include_str!("ui.html");

const USAGE: &str = "\
Uso: marp-order [opções] <apresentacao.md> [-- argumentos do marp-cli]

Abre no navegador uma matriz com as lâminas da apresentação Marp para
reorganizar arrastando. Ao salvar, o arquivo .md é reescrito na nova ordem.

Opções:
  --port <n>         porta do servidor local (padrão: escolhida pelo sistema)
  --no-open          não abre o navegador automaticamente
  --no-thumbs        não gera miniaturas (mostra só o texto das lâminas)
  --marp-cmd <cmd>   comando do marp-cli (padrão: `marp` ou `npx @marp-team/marp-cli`)
  -h, --help         mostra esta ajuda
  -V, --version      mostra a versão

Exemplo com tema próprio:
  marp-order aula.md -- --theme-set tema.css";

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

    fn save(&mut self, order: Vec<usize>, force: bool) -> Result<(), SaveError> {
        if order.iter().any(|&id| id >= self.deck.slides.len()) {
            return Err(SaveError::Other("lâmina desconhecida".into()));
        }
        let changed = self.external_change();
        if changed && !force {
            return Err(SaveError::ExternalChange);
        }
        // O backup guarda o que está prestes a ser sobrescrito pela primeira vez.
        if self.last_backup.is_none() || changed {
            let backup = free_backup_path(&self.path);
            fs::copy(&self.path, &backup)
                .map_err(|e| SaveError::Other(format!("falha ao criar o backup: {e}")))?;
            self.last_backup = Some(backup);
        }
        let content = self.deck.compose(&order).into_bytes();
        write_atomic(&self.path, &content)
            .map_err(|e| SaveError::Other(format!("falha ao gravar: {e}")))?;
        self.last_disk = content;
        self.last_meta = file_signature(&self.path);
        self.disk_order = order;
        self.save_seq += 1;
        Ok(())
    }

    fn reload(&mut self) -> Result<(), String> {
        let (bytes, deck) = load(&self.path)?;
        self.has_thumb = vec![false; deck.slides.len()];
        self.disk_order = (0..deck.slides.len()).collect();
        self.deck = deck;
        self.last_disk = bytes;
        self.last_meta = file_signature(&self.path);
        self.generation += 1;
        self.thumb_version += 1;
        Ok(())
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
                        "Diretiva que vale desta lâmina em diante: {}. Mover esta lâmina muda quais lâminas são afetadas.",
                        directives.join(", ")
                    ));
                }
                if slide.lead_comment {
                    warn.push(
                        "Com headingDivider, o comentário antes do título passa para a lâmina anterior se esta deixar de ser a primeira."
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

fn load(path: &Path) -> Result<(Vec<u8>, Deck), String> {
    let bytes = fs::read(path).map_err(|e| format!("não foi possível ler {}: {e}", path.display()))?;
    let text = String::from_utf8(bytes.clone())
        .map_err(|_| format!("{} não é um arquivo UTF-8 válido", path.display()))?;
    Ok((bytes, Deck::parse(text)))
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
                    .ok_or("--port exige um número de porta")?;
            }
            "--marp-cmd" => {
                args.marp_cmd = Some(it.next().ok_or("--marp-cmd exige um comando")?);
            }
            "--" => {
                args.marp_extra = it.by_ref().collect();
            }
            other if other.starts_with('-') => return Err(format!("opção desconhecida: {other}")),
            _ if file.is_some() => return Err("informe apenas um arquivo".into()),
            _ => file = Some(PathBuf::from(arg)),
        }
    }
    args.file = file.ok_or("informe o arquivo .md da apresentação")?;
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
        .map_err(|e| format!("não foi possível abrir {}: {e}", args.file.display()))?;
    let (bytes, deck) = load(&path)?;
    let count = deck.slides.len();

    let thumb_dir = std::env::temp_dir().join(format!("marp-order-{}", std::process::id()));
    let marp_cmd = if args.thumbs {
        thumbs::resolve_cmd(args.marp_cmd.as_deref())
    } else {
        None
    };
    let (thumb_state, thumb_msg) = match (&marp_cmd, args.thumbs) {
        (Some(_), _) => ("rendering", String::new()),
        (None, true) => ("error", "marp-cli não encontrado (instale `marp` ou `npx`)".into()),
        (None, false) => ("off", String::new()),
    };

    let app = Arc::new(Mutex::new(App {
        last_meta: file_signature(&path),
        path: path.clone(),
        deck,
        last_disk: bytes,
        disk_order: (0..count).collect(),
        generation: 1,
        save_seq: 0,
        last_backup: None,
        thumb_dir: thumb_dir.clone(),
        has_thumb: vec![false; count],
        thumb_version: 0,
        thumb_state,
        thumb_msg,
    }));

    let render = marp_cmd.map(|cmd| {
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
        .map_err(|e| format!("não foi possível iniciar o servidor: {e}"))?;
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(args.port);
    let url = format!("http://127.0.0.1:{port}/");

    println!("marp-order: {} — {count} lâminas", path.display());
    println!("Abra {url}  (Ctrl+C para encerrar)");
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

fn handle(mut request: Request, app: &Mutex<App>, render: Option<&Sender<()>>, port: u16) {
    // Só o próprio navegador local fala com o servidor: barra DNS rebinding e POSTs de outros sites.
    let host = header_value(&request, "Host").unwrap_or_default();
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return respond_error(request, 403, "host inválido");
    }
    let is_post = *request.method() == Method::Post;
    if is_post && header_value(&request, "X-Marp-Order").is_none() {
        return respond_error(request, 403, "requisição não autorizada");
    }
    if !is_post && *request.method() != Method::Get {
        return respond_error(request, 405, "método não suportado");
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
                None => respond_error(request, 404, "miniatura indisponível"),
            }
        }
        (true, "/api/save") => {
            let mut body = String::new();
            if request.as_reader().take(4 << 20).read_to_string(&mut body).is_err() {
                return respond_error(request, 400, "corpo inválido");
            }
            let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let order: Option<Vec<usize>> = parsed["order"].as_array().and_then(|items| {
                items
                    .iter()
                    .map(|v| v.as_u64().map(|n| n as usize))
                    .collect()
            });
            let (Some(order), Some(generation)) = (order, parsed["generation"].as_u64()) else {
                return respond_error(request, 400, "pedido inválido");
            };
            if generation != app.generation {
                return respond_error(request, 409, "o arquivo foi recarregado; atualize a página");
            }
            match app.save(order, parsed["force"].as_bool().unwrap_or(false)) {
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
        _ => respond_error(request, 404, "não encontrado"),
    }
}

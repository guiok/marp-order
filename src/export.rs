//! Exporting a tab, as it is, to PDF, PowerPoint, HTML, images or a text file with the
//! speaker notes. marp-cli does the conversion in the background; the interface polls
//! `Status` until it is done.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};

use crate::links;
use crate::thumbs::{self, Config};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    Pdf,
    Pptx,
    Html,
    /// One image per slide.
    Png,
    /// Only the speaker notes, as plain text.
    Notes,
}

impl Format {
    /// The format named by its file extension, which is also how the interface names it.
    pub fn parse(extension: &str) -> Option<Format> {
        Some(match extension {
            "pdf" => Format::Pdf,
            "pptx" => Format::Pptx,
            "html" => Format::Html,
            "png" => Format::Png,
            "txt" => Format::Notes,
            _ => return None,
        })
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Pdf => "pdf",
            Format::Pptx => "pptx",
            Format::Html => "html",
            Format::Png => "png",
            Format::Notes => "txt",
        }
    }

    /// marp-cli options; HTML is what it writes when given none.
    fn args(self, notes: bool) -> &'static [&'static str] {
        match self {
            Format::Pdf if notes => &["--pdf", "--pdf-notes"],
            Format::Pdf => &["--pdf"],
            Format::Pptx => &["--pptx"],
            Format::Html => &[],
            Format::Png => &["--images", "png"],
            Format::Notes => &["--notes"],
        }
    }

    /// The first file written: marp-cli numbers images after the name (`deck.001.png`).
    pub fn first_file(self, target: &Path) -> PathBuf {
        match self {
            Format::Png => target.with_extension("001.png"),
            _ => target.to_path_buf(),
        }
    }
}

pub struct Job {
    /// The tab's file; the draft goes next to it so relative paths resolve.
    pub deck: PathBuf,
    pub markdown: String,
    pub format: Format,
    /// PDF only: the speaker notes as annotations.
    pub notes: bool,
    pub target: PathBuf,
    pub slides: usize,
}

/// The export running or the last one finished.
#[derive(Default)]
pub struct Status {
    running: bool,
    format: Option<Format>,
    /// What was written: the file, or the folder of the images.
    path: Option<PathBuf>,
    slides: usize,
    error: Option<String>,
}

impl Status {
    pub fn running(&self) -> bool {
        self.running
    }

    /// The result of the last export, if it worked.
    pub fn done(&self) -> Option<&Path> {
        self.path.as_deref().filter(|_| !self.running)
    }

    pub fn json(&self) -> Value {
        let state = match (self.running, &self.error, &self.path) {
            (true, _, _) => "running",
            (_, Some(_), _) => "error",
            (_, _, Some(_)) => "done",
            _ => "idle",
        };
        json!({
            "state": state,
            "format": self.format.map(Format::extension),
            "path": self.path.as_ref().map(|path| path.display().to_string()),
            "slides": self.slides,
            "message": self.error,
        })
    }
}

/// Starts the export in its own thread; `status` follows it.
pub fn start(status: &Arc<Mutex<Status>>, config: Config, job: Job) {
    *status.lock().unwrap() = Status {
        running: true,
        format: Some(job.format),
        slides: job.slides,
        ..Status::default()
    };
    let status = status.clone();
    thread::spawn(move || {
        let result = run(&config, &job);
        let mut status = status.lock().unwrap();
        status.running = false;
        match result {
            Ok(()) => {
                let folder = job.target.parent().map(Path::to_path_buf);
                status.path = match job.format {
                    Format::Png => folder,
                    _ => Some(job.target),
                };
            }
            Err(message) => status.error = Some(message),
        }
    });
}

fn run(config: &Config, job: &Job) -> Result<(), String> {
    let markdown = match job.format {
        // the HTML keeps image paths as written, so they must work from where it is saved;
        // the other formats are rendered from the deck's folder
        Format::Html => {
            let from = job.deck.parent().unwrap_or(Path::new("."));
            let to = job.target.parent().unwrap_or(Path::new("."));
            links::rebase(&job.markdown, from, to)
        }
        _ => job.markdown.clone(),
    };
    let name = job.deck.file_name().unwrap_or_default().to_string_lossy();
    let draft = job.deck.with_file_name(format!(".{name}.marp-order.export.md"));
    fs::write(&draft, markdown).map_err(|e| format!("could not write a draft next to the file: {e}"))?;
    let result = thumbs::marp(config, &draft, job.format.args(job.notes), &job.target);
    let _ = fs::remove_file(&draft);
    result
}

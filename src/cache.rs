//! Thumbnails kept on disk between runs, so a file that has not changed shows its
//! slides at once when it is opened again.
//!
//! There is one entry per deck: a folder named after the deck's path and the marp-cli
//! command, holding one image per slide in file order and a `meta.json` with the hash
//! of the deck content they were rendered from.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Decks whose thumbnails are kept; the least recently rendered ones are dropped.
const KEEP: usize = 40;

#[derive(Clone)]
pub struct Cache {
    dir: PathBuf,
    /// What besides the deck changes the result: the marp-cli command and its arguments.
    salt: String,
}

/// FNV-1a. Unlike the standard hasher, it is guaranteed to give the same value on every run.
pub fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

impl Cache {
    pub fn new(dir: PathBuf, command: &[String], extra: &[String]) -> Cache {
        let salt = format!("{}\0{}", command.join(" "), extra.join(" "));
        Cache { dir, salt }
    }

    fn entry(&self, deck: &Path) -> PathBuf {
        let key = format!("{}\0{}", deck.display(), self.salt);
        self.dir.join(format!("{:016x}", hash(key.as_bytes())))
    }

    /// Copies the cached thumbnails of `deck` into `targets` (one path per slide, in file
    /// order) if they were rendered from content with this hash. Returns whether it did.
    pub fn load(&self, deck: &Path, content: u64, targets: &[PathBuf]) -> bool {
        let entry = self.entry(deck);
        let meta = fs::read_to_string(entry.join("meta.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let Some(meta) = meta else { return false };
        if meta["hash"] != json!(format!("{content:016x}")) || meta["count"] != json!(targets.len()) {
            return false;
        }
        targets
            .iter()
            .enumerate()
            .all(|(at, target)| fs::copy(entry.join(format!("{at}.jpeg")), target).is_ok())
    }

    /// Keeps `images` (one per slide, in file order) as the thumbnails of `deck` for
    /// content with this hash, replacing what was there.
    pub fn store(&self, deck: &Path, content: u64, images: &[PathBuf]) {
        let entry = self.entry(deck);
        // without meta.json a half-written entry is never used
        let _ = fs::remove_dir_all(&entry);
        if fs::create_dir_all(&entry).is_err() {
            return;
        }
        let copied = images
            .iter()
            .enumerate()
            .all(|(at, image)| fs::copy(image, entry.join(format!("{at}.jpeg"))).is_ok());
        if !copied {
            let _ = fs::remove_dir_all(&entry);
            return;
        }
        let meta = json!({ "hash": format!("{content:016x}"), "count": images.len() });
        let _ = fs::write(entry.join("meta.json"), meta.to_string());
        self.prune();
    }

    fn prune(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else { return };
        let mut entries: Vec<_> = entries
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                let written = fs::metadata(path.join("meta.json")).and_then(|m| m.modified()).ok();
                Some((written, path))
            })
            .collect();
        // entries without meta.json (None) sort first and go first
        entries.sort();
        for (_, path) in entries.iter().rev().skip(KEEP) {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marp-order-cache-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn images(dir: &Path, prefix: &str, count: usize) -> Vec<PathBuf> {
        (0..count)
            .map(|n| {
                let path = dir.join(format!("{prefix}{n}.jpeg"));
                fs::write(&path, format!("{prefix}{n}")).unwrap();
                path
            })
            .collect()
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(hash(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(hash(b"ab"), hash(b"ba"));
    }

    #[test]
    fn thumbnails_come_back_only_for_the_same_content_and_command() {
        let dir = sandbox("roundtrip");
        let cache = Cache::new(dir.join("cache"), &["marp".into()], &[]);
        let deck = Path::new("/talks/deck.md");
        cache.store(deck, 7, &images(&dir, "in", 3));

        let out: Vec<PathBuf> = (0..3).map(|n| dir.join(format!("out{n}.jpeg"))).collect();
        assert!(cache.load(deck, 7, &out));
        assert_eq!(fs::read_to_string(&out[2]).unwrap(), "in2");

        assert!(!cache.load(deck, 8, &out), "the deck changed");
        assert!(!cache.load(deck, 7, &out[..2]), "another number of slides");
        assert!(!cache.load(Path::new("/talks/other.md"), 7, &out), "another deck");
        let themed = Cache::new(dir.join("cache"), &["marp".into()], &["--theme-set".into(), "t.css".into()]);
        assert!(!themed.load(deck, 7, &out), "other marp-cli arguments");

        // a new render replaces the entry
        cache.store(deck, 8, &images(&dir, "new", 2));
        assert!(!cache.load(deck, 7, &out));
        assert!(cache.load(deck, 8, &out[..2]));
        assert_eq!(fs::read_to_string(&out[0]).unwrap(), "new0");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn only_the_most_recent_decks_are_kept() {
        let dir = sandbox("prune");
        let cache = Cache::new(dir.join("cache"), &[], &[]);
        let one = images(&dir, "i", 1);
        for n in 0..KEEP + 5 {
            cache.store(Path::new(&format!("/talks/{n}.md")), 1, &one);
        }
        assert_eq!(fs::read_dir(dir.join("cache")).unwrap().count(), KEEP);
        let last = PathBuf::from(format!("/talks/{}.md", KEEP + 4));
        assert!(cache.load(&last, 1, &[dir.join("o.jpeg")]));
        let _ = fs::remove_dir_all(dir);
    }
}

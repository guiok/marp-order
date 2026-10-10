//! Caminhos relativos em lâminas copiadas de um deck para outro, em outra pasta.

use std::path::{Component, Path, PathBuf};

/// Caminho relativo de `to` até `from` (as duas pastas absolutas), terminado em `/`.
fn prefix(from: &Path, to: &Path) -> Option<String> {
    let from: Vec<Component> = from.components().collect();
    let to: Vec<Component> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..to.len() {
        out.push("..");
    }
    for part in &from[common..] {
        out.push(part);
    }
    let text = out.to_str()?.replace('\\', "/");
    Some(if text.is_empty() { text } else { text + "/" })
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

/// Um destino é reescrito só se for um caminho relativo para um arquivo que existe
/// na pasta de origem; URLs, âncoras e caminhos absolutos ficam como estão.
fn is_local_file(target: &str, from: &Path) -> bool {
    let path = target.split(['?', '#']).next().unwrap_or("");
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(':')
        && from.join(percent_decode(path)).is_file()
}

/// Reescreve `](alvo)`, `src="alvo"` e `url(alvo)` de `text`, escrito para um deck na
/// pasta `from`, para que continuem apontando para os mesmos arquivos a partir de `to`.
pub fn rebase(text: &str, from: &Path, to: &Path) -> String {
    let Some(prefix) = prefix(from, to).filter(|p| !p.is_empty()) else {
        return text.to_string();
    };
    const OPENERS: [&str; 4] = ["](", "src=\"", "src='", "url("];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let next = OPENERS
            .iter()
            .filter_map(|opener| rest.find(opener).map(|at| (at, *opener)))
            .min();
        let Some((at, opener)) = next else {
            out.push_str(rest);
            return out;
        };
        let start = at + opener.len();
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        // `url("x")` e `url('x')` trazem aspas dentro dos parênteses
        let quote = rest.chars().next().filter(|c| opener == "url(" && matches!(c, '"' | '\''));
        if let Some(quote) = quote {
            out.push(quote);
            rest = &rest[1..];
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '>'))
            .unwrap_or(rest.len());
        if is_local_file(&rest[..end], from) {
            out.push_str(&prefix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rewrites_only_existing_relative_files() {
        let root = std::env::temp_dir().join(format!("marp-order-links-{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("b/deep"));
        fs::create_dir_all(a.join("img")).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("img/x y.png"), "").unwrap();
        fs::write(a.join("bg.jpg"), "").unwrap();

        let text = "![bg](bg.jpg)\n![w:200](img/x%20y.png \"t\") [site](https://e.com/bg.jpg)\n\
                    <img src=\"bg.jpg\"> <img src='missing.png'>\n\
                    <style>section { background: url('bg.jpg') url(bg.jpg#f) url(/abs.png) }</style>\n\
                    [anchor](#bg.jpg)";
        let want = "![bg](../../a/bg.jpg)\n![w:200](../../a/img/x%20y.png \"t\") [site](https://e.com/bg.jpg)\n\
                    <img src=\"../../a/bg.jpg\"> <img src='missing.png'>\n\
                    <style>section { background: url('../../a/bg.jpg') url(../../a/bg.jpg#f) url(/abs.png) }</style>\n\
                    [anchor](#bg.jpg)";
        assert_eq!(rebase(text, &a, &b), want);
        assert_eq!(rebase(text, &a, &a), text);
        assert_eq!(rebase("![x](bg.jpg)", &a, &root), "![x](a/bg.jpg)");
        let _ = fs::remove_dir_all(root);
    }
}

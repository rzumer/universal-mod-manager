//! A list block at the top level of a Paradox settings file: Crusader Kings II's
//! `last_mods={ "mod/a.mod" "mod/b.mod" }` in `settings.txt`. Only the block changes; the
//! rest of the file keeps its bytes.

/// The file's text, byte for byte (these files are not always UTF-8).
pub fn decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Back to the file's bytes ([`decode`]'s inverse; entries UMM adds are ASCII).
pub fn encode(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect()
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-' | b':' | b'@')
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Past a comment or quoted string starting at `i`, or `None` if there is none there.
fn skip_special(b: &[u8], i: usize) -> Option<usize> {
    match b[i] {
        b'#' => Some(
            b[i..]
                .iter()
                .position(|&c| c == b'\n')
                .map_or(b.len(), |p| i + p),
        ),
        b'"' => Some(
            b[i + 1..]
                .iter()
                .position(|&c| c == b'"')
                .map_or(b.len(), |p| i + p + 2),
        ),
        _ => None,
    }
}

/// The `}` closing the block whose content starts at `open`.
fn closing(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut i = open;
    while i < b.len() {
        if let Some(next) = skip_special(b, i) {
            i = next;
            continue;
        }
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Where the top-level block `key={…}` is: from its key to past its `}`, and its content.
fn find(text: &str, key: &str) -> Option<(usize, usize, usize, usize)> {
    let b = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len() {
        if let Some(next) = skip_special(b, i) {
            i = next;
            continue;
        }
        match b[i] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            c if is_word(c) => {
                let start = i;
                while i < b.len() && is_word(b[i]) {
                    i += 1;
                }
                if depth == 0 && text[start..i].eq_ignore_ascii_case(key) {
                    let j = skip_ws(b, i);
                    if b.get(j) == Some(&b'=') {
                        let j = skip_ws(b, j + 1);
                        if b.get(j) == Some(&b'{') {
                            let close = closing(b, j + 1)?;
                            return Some((start, close + 1, j + 1, close));
                        }
                    }
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The block's entries (quoted or bare), or `None` when the file has no such block.
pub fn get(text: &str, key: &str) -> Option<Vec<String>> {
    let (_, _, open, close) = find(text, key)?;
    let inner = &text[open..close];
    let b = inner.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            c if c.is_ascii_whitespace() => i += 1,
            b'#' => i = skip_special(b, i).unwrap_or(b.len()),
            b'"' => {
                let end = skip_special(b, i).unwrap_or(b.len());
                out.push(inner[i + 1..end.saturating_sub(1).max(i + 1)].to_string());
                i = end;
            }
            _ => {
                let start = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'"' | b'#') {
                    i += 1;
                }
                out.push(inner[start..i].to_string());
            }
        }
    }
    Some(out)
}

/// The text with the block holding these entries (added at the end when missing), or without
/// it for `None`.
pub fn set(text: &str, key: &str, entries: Option<&[String]>) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let block = entries.map(|es| {
        let mut s = format!("{key}={{{nl}");
        for e in es {
            s.push_str(&format!("\t\"{e}\"{nl}"));
        }
        s.push('}');
        s
    });
    match (find(text, key), block) {
        (Some((start, end, _, _)), Some(block)) => {
            format!("{}{block}{}", &text[..start], &text[end..])
        }
        (Some((start, end, _, _)), None) => {
            let rest = &text[end..];
            let rest = rest
                .strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix('\n'))
                .unwrap_or(rest);
            format!("{}{rest}", &text[..start])
        }
        (None, Some(block)) => {
            let sep = if text.is_empty() || text.ends_with('\n') {
                ""
            } else {
                nl
            };
            format!("{text}{sep}{block}{nl}")
        }
        (None, None) => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_only_the_block() {
        let text = "# settings\r\nsize={\r\n\tx=1920\r\n\tlast_mods=3\r\n}\r\nlast_mods={\r\n\t\"mod/a.mod\"\r\n\t\"mod/b c.mod\" }\r\nlanguage=\"l_english\"\r\n";
        assert_eq!(
            get(text, "last_mods").unwrap(),
            ["mod/a.mod", "mod/b c.mod"],
            "the nested key is not the block"
        );
        let mods = ["mod/a.mod".to_string(), "mod/umm-x.mod".to_string()];
        let new = set(text, "last_mods", Some(&mods));
        assert!(new.starts_with("# settings\r\nsize={\r\n\tx=1920\r\n\tlast_mods=3\r\n}\r\n"));
        assert!(new.ends_with("last_mods={\r\n\t\"mod/a.mod\"\r\n\t\"mod/umm-x.mod\"\r\n}\r\nlanguage=\"l_english\"\r\n"));
        assert_eq!(get(&new, "last_mods").unwrap(), mods);
        let gone = set(&new, "last_mods", None);
        assert_eq!(get(&gone, "last_mods"), None);
        assert!(gone.ends_with("}\r\nlanguage=\"l_english\"\r\n"));

        assert_eq!(get("language=x\n", "last_mods"), None);
        let added = set("language=x", "last_mods", Some(&mods[1..]));
        assert_eq!(added, "language=x\nlast_mods={\n\t\"mod/umm-x.mod\"\n}\n");
        assert_eq!(
            get("last_mods={}", "last_mods").unwrap(),
            Vec::<String>::new()
        );
        let bytes = [b'n', b'=', 0xe9, b'\n'];
        assert_eq!(encode(&decode(&bytes)), bytes);
    }
}

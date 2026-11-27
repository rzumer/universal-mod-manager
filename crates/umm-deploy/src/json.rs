//! An array of objects under a key of a JSON file's top-level object (Terraria's
//! `ResourcePacks` in `config.json`). Only that array changes; the rest of the file keeps its
//! bytes. Entries are the array's elements, as compact JSON; two are the same when their
//! `FileName` is (in any case), or else their text.

use serde_json::Value;
use std::ops::Range;

/// What makes an entry the one it is: its `FileName`, lower-case.
pub fn id(entry: &str) -> String {
    match serde_json::from_str::<Value>(entry) {
        Ok(Value::Object(o)) => o
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("FileName"))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_lowercase)
            .unwrap_or_else(|| entry.trim().to_string()),
        _ => entry.trim().to_string(),
    }
}

/// The end of the JSON string starting at `i` (its opening quote), past its closing quote.
fn string_end(b: &[u8], mut i: usize) -> Option<usize> {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// The end of the value starting at `i`: past its closing bracket for an array or object.
fn value_end(b: &[u8], i: usize) -> Option<usize> {
    match b.get(i)? {
        b'"' => string_end(b, i),
        b'[' | b'{' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = string_end(b, j)?;
                        continue;
                    }
                    b'[' | b'{' => depth += 1,
                    b']' | b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let rest = &b[i..];
            Some(
                i + rest
                    .iter()
                    .position(|c| matches!(c, b',' | b'}' | b']') || c.is_ascii_whitespace())
                    .unwrap_or(rest.len()),
            )
        }
    }
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// A JSON object's members: each key's span (with its quotes), and its value's.
type Members = Vec<(Range<usize>, Range<usize>)>;

/// The top-level object's members: each key's span (with its quotes), and its value's.
fn members(text: &str) -> Option<Members> {
    let b = text.as_bytes();
    let mut i = skip_ws(b, 0);
    // A byte order mark, as .NET may write.
    if text[i..].starts_with('\u{feff}') {
        i = skip_ws(b, i + '\u{feff}'.len_utf8());
    }
    if b.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    let mut out = Vec::new();
    loop {
        i = skip_ws(b, i);
        match b.get(i)? {
            b'}' => return Some(out),
            b',' => {
                i += 1;
                continue;
            }
            b'"' => {}
            _ => return None,
        }
        let key = i..string_end(b, i)?;
        i = skip_ws(b, key.end);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i = skip_ws(b, i + 1);
        let value = i..value_end(b, i)?;
        i = value.end;
        out.push((key, value));
    }
}

/// The span of `key`'s value, and the members.
fn find(text: &str, key: &str) -> Option<(Option<Range<usize>>, Members)> {
    let m = members(text)?;
    let found = m
        .iter()
        .find(|(k, _)| serde_json::from_str::<String>(&text[k.clone()]).is_ok_and(|k| k == key))
        .map(|(_, v)| v.clone());
    Some((found, m))
}

/// The array's elements, or `None` when the file has no such array.
pub fn get(text: &str, key: &str) -> Option<Vec<String>> {
    let (Some(v), _) = find(text, key)? else {
        return None;
    };
    match serde_json::from_str::<Value>(&text[v]).ok()? {
        Value::Array(items) => Some(items.iter().map(Value::to_string).collect()),
        _ => None,
    }
}

/// The line ending the file uses; CRLF for a new one, as .NET writes on Windows.
fn eol(text: &str) -> &'static str {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => "\r\n",
        Some(_) => "\n",
        None => "\r\n",
    }
}

/// The array as Json.NET writes it, indented, at a member's depth.
fn render(entries: &[String], nl: &str) -> String {
    let items: Vec<String> = entries
        .iter()
        .filter_map(|e| serde_json::from_str::<Value>(e).ok())
        .map(|v| {
            let pretty = serde_json::to_string_pretty(&v).unwrap_or_default();
            // Its own lines one level in, under the member.
            pretty
                .lines()
                .map(|l| format!("    {l}"))
                .collect::<Vec<_>>()
                .join(nl)
        })
        .collect();
    if items.is_empty() {
        return "[]".into();
    }
    format!("[{nl}{}{nl}  ]", items.join(&format!(",{nl}")))
}

/// The text with the array holding these entries (`None`: emptied, or the file left with
/// nothing if it held only this). `None` when the file is not a JSON object.
pub fn set(text: &str, key: &str, entries: Option<&[String]>) -> Option<String> {
    let nl = eol(text);
    if text.trim().is_empty() {
        return Some(match entries {
            Some(e) if !e.is_empty() => format!(
                "{{{nl}  {}: {}{nl}}}",
                Value::String(key.into()),
                render(e, nl)
            ),
            _ => String::new(),
        });
    }
    let (found, members) = find(text, key)?;
    let array = render(entries.unwrap_or_default(), nl);
    match found {
        Some(v) => {
            if entries.is_none() && members.len() == 1 {
                // Nothing but the array, emptied: a file UMM made is removed.
                return Some(String::new());
            }
            Some(format!("{}{array}{}", &text[..v.start], &text[v.end..]))
        }
        None if entries.is_none_or(<[String]>::is_empty) => Some(text.to_string()),
        None => {
            let close = text.rfind('}')?;
            let before = text[..close].trim_end();
            let comma = if members.is_empty() { "" } else { "," };
            Some(format!(
                "{before}{comma}{nl}  {}: {array}{nl}{}",
                Value::String(key.into()),
                &text[close..]
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "{\r\n  \"SmartCursorToggle\": true,\r\n  \"ResourcePacks\": [\r\n    {\r\n      \"FileName\": \"Theirs\",\r\n      \"Enabled\": true,\r\n      \"SortingOrder\": 0\r\n    }\r\n  ],\r\n  \"Language\": \"en-US\"\r\n}";

    #[test]
    fn reads_and_changes_only_the_array() {
        let got = get(CONFIG, "ResourcePacks").unwrap();
        assert_eq!(
            got,
            [r#"{"Enabled":true,"FileName":"Theirs","SortingOrder":0}"#]
        );
        assert_eq!(id(&got[0]), "theirs");
        let ours = r#"{"FileName":"Ours","Enabled":true,"SortingOrder":1}"#.to_string();
        let mut entries = got.clone();
        entries.push(ours.clone());
        let new = set(CONFIG, "ResourcePacks", Some(&entries)).unwrap();
        assert!(new
            .starts_with("{\r\n  \"SmartCursorToggle\": true,\r\n  \"ResourcePacks\": [\r\n    {"));
        assert!(new.ends_with("  ],\r\n  \"Language\": \"en-US\"\r\n}"));
        let back = get(&new, "ResourcePacks").unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(id(&back[1]), "ours");
        let v: Value = serde_json::from_str(&new).unwrap();
        assert_eq!(v["Language"], "en-US");
        // Taken out again.
        let left = set(&new, "ResourcePacks", Some(&got)).unwrap();
        assert_eq!(get(&left, "ResourcePacks").unwrap(), got);
    }

    #[test]
    fn adds_the_array_when_missing_and_makes_a_new_file() {
        let e = vec![r#"{"FileName":"Ours","Enabled":true,"SortingOrder":0}"#.to_string()];
        let text = "{\n  \"Language\": \"en-US\"\n}\n";
        let new = set(text, "ResourcePacks", Some(&e)).unwrap();
        let v: Value = serde_json::from_str(&new).unwrap();
        assert_eq!(v["ResourcePacks"][0]["FileName"], "Ours");
        assert_eq!(v["Language"], "en-US");
        assert!(get(text, "ResourcePacks").is_none());

        let made = set("", "ResourcePacks", Some(&e)).unwrap();
        assert_eq!(get(&made, "ResourcePacks").unwrap().len(), 1);
        assert_eq!(set(&made, "ResourcePacks", None).unwrap(), "");
        assert!(set("not json", "ResourcePacks", Some(&e)).is_none());
    }
}

//! Minimal parser for Valve's KeyValues text format (`libraryfolders.vdf`, `appmanifest_*.acf`).

#[derive(Debug, Clone)]
pub enum Value {
    Str(String),
    Obj(Vec<(String, Value)>),
}

pub fn get<'a>(obj: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    obj.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

enum Tok {
    Str(String),
    Open,
    Close,
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(&c) = it.peek() {
        if c.is_whitespace() {
            it.next();
            continue;
        }
        match c {
            '{' => {
                it.next();
                out.push(Tok::Open);
            }
            '}' => {
                it.next();
                out.push(Tok::Close);
            }
            '/' => {
                it.next();
                if it.peek() != Some(&'/') {
                    return None;
                }
                for c in it.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                it.next();
                let mut st = String::new();
                loop {
                    match it.next()? {
                        '"' => break,
                        '\\' => match it.next()? {
                            'n' => st.push('\n'),
                            't' => st.push('\t'),
                            o => st.push(o),
                        },
                        o => st.push(o),
                    }
                }
                out.push(Tok::Str(st));
            }
            _ => {
                let mut st = String::new();
                while let Some(&c) = it.peek() {
                    if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                        break;
                    }
                    st.push(c);
                    it.next();
                }
                out.push(Tok::Str(st));
            }
        }
    }
    Some(out)
}

pub fn parse(s: &str) -> Option<Vec<(String, Value)>> {
    let toks = tokenize(s)?;
    let mut i = 0;
    parse_obj(&toks, &mut i, true)
}

fn parse_obj(t: &[Tok], i: &mut usize, top: bool) -> Option<Vec<(String, Value)>> {
    let mut out = Vec::new();
    loop {
        match t.get(*i) {
            None => return if top { Some(out) } else { None },
            Some(Tok::Close) => {
                if top {
                    return None;
                }
                *i += 1;
                return Some(out);
            }
            Some(Tok::Open) => return None,
            Some(Tok::Str(k)) => {
                *i += 1;
                let k = k.clone();
                match t.get(*i)? {
                    Tok::Str(v) => {
                        *i += 1;
                        out.push((k, Value::Str(v.clone())));
                    }
                    Tok::Open => {
                        *i += 1;
                        let o = parse_obj(t, i, false)?;
                        out.push((k, Value::Obj(o)));
                    }
                    Tok::Close => return None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_folders() {
        let text = r#"
"libraryfolders"
{
    "0"
    {
        "path"      "C:\\Program Files (x86)\\Steam"
        "apps" { "228980" "1234" }
    }
    // comment
    "1" { "path" "D:\\SteamLibrary" }
}"#;
        let doc = parse(text).unwrap();
        let Some(Value::Obj(folders)) = get(&doc, "libraryfolders") else {
            panic!()
        };
        let paths: Vec<_> = folders
            .iter()
            .filter_map(|(_, v)| match v {
                Value::Obj(f) => match get(f, "path") {
                    Some(Value::Str(p)) => Some(p.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            paths,
            ["C:\\Program Files (x86)\\Steam", "D:\\SteamLibrary"]
        );
    }
}

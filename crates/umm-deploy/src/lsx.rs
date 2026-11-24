//! The list of enabled modules in a Larian game's `modsettings.lsx` (Baldur's Gate 3): the
//! `<node id="ModuleShortDesc">` children of `<node id="Mods">`, under the file's `root`
//! node. Only those nodes change; the rest of the file keeps its bytes.

use roxmltree::{Document, Node};
use std::ops::Range;

/// The module a `ModuleShortDesc` node names: its `UUID`, lower-case (or the node's text,
/// trimmed, when it has none).
pub fn id(entry: &str) -> String {
    Document::parse(entry)
        .ok()
        .and_then(|d| attribute(d.root_element(), "UUID").map(str::to_ascii_lowercase))
        .unwrap_or_else(|| entry.trim().to_string())
}

/// The value of a node's `<attribute id="…" value="…"/>` child.
pub fn attribute<'a>(node: Node<'a, '_>, id: &str) -> Option<&'a str> {
    node.children()
        .find(|c| c.has_tag_name("attribute") && c.attribute("id") == Some(id))
        .and_then(|c| c.attribute("value"))
}

/// The list's node: `<node id="<key>">` among the `root` node's children.
fn list<'a, 'i>(doc: &'a Document<'i>, key: &str) -> Option<Node<'a, 'i>> {
    doc.descendants().find(|n| {
        n.has_tag_name("node")
            && n.attribute("id") == Some(key)
            && n.parent_element().is_some_and(|c| {
                c.has_tag_name("children")
                    && c.parent_element()
                        .is_some_and(|r| r.attribute("id") == Some("root"))
            })
    })
}

fn entries<'a, 'i: 'a>(list: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    list.children()
        .filter(|c| c.has_tag_name("children"))
        .flat_map(|c| c.children())
        .filter(|n| n.has_tag_name("node"))
}

fn split_bom(text: &str) -> (&str, &str) {
    match text.strip_prefix('\u{feff}') {
        Some(body) => ("\u{feff}", body),
        None => ("", text),
    }
}

/// The list's entries (each node's XML as written), or `None` when the file has no such list
/// (or is not XML).
pub fn get(text: &str, key: &str) -> Option<Vec<String>> {
    let (_, body) = split_bom(text);
    let doc = Document::parse(body).ok()?;
    let node = list(&doc, key)?;
    Some(entries(node).map(|n| body[n.range()].to_string()).collect())
}

/// The span of a node, with its line when it has one to itself.
fn line_span(text: &str, r: Range<usize>) -> Range<usize> {
    let b = text.as_bytes();
    let mut start = r.start;
    while start > 0 && matches!(b[start - 1], b' ' | b'\t') {
        start -= 1;
    }
    if start > 0 && b[start - 1] != b'\n' {
        return r;
    }
    let mut end = r.end;
    while end < b.len() && matches!(b[end], b' ' | b'\t') {
        end += 1;
    }
    if text[end..].starts_with("\r\n") {
        end += 2;
    } else if text[end..].starts_with('\n') {
        end += 1;
    } else if end < b.len() {
        return r;
    }
    start..end
}

/// The text with exactly these entries in the list, by [`id`]: those it has stay as they are
/// written, the others go, and new ones are added at its end. `None` when the file has no
/// such list.
pub fn set(text: &str, key: &str, wanted: &[String]) -> Option<String> {
    let (bom, body) = split_bom(text);
    let doc = Document::parse(body).ok()?;
    let node = list(&doc, key)?;
    let nl = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let ids: Vec<String> = wanted.iter().map(|e| id(e)).collect();
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    for n in entries(node) {
        let i = id(&body[n.range()]);
        if ids.contains(&i) && !kept.contains(&i) {
            kept.push(i);
        } else {
            edits.push((line_span(body, n.range()), String::new()));
        }
    }
    let mut new: Vec<&str> = Vec::new();
    for (e, i) in wanted.iter().zip(&ids) {
        if !kept.contains(i) {
            kept.push(i.clone());
            new.push(e.trim());
        }
    }
    if !new.is_empty() {
        match node.children().find(|c| c.has_tag_name("children")) {
            Some(ch) => {
                let r = ch.range();
                match body[r.clone()].rfind("</") {
                    Some(close) => {
                        let at = r.start + close;
                        let line = body[..at].rfind('\n').map_or(0, |p| p + 1);
                        let indent = &body[line..at];
                        if indent.trim().is_empty() {
                            // On lines of their own, one level in from `</children>`.
                            let unit = if indent.is_empty() || indent.contains('\t') {
                                "\t"
                            } else {
                                "    "
                            };
                            let mut s = String::new();
                            for e in &new {
                                s.push_str(&format!("{indent}{unit}{e}{nl}"));
                            }
                            edits.push((line..line, s));
                        } else {
                            edits.push((at..at, new.concat()));
                        }
                    }
                    // `<children/>`
                    None => edits.push((r, format!("<children>{}</children>", new.concat()))),
                }
            }
            None => {
                let r = node.range();
                let block = format!("<children>{}</children>", new.concat());
                if body[r.clone()].ends_with("/>") {
                    edits.push((r.end - 2..r.end, format!(">{block}</node>")));
                } else {
                    let close = r.start + body[r.clone()].rfind("</")?;
                    edits.push((close..close, block));
                }
            }
        }
    }
    let mut out = body.to_string();
    edits.sort_by(|a, b| b.0.start.cmp(&a.0.start).then(b.0.end.cmp(&a.0.end)));
    for (r, s) in edits {
        out.replace_range(r, &s);
    }
    Some(format!("{bom}{out}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<save>\r\n    <version major=\"4\" minor=\"8\" revision=\"0\" build=\"200\"/>\r\n    <region id=\"ModuleSettings\">\r\n        <node id=\"root\">\r\n            <children>\r\n                <node id=\"Mods\">\r\n                    <children>\r\n                        <node id=\"ModuleShortDesc\">\r\n                            <attribute id=\"Folder\" type=\"LSString\" value=\"GustavX\"/>\r\n                            <attribute id=\"UUID\" type=\"guid\" value=\"cb555efe-2d9e-131f-8195-a89329d218ea\"/>\r\n                        </node>\r\n                    </children>\r\n                </node>\r\n            </children>\r\n        </node>\r\n    </region>\r\n</save>\r\n";

    fn entry(uuid: &str) -> String {
        format!("<node id=\"ModuleShortDesc\"><attribute id=\"Folder\" type=\"LSString\" value=\"F\"/><attribute id=\"UUID\" type=\"guid\" value=\"{uuid}\"/></node>")
    }

    #[test]
    fn edits_only_the_list() {
        let base = get(GAME, "Mods").unwrap();
        assert_eq!(base.len(), 1);
        assert_eq!(id(&base[0]), "cb555efe-2d9e-131f-8195-a89329d218ea");
        let mut all = base.clone();
        all.push(entry("AAAA-1"));
        let new = set(GAME, "Mods", &all).unwrap();
        assert!(new.contains(&format!(
            "                        </node>\r\n                        {}\r\n                    </children>",
            entry("AAAA-1")
        )));
        assert_eq!(get(&new, "Mods").unwrap().len(), 2);
        // Already there, by its UUID: nothing added.
        assert_eq!(
            set(&new, "Mods", &[base[0].clone(), entry("aaaa-1")]).unwrap(),
            new
        );
        // Taken out again: back to the bytes it had.
        assert_eq!(set(&new, "Mods", &base).unwrap(), GAME);

        let empty = "<save><region id=\"ModuleSettings\"><node id=\"root\"><children><node id=\"Mods\"/></children></node></region></save>";
        let one = set(empty, "Mods", &[entry("b")]).unwrap();
        assert_eq!(get(&one, "Mods").unwrap(), [entry("b")]);
        assert_eq!(
            set(&one, "Mods", &[]).map(|t| get(&t, "Mods").unwrap().len()),
            Some(0)
        );
        assert_eq!(get("<save/>", "Mods"), None);
        assert_eq!(set("not xml", "Mods", &[]), None);
    }
}

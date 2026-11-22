//! Reading `ModuleConfig.xml` (FOMOD schema 5.0, and the older files that mostly follow it).
//! Lenient: element names are matched without regard to case, unknown elements are skipped,
//! and missing optional parts take the schema's defaults.

use crate::*;
use roxmltree::{Document, Node, ParsingOptions};

/// The text of the file: UTF-16 (with or without a byte-order mark) or UTF-8.
fn decode(bytes: &[u8]) -> String {
    let utf16 = |b: &[u8], le: bool| {
        let units: Vec<u16> = b
            .chunks_exact(2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [b'<', 0, ..] => utf16(bytes, true),
        [0, b'<', ..] => utf16(bytes, false),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Drops the `<?xml ...?>` declaration: the text is already decoded, and a declared encoding
/// other than UTF-8 would be refused.
fn without_declaration(text: &str) -> &str {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with("<?xml") {
        if let Some(end) = t.find("?>") {
            return &t[end + 2..];
        }
    }
    t
}

fn is(n: &Node, name: &str) -> bool {
    n.is_element() && n.tag_name().name().eq_ignore_ascii_case(name)
}

fn child<'a, 'i>(n: &Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| is(c, name))
}

fn children<'a, 'i: 'a>(
    n: &Node<'a, 'i>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children().filter(move |c| is(c, name))
}

fn attr(n: &Node, name: &str) -> Option<String> {
    n.attributes()
        .find(|a| a.name().eq_ignore_ascii_case(name))
        .map(|a| a.value().to_string())
}

fn text(n: Option<Node>) -> String {
    n.map(|n| {
        n.descendants()
            .filter(|d| d.is_text())
            .filter_map(|d| d.text())
            .collect::<String>()
            .trim()
            .to_string()
    })
    .unwrap_or_default()
}

/// A path as written in the file: `\` or `/`, no leading or trailing separator.
fn path(p: &str) -> String {
    p.replace('\\', "/").trim_matches('/').to_string()
}

fn bool_attr(n: &Node, name: &str) -> bool {
    attr(n, name).is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
}

/// Sorts by the `order` attribute: `Ascending` (the schema's default) or `Descending` by
/// name, `Explicit` as written.
fn order<T>(items: &mut [T], order: Option<String>, name: impl Fn(&T) -> String) {
    match order.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("explicit") => {}
        Some("descending") => items.sort_by_key(|t| std::cmp::Reverse(name(t).to_lowercase())),
        _ => items.sort_by_key(|t| name(t).to_lowercase()),
    }
}

fn files(n: Option<Node>) -> Vec<FileItem> {
    let Some(n) = n else { return Vec::new() };
    n.children()
        .filter(|c| is(c, "file") || is(c, "folder"))
        .filter_map(|c| {
            let source = path(&attr(&c, "source")?);
            Some(FileItem {
                folder: is(&c, "folder"),
                destination: attr(&c, "destination").map(|d| path(&d)),
                priority: attr(&c, "priority")
                    .and_then(|p| p.trim().parse().ok())
                    .unwrap_or(0),
                always: bool_attr(&c, "alwaysInstall"),
                if_usable: bool_attr(&c, "installIfUsable"),
                source,
            })
        })
        .collect()
}

fn condition(n: &Node) -> Condition {
    let parts: Vec<Condition> = n
        .children()
        .filter(|c| c.is_element())
        .filter_map(|c| {
            let name = c.tag_name().name().to_ascii_lowercase();
            Some(match name.as_str() {
                "flagdependency" => Condition::Flag {
                    name: attr(&c, "flag")?,
                    value: attr(&c, "value").unwrap_or_default(),
                },
                "filedependency" => Condition::File {
                    path: path(&attr(&c, "file")?),
                    state: match attr(&c, "state")?.to_ascii_lowercase().as_str() {
                        "missing" => FileState::Missing,
                        "inactive" => FileState::Inactive,
                        _ => FileState::Active,
                    },
                },
                "gamedependency" | "fommdependency" | "fomodependency" => Condition::Version,
                "dependencies" => condition(&c),
                _ => return None,
            })
        })
        .collect();
    if attr(n, "operator").is_some_and(|o| o.eq_ignore_ascii_case("or")) {
        Condition::Any(parts)
    } else {
        Condition::All(parts)
    }
}

fn plugin_type(n: Option<Node>) -> PluginType {
    match n
        .and_then(|n| attr(&n, "name"))
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "required" => PluginType::Required,
        "recommended" => PluginType::Recommended,
        "notusable" => PluginType::NotUsable,
        "couldbeusable" => PluginType::CouldBeUsable,
        _ => PluginType::Optional,
    }
}

fn plugin(n: &Node) -> Plugin {
    let td = child(n, "typeDescriptor");
    let kind = match td.and_then(|t| child(&t, "dependencyType")) {
        Some(dt) => PluginKind::Dependent {
            default: plugin_type(child(&dt, "defaultType")),
            patterns: child(&dt, "patterns")
                .map(|ps| {
                    children(&ps, "pattern")
                        .map(|p| {
                            let when = child(&p, "dependencies")
                                .map(|d| condition(&d))
                                .unwrap_or(Condition::All(Vec::new()));
                            (when, plugin_type(child(&p, "type")))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        },
        None => PluginKind::Fixed(plugin_type(td.and_then(|t| child(&t, "type")))),
    };
    Plugin {
        name: attr(n, "name").unwrap_or_default().trim().to_string(),
        description: text(child(n, "description")),
        image: child(n, "image")
            .and_then(|i| attr(&i, "path"))
            .map(|p| path(&p)),
        files: files(child(n, "files")),
        flags: child(n, "conditionFlags")
            .map(|f| {
                children(&f, "flag")
                    .filter_map(|fl| Some((attr(&fl, "name")?, text(Some(fl)))))
                    .collect()
            })
            .unwrap_or_default(),
        kind,
    }
}

fn group_kind(s: Option<String>) -> GroupKind {
    match s.unwrap_or_default().to_ascii_lowercase().as_str() {
        "selectexactlyone" => GroupKind::ExactlyOne,
        "selectatmostone" => GroupKind::AtMostOne,
        "selectatleastone" => GroupKind::AtLeastOne,
        "selectall" => GroupKind::All,
        _ => GroupKind::Any,
    }
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Installer, FomodError> {
    let decoded = decode(bytes);
    let opts = ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = Document::parse_with_options(without_declaration(&decoded), opts)
        .map_err(|e| FomodError::Xml(e.to_string()))?;
    let root = doc.root_element();
    if !is(&root, "config") {
        return Err(FomodError::NoConfig);
    }
    let mut steps: Vec<Step> = child(&root, "installSteps")
        .map(|s| {
            let mut v: Vec<Step> = children(&s, "installStep")
                .map(|st| {
                    let mut groups: Vec<Group> = child(&st, "optionalFileGroups")
                        .map(|g| {
                            let mut gs: Vec<Group> = children(&g, "group")
                                .map(|gr| {
                                    let mut plugins: Vec<Plugin> = child(&gr, "plugins")
                                        .map(|p| {
                                            children(&p, "plugin").map(|pl| plugin(&pl)).collect()
                                        })
                                        .unwrap_or_default();
                                    order(
                                        &mut plugins,
                                        child(&gr, "plugins").and_then(|p| attr(&p, "order")),
                                        |p| p.name.clone(),
                                    );
                                    Group {
                                        name: attr(&gr, "name")
                                            .unwrap_or_default()
                                            .trim()
                                            .to_string(),
                                        kind: group_kind(attr(&gr, "type")),
                                        plugins,
                                    }
                                })
                                .collect();
                            order(&mut gs, attr(&g, "order"), |g| g.name.clone());
                            gs
                        })
                        .unwrap_or_default();
                    groups.retain(|g| !g.plugins.is_empty());
                    Step {
                        name: attr(&st, "name").unwrap_or_default().trim().to_string(),
                        visible: child(&st, "visible").map(|v| {
                            // `<visible>` holds the conditions directly, or a `<dependencies>`.
                            match child(&v, "dependencies") {
                                Some(d) if v.children().filter(|c| c.is_element()).count() == 1 => {
                                    condition(&d)
                                }
                                _ => condition(&v),
                            }
                        }),
                        groups,
                    }
                })
                .collect();
            order(&mut v, attr(&s, "order"), |s| s.name.clone());
            v
        })
        .unwrap_or_default();
    steps.retain(|s| !s.groups.is_empty());
    Ok(Installer {
        name: text(child(&root, "moduleName")),
        image: child(&root, "moduleImage")
            .and_then(|i| attr(&i, "path"))
            .map(|p| path(&p)),
        required: files(child(&root, "requiredInstallFiles")),
        steps,
        conditional: child(&root, "conditionalFileInstalls")
            .and_then(|c| child(&c, "patterns"))
            .map(|ps| {
                children(&ps, "pattern")
                    .map(|p| ConditionalFiles {
                        when: child(&p, "dependencies")
                            .map(|d| condition(&d))
                            .unwrap_or(Condition::All(Vec::new())),
                        files: files(child(&p, "files")),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        needs: child(&root, "moduleDependencies").map(|d| condition(&d)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16_with_and_without_a_mark() {
        let s = "<config/>";
        let le: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode(&[[0xFF, 0xFE].as_slice(), &le].concat()), s);
        assert_eq!(decode(&le), s);
        assert_eq!(decode(b"\xEF\xBB\xBF<config/>"), s);
        assert_eq!(
            without_declaration("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n<config/>").trim(),
            "<config/>"
        );
    }
}

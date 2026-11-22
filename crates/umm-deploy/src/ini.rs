//! Reading and changing one key of an ini file, leaving every other line as it is.

/// The line ending the text uses.
fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn section_of(line: &str) -> Option<&str> {
    let t = line.trim();
    t.strip_prefix('[')?.strip_suffix(']').map(str::trim)
}

/// The key a line sets, if it is a `key=value` line.
fn key_of(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    if t.starts_with([';', '#']) {
        return None;
    }
    let (k, v) = t.split_once('=')?;
    Some((k.trim(), v.trim()))
}

/// Whether the text has the section.
pub(crate) fn has_section(text: &str, section: &str) -> bool {
    text.lines()
        .any(|l| section_of(l).is_some_and(|s| s.eq_ignore_ascii_case(section)))
}

/// The text without the section, if it sets no key (with the blank line [`set`] put before
/// it). Otherwise unchanged.
pub(crate) fn remove_empty_section(text: &str, section: &str) -> String {
    let nl = newline(text);
    let mut lines: Vec<&str> = text.lines().collect();
    let Some(at) = lines
        .iter()
        .position(|l| section_of(l).is_some_and(|s| s.eq_ignore_ascii_case(section)))
    else {
        return text.to_string();
    };
    let end = lines[at + 1..]
        .iter()
        .position(|l| section_of(l).is_some())
        .map_or(lines.len(), |i| at + 1 + i);
    if lines[at + 1..end].iter().any(|l| key_of(l).is_some()) {
        return text.to_string();
    }
    let start = if at > 0 && lines[at - 1].trim().is_empty() {
        at - 1
    } else {
        at
    };
    lines.drain(start..end);
    let mut out = lines.join(nl);
    if !out.is_empty() && text.ends_with('\n') {
        out.push_str(nl);
    }
    out
}

/// Whether the text sets any key at all (beyond section headers, comments and blank lines).
pub(crate) fn has_keys(text: &str) -> bool {
    text.lines()
        .any(|l| section_of(l).is_none() && key_of(l).is_some())
}

/// The value of `key` in `section`, if the file sets it. Names compare without regard to case.
pub(crate) fn get(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current: Option<&str> = None;
    for line in text.lines() {
        if let Some(s) = section_of(line) {
            current = Some(s);
            continue;
        }
        if current.is_some_and(|c| c.eq_ignore_ascii_case(section)) {
            if let Some((k, v)) = key_of(line) {
                if k.eq_ignore_ascii_case(key) {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// The text with `key` in `section` set to `value`, or removed with `None`. The section is
/// added at the end when missing; other lines are kept exactly.
pub(crate) fn set(text: &str, section: &str, key: &str, value: Option<&str>) -> String {
    let nl = newline(text);
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let mut current: Option<String> = None;
    let mut found: Option<usize> = None;
    let mut last_in_section: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(s) = section_of(line) {
            current = Some(s.to_string());
            if s.eq_ignore_ascii_case(section) {
                last_in_section = Some(i);
            }
            continue;
        }
        if current
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case(section))
        {
            if !line.trim().is_empty() {
                last_in_section = Some(i);
            }
            if found.is_none() && key_of(line).is_some_and(|(k, _)| k.eq_ignore_ascii_case(key)) {
                found = Some(i);
            }
        }
    }
    match (found, value) {
        (Some(i), Some(v)) => {
            let k = key_of(&lines[i])
                .map(|(k, _)| k.to_string())
                .unwrap_or_else(|| key.to_string());
            lines[i] = format!("{k}={v}");
        }
        (Some(i), None) => {
            lines.remove(i);
        }
        (None, Some(v)) => match last_in_section {
            Some(i) => lines.insert(i + 1, format!("{key}={v}")),
            None => {
                if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.push(format!("[{section}]"));
                lines.push(format!("{key}={v}"));
            }
        },
        (None, None) => {}
    }
    let mut out = lines.join(nl);
    if text.is_empty() || text.ends_with('\n') || (found.is_none() && value.is_some()) {
        out.push_str(nl);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFS: &str = "[Display]\r\niSize W=1920\r\n\r\n[Launcher]\r\nbEnableFileSelection=0\r\nuLastAspectRatio=1\r\n";

    #[test]
    fn reads_and_changes_one_key() {
        assert_eq!(
            get(PREFS, "launcher", "BENABLEFILESELECTION").as_deref(),
            Some("0")
        );
        assert_eq!(get(PREFS, "Display", "bEnableFileSelection"), None);
        let on = set(PREFS, "Launcher", "bEnableFileSelection", Some("1"));
        assert_eq!(
            on,
            PREFS.replace("bEnableFileSelection=0", "bEnableFileSelection=1")
        );
        assert_eq!(
            set(&on, "Launcher", "bEnableFileSelection", Some("0")),
            PREFS
        );
    }

    #[test]
    fn adds_and_removes_a_missing_key_or_section() {
        let no_key = "[Launcher]\r\nuLastAspectRatio=1\r\n\r\n[Display]\r\niSize W=1920\r\n";
        let added = set(no_key, "Launcher", "bEnableFileSelection", Some("1"));
        assert_eq!(added, "[Launcher]\r\nuLastAspectRatio=1\r\nbEnableFileSelection=1\r\n\r\n[Display]\r\niSize W=1920\r\n");
        assert_eq!(
            set(&added, "Launcher", "bEnableFileSelection", None),
            no_key
        );

        let no_section = "[Display]\niSize W=1920\n";
        let added = set(no_section, "Launcher", "bEnableFileSelection", Some("1"));
        assert_eq!(
            added,
            "[Display]\niSize W=1920\n\n[Launcher]\nbEnableFileSelection=1\n"
        );
        assert_eq!(
            get(&added, "Launcher", "bEnableFileSelection").as_deref(),
            Some("1")
        );
    }
}

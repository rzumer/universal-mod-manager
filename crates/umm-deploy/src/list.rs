//! Lists in files outside the game folder that UMM adds entries to while mods are deployed.

use serde::{Deserialize, Serialize};

/// How a list is written in its file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListFormat {
    /// A Paradox block, `key={ "a" "b" }` (Crusader Kings II's `settings.txt`): entries are
    /// the strings in it.
    #[default]
    Block,
    /// A node of nodes in a Larian `.lsx` file (Baldur's Gate 3's `modsettings.lsx`): entries
    /// are the nodes' XML, the same entry when their `UUID` is.
    Lsx,
}

impl ListFormat {
    pub(crate) fn is_default(&self) -> bool {
        *self == ListFormat::Block
    }

    /// The file's text.
    pub fn decode(self, bytes: &[u8]) -> String {
        match self {
            ListFormat::Block => crate::block::decode(bytes),
            ListFormat::Lsx => String::from_utf8_lossy(bytes).into_owned(),
        }
    }

    /// Back to the file's bytes.
    pub fn encode(self, text: &str) -> Vec<u8> {
        match self {
            ListFormat::Block => crate::block::encode(text),
            ListFormat::Lsx => text.as_bytes().to_vec(),
        }
    }

    /// The list's entries, or `None` when the file has none.
    pub fn get(self, text: &str, key: &str) -> Option<Vec<String>> {
        match self {
            ListFormat::Block => crate::block::get(text, key),
            ListFormat::Lsx => crate::lsx::get(text, key),
        }
    }

    /// The text with the list holding these entries, or without it for `None` (an `.lsx`
    /// list stays, emptied). `None` when an `.lsx` file has no such list to change.
    pub fn set(self, text: &str, key: &str, entries: Option<&[String]>) -> Option<String> {
        match self {
            ListFormat::Block => Some(crate::block::set(text, key, entries)),
            ListFormat::Lsx => crate::lsx::set(text, key, entries.unwrap_or_default()),
        }
    }

    /// What makes two entries the same.
    pub fn id(self, entry: &str) -> String {
        match self {
            ListFormat::Block => entry.to_string(),
            ListFormat::Lsx => crate::lsx::id(entry),
        }
    }
}

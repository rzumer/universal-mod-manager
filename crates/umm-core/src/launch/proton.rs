//! A Windows game run through Proton: the DLL its mod loader replaces, and paths as Wine
//! sees them.

use super::*;

/// The DLL a mod loader replaces, which Wine loads instead of its own only when told to.
pub(super) fn proton_override(i: &Injection) -> Option<&'static str> {
    match i {
        Injection::Doorstop(_) | Injection::BepisLoader { .. } => Some("winhttp"),
        Injection::MelonLoader { .. } | Injection::Lovely => Some("version"),
        _ => None,
    }
}

/// A path as a Windows program run through Wine sees it: Wine's `Z:` drive is the Linux
/// file system's root.
pub(super) fn wine_path(p: &Path) -> String {
    format!("Z:{}", p.display().to_string().replace('/', "\\"))
}

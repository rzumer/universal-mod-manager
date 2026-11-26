//! The Steam Workshop in a window of UMM's: Steam's community site, where the user signs in
//! with Steam's own sign-in page, browses, and subscribes. Steam's client downloads what is
//! subscribed to; UMM watches Steam's record of the game's items meanwhile and tells the main
//! window when it changes.
//!
//! The site has no access to UMM: only the main window may call its commands. Pages of other
//! sites a Workshop page links to open in the default browser, and `steam://` links in the
//! Steam client.

use crate::commands::{blocking, open_url, Res};
use crate::AppState;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Runtime, State};

const WINDOW: &str = "steam-workshop";

/// How often Steam's record is checked while the window is open.
const EVERY: Duration = Duration::from_secs(3);

/// How long Steam's record is still watched after the window closes: Steam may finish a
/// download after the last subscription.
const AFTER: Duration = Duration::from_secs(120);

/// How long it is watched after subscribing or unsubscribing from UMM's own tab, with no
/// window: Steam notes the change when it next looks, and downloads meanwhile.
const AFTER_ACTION: Duration = Duration::from_secs(15 * 60);

/// Hosts the window shows itself: Steam's community site, store and sign-in pages, and what
/// they are served from. Anything else opens in the default browser.
fn is_steam(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    [
        "steamcommunity.com",
        "steampowered.com",
        "steamstatic.com",
        "steamusercontent.com",
        "steamcontent.com",
    ]
    .iter()
    .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// What the window does with an address: show it, or hand it to the system.
fn route(url: &tauri::Url) -> bool {
    match url.scheme() {
        "https" if url.host_str().is_some_and(is_steam) => true,
        "about" | "blob" | "data" => true,
        "https" | "http" | "steam" => {
            let _ = open_url(url.as_str());
            false
        }
        _ => false,
    }
}

/// Opens the game's Workshop (or one item's page) in UMM's Steam Workshop window, and
/// watches Steam's record of the game's items, emitting `workshop-changed` (with the game's
/// id) when it changes.
#[tauri::command]
pub async fn open_workshop_window<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    item: Option<String>,
) -> Res<()> {
    let (url, record) = {
        let game = game.clone();
        blocking(&state, move |s| {
            Ok((
                s.workshop_web_page(&game, item.as_deref())?,
                s.workshop_record_files(&game)?,
            ))
        })
        .await?
    };
    open_window(&app, &url)?;
    watch(app, game, record, AFTER);
    Ok(())
}

/// Opens Steam's sign-in page in UMM's Steam Workshop window, going on to the game's
/// Workshop once signed in. The user types into Steam's page; UMM never sees it.
#[tauri::command]
pub async fn open_steam_sign_in<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
) -> Res<()> {
    let (app_id, record) = {
        let game = game.clone();
        blocking(&state, move |s| {
            let def = s.def(&game)?.def;
            let app_id = def.detect.steam_app_id.ok_or_else(|| {
                umm_core::CoreError::Invalid(format!("{} is not a Steam game", def.name))
            })?;
            Ok((app_id, s.workshop_record_files(&game)?))
        })
        .await?
    };
    open_window(
        &app,
        &format!("https://steamcommunity.com/login/home/?goto=app%2F{app_id}%2Fworkshop%2F"),
    )?;
    watch(app, game, record, AFTER);
    Ok(())
}

fn open_window<R: Runtime>(app: &AppHandle<R>, url: &str) -> Res<()> {
    use tauri::webview::NewWindowResponse;
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
    let parsed: tauri::Url = url.parse().map_err(|e| format!("{url}: {e}"))?;
    if let Some(w) = app.get_webview_window(WINDOW) {
        w.navigate(parsed).map_err(|e| e.to_string())?;
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    let opener = app.clone();
    WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::External(parsed))
        .title("Steam Workshop – UMM")
        .inner_size(1180.0, 860.0)
        .min_inner_size(640.0, 480.0)
        .on_navigation(route)
        // Each page shown, so the main window can see whether a sign-in it asked for is done.
        .on_page_load(|w, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                let _ = w.emit("steam-window-page", ());
            }
        })
        // Links that open a new window: Steam's in this one, others in the browser.
        .on_new_window(move |u, _| {
            if u.scheme() == "https" && u.host_str().is_some_and(is_steam) {
                if let Some(w) = opener.get_webview_window(WINDOW) {
                    let _ = w.navigate(u);
                }
            } else {
                route(&u);
            }
            NewWindowResponse::Deny
        })
        .build()
        .map_err(|e| format!("opening the Steam Workshop window: {e}"))?;
    Ok(())
}

/// One watch at a time: a new one (another game, or the window opened again) ends the last.
static WATCH: std::sync::Mutex<Option<Arc<AtomicBool>>> = std::sync::Mutex::new(None);

/// Watches Steam's record of the game's items while the window is open, and for `after`
/// once it is not.
fn watch<R: Runtime>(app: AppHandle<R>, game: String, record: Vec<PathBuf>, after: Duration) {
    use tauri::Manager;
    let stop = Arc::new(AtomicBool::new(false));
    if let Ok(mut w) = WATCH.lock() {
        if let Some(old) = w.replace(Arc::clone(&stop)) {
            old.store(true, Ordering::Relaxed);
        }
    }
    std::thread::spawn(move || {
        let stamp = |files: &[PathBuf]| -> Vec<Option<SystemTime>> {
            files
                .iter()
                .map(|f| std::fs::metadata(f).ok()?.modified().ok())
                .collect()
        };
        let mut last = stamp(&record);
        let mut closed_at: Option<Instant> = None;
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(EVERY);
            let now = stamp(&record);
            if now != last {
                last = now;
                let _ = app.emit("workshop-changed", &game);
            }
            match (app.get_webview_window(WINDOW), closed_at) {
                (Some(_), _) => closed_at = None,
                (None, None) => closed_at = Some(Instant::now()),
                (None, Some(t)) if t.elapsed() > after => break,
                _ => {}
            }
        }
    });
}

/// A hidden window on Steam's Workshop site, which subscribes and unsubscribes for the user
/// with their Steam session (signed in to in the Steam Workshop window, whose cookies it
/// shares). Its pages have no access to UMM; it reports by going to an address UMM stops
/// (`/umm-result?…`), which `workshop-action` relays to the main window.
const SESSION: &str = "steam-session";

/// The address the session window's script reports to.
const RESULT_PATH: &str = "/umm-result";

/// What the session window does once its page has loaded.
#[derive(Clone)]
struct Action {
    game: String,
    app_id: u32,
    /// `subscribe`, `unsubscribe`, or `status` (whether the user is signed in).
    kind: &'static str,
    item: Option<String>,
}

static PENDING: std::sync::Mutex<Option<Action>> = std::sync::Mutex::new(None);

/// The script that does an action on Steam's page and reports how it went: `ok`, `login`
/// (not signed in) or `error`, with Steam's reason (`why`) when it gives one.
///
/// It runs on the item's own page (or, to check the sign-in, the Workshop's), and subscribes
/// as that page's Subscribe button does: `/sharedfiles/subscribe` with the page's session id.
fn script(a: &Action) -> String {
    let meta = serde_json::json!({
        "game": a.game,
        "action": a.kind,
        "item": a.item.clone().unwrap_or_default(),
    });
    format!(
        r#"(async () => {{
  const meta = {meta};
  const report = (r, why = "") => location.replace("https://steamcommunity.com{RESULT_PATH}?" + new URLSearchParams({{ ...meta, r, why }}));
  try {{
    // Steam's newer pages, like the Workshop's, keep the user in `UserConfig`; its older ones,
    // like an item's page, set `g_steamID` (false when signed out).
    const user = window.UserConfig;
    const signedIn = user ? !!user.logged_in : typeof window.g_steamID !== "undefined" ? !!window.g_steamID : null;
    if (signedIn === null) return report("error", "not a page UMM knows");
    if (!signedIn) return report("login");
    if (meta.action === "status") return report("ok");
    const sid = window.g_sessionID || (document.cookie.match(/(?:^|; )sessionid=([^;]+)/) || [])[1] || "";
    if (!sid) return report("error", "no session id on Steam's page");
    const params = {{ id: meta.item, appid: "{app}", sessionid: sid }};
    if (meta.action === "subscribe") params.include_dependencies = "false";
    const res = await fetch("/sharedfiles/" + meta.action, {{
      method: "POST",
      credentials: "same-origin",
      headers: {{ "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8" }},
      body: new URLSearchParams(params),
    }});
    const j = await res.json().catch(() => null);
    if (j && j.success === 1) return report("ok");
    report("error", j ? "Steam result " + j.success : "HTTP " + res.status);
  }} catch (e) {{
    report("error", String(e));
  }}
}})();"#,
        app = a.app_id
    )
}

/// Subscribes to a Workshop item (`subscribe`), unsubscribes (`unsubscribe`), or checks that
/// the user is signed in to Steam (`status`, no item), through the hidden session window.
/// The answer comes as a `workshop-action` event: `{ game, action, item, result }`, with
/// `result` one of `ok`, `login` or `error`.
#[tauri::command]
pub async fn workshop_action<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    game: String,
    action: String,
    item: Option<String>,
) -> Res<()> {
    let kind = match action.as_str() {
        "subscribe" => "subscribe",
        "unsubscribe" => "unsubscribe",
        "status" => "status",
        _ => return Err(format!("unknown Workshop action `{action}`")),
    };
    if kind != "status"
        && !item
            .as_deref()
            .is_some_and(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_digit()))
    {
        return Err("a Workshop item id is needed".into());
    }
    let (app_id, page, record) = {
        let game = game.clone();
        blocking(&state, move |s| {
            let def = s.def(&game)?.def;
            let app_id = def.detect.steam_app_id.ok_or_else(|| {
                umm_core::CoreError::Invalid(format!("{} is not a Steam game", def.name))
            })?;
            Ok((
                app_id,
                umm_core::workshop::browse_url(app_id, &Default::default()),
                s.workshop_record_files(&game)?,
            ))
        })
        .await?
    };
    // Steam notes a change in its record of the game's items a while after: the main window
    // hears of it then.
    if kind != "status" {
        watch(app.clone(), game.clone(), record, AFTER_ACTION);
    }
    // An item's own page, whose Subscribe button the script does the work of.
    let page = match &item {
        Some(id) if kind != "status" => {
            format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}")
        }
        _ => page,
    };
    if let Ok(mut p) = PENDING.lock() {
        *p = Some(Action {
            game,
            app_id,
            kind,
            item,
        });
    }
    // The page is loaded afresh each time, so it sees a sign-in made since.
    open_session(&app, &page)
}

fn open_session<R: Runtime>(app: &AppHandle<R>, url: &str) -> Res<()> {
    use tauri::webview::PageLoadEvent;
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
    let parsed: tauri::Url = url.parse().map_err(|e| format!("{url}: {e}"))?;
    if let Some(w) = app.get_webview_window(SESSION) {
        return w.navigate(parsed).map_err(|e| e.to_string());
    }
    let relay = app.clone();
    WebviewWindowBuilder::new(app, SESSION, WebviewUrl::External(parsed))
        .title("Steam – UMM")
        .visible(false)
        .skip_taskbar(true)
        .on_navigation(move |u| {
            if u.host_str() == Some("steamcommunity.com") && u.path() == RESULT_PATH {
                let get = |k: &str| {
                    u.query_pairs()
                        .find(|(key, _)| key == k)
                        .map(|(_, v)| v.into_owned())
                        .unwrap_or_default()
                };
                let _ = relay.emit(
                    "workshop-action",
                    serde_json::json!({
                        "game": get("game"),
                        "action": get("action"),
                        "item": get("item"),
                        "result": get("r"),
                        "why": get("why"),
                    }),
                );
                return false;
            }
            u.scheme() == "https" && u.host_str().is_some_and(is_steam)
        })
        .on_page_load(|w, payload| {
            if payload.event() != PageLoadEvent::Finished
                || payload.url().host_str() != Some("steamcommunity.com")
            {
                return;
            }
            let action = PENDING.lock().ok().and_then(|mut p| p.take());
            if let Some(a) = action {
                let _ = w.eval(script(&a));
            }
        })
        .build()
        .map_err(|e| format!("opening Steam's page: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_script_carries_its_action_as_data() {
        let s = script(&Action {
            game: "odd\"game</script>".into(),
            app_id: 2212330,
            kind: "subscribe",
            item: Some("42".into()),
        });
        // The game id is JSON in the script, never code.
        assert!(s.contains(r#""game":"odd\"game</script>""#), "{s}");
        assert!(s.contains(r#""action":"subscribe""#) && s.contains(r#""item":"42""#));
        assert!(s.contains(r#"appid: "2212330""#));
        assert!(s.contains(&format!("https://steamcommunity.com{RESULT_PATH}?")));
        assert!(s.contains(r#"fetch("/sharedfiles/" + meta.action"#));
        // Read from where Steam's pages leave it once started.
        assert!(s.contains("const user = window.UserConfig;"));
        assert!(s.contains("window.g_sessionID"));
    }

    #[test]
    fn only_steams_own_pages_open_in_the_window() {
        assert!(is_steam("steamcommunity.com"));
        assert!(is_steam("store.steampowered.com"));
        assert!(is_steam("login.steampowered.com"));
        assert!(is_steam("Community.Akamai.SteamStatic.com"));
        assert!(!is_steam("steamcommunity.com.example.net"));
        assert!(!is_steam("notsteamcommunity.com"));
        assert!(!is_steam("github.com"));
    }
}

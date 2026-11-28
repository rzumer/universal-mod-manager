import { $, h } from "./dom.js";
import { action } from "./tabs/gamebanana.js";

// ---------- state ----------

export const S = {
  startup: null,
  games: [],
  gameId: null,
  details: null,
  profileName: "Default",
  profile: null,
  tab: "mods",
  plan: null,
  health: null,
  busy: false,
  session: { game: null, phase: "idle", log: [] }, // phase: idle|deploying|running|waiting|reverting
  lastReport: {},
  // Patcher mods' status, by `game/mod`: `{ s, gen, loading }`, loaded as their rows show
  // and again after each refresh (`patchersGen`).
  patchers: {},
  patchersGen: 0,
  // The profile's WeiDU mods' install: `{ key, status, gen, loading }`.
  weidu: { key: null, status: null, gen: -1, loading: false },
  // Thunderstore tab: the query, the page shown (null while loading), and an install/update
  // in progress ({ message }).
  ts: { game: null, query: { text: "", category: "", sort: "rating", installedOnly: false, offset: 0, limit: 40 }, page: null, busy: null },
  // Whether Steam games have a Workshop, by game: true, false, or null (not known).
  workshopAvailable: {},
  // The Workshop tab: the game it shows, and its items Steam has here (or "loading").
  workshop: {
    game: null,
    list: null,
    // Items subscribed to or unsubscribed from here that Steam's record does not show yet, by
    // game: `{ [game]: { [item]: item shown meanwhile, or false once unsubscribed } }`.
    pending: {},
    // Browsing (or only the items Steam has here, when `subscribedOnly`): the search, and its
    // results (or "loading").
    subscribedOnly: false,
    query: { text: "", sort: "trend", tag: "", page: 1 },
    found: null,
    // The search the browse results are for (game, words, sort, tag).
    foundKey: null,
    more: false,
    // Items being subscribed to or unsubscribed from, and whether the user is signed in.
    busy: {},
    signedIn: null,
    signInChecked: false,
  },
  // Nexus Mods tab: the game's Nexus game, the query and page shown, installed mods' updates,
  // and an install in progress ({ message }).
  // The Configs tab: the profile's config files, and the one open in the editor
  // (`{ path, view, edits, raw, text }`).
  config: { key: null, list: null, query: "", open: null },
  nexus: {
    game: null,
    picker: null,
    query: { text: "", sort: "endorsements", offset: 0, count: 40, installed: false },
    page: null,
    updates: null,
    busy: null,
    // "mods" or "collections"; the collections search, its results, and the collection
    // open as a checklist ({ slug, revision, data }), with the walk through its missing files.
    view: "mods",
    cquery: { text: "", sort: "endorsements", offset: 0, count: 20 },
    cpage: null,
    open: null,
    optional: false,
    walk: null,
  },
  // GameBanana tab: the game's GameBanana game, the query and page shown, installed mods'
  // updates, and an install in progress ({ message }).
  gb: {
    game: null,
    picker: null,
    query: { text: "", sort: "downloads", page: 1, installed: false },
    page: null,
    updates: null,
    busy: null,
    stale: false,
  },
};

/// Shows a message for long enough to read it: longer for longer text and for errors, longer
/// again when others are showing, and for as long as the pointer is over it.
export function toast(message, kind = "info", action = null) {
  const t = h(
    "div",
    { class: `toast ${kind}`, role: kind === "error" ? "alert" : "status" },
    message,
    action && h("button", { class: "small", onclick: () => { t.remove(); action.run(); } }, action.label),
  );
  const box = $("#toasts");
  const others = box.querySelectorAll(".toast:not(.leaving)").length;
  box.append(t);
  const base = kind === "error" ? 6000 : 3000;
  const reading = String(message).length * 60;
  const ms = base + reading + others * 2500;
  let timer;
  const start = (delay) => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      t.classList.add("leaving");
      setTimeout(() => t.remove(), 400);
    }, delay);
  };
  t.addEventListener("mouseenter", () => clearTimeout(timer));
  t.addEventListener("mouseleave", () => start(2000));
  start(ms);
}

export async function attempt(fn, success) {
  try {
    const r = await fn();
    if (success) toast(success, "ok");
    return r;
  } catch (e) {
    toast(String(e), "error");
    return undefined;
  }
}

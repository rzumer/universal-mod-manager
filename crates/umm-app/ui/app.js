import { api } from "./api.js";

// ---------- tiny DOM helpers ----------

function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v == null) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (k === "class") el.className = v;
    else if (k === "html") el.innerHTML = v;
    else if (v === true) el.setAttribute(k, "");
    else el.setAttribute(k, v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return el;
}

const $ = (sel) => document.querySelector(sel);

function fmtBytes(n) {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB"];
  let i = -1;
  do {
    n /= 1024;
    i++;
  } while (n >= 1024 && i < units.length - 1);
  return `${n.toFixed(n < 10 ? 1 : 0)} ${units[i]}`;
}

const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

function fmtDuration(s) {
  if (s < 60) return plural(s, "second");
  const m = Math.round(s / 60);
  if (m < 60) return plural(m, "minute");
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}

function fmtTime(d = new Date()) {
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

const ICONS = {
  grip: '<svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="5.5" cy="4" r="1.2"/><circle cx="10.5" cy="4" r="1.2"/><circle cx="5.5" cy="8" r="1.2"/><circle cx="10.5" cy="8" r="1.2"/><circle cx="5.5" cy="12" r="1.2"/><circle cx="10.5" cy="12" r="1.2"/></svg>',
  up: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 4l-4.5 5h9z"/></svg>',
  down: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 12l4.5-5h-9z"/></svg>',
  trash: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 2h4l.5 1H13v1.5H3V3h2.5zM4 5.5h8l-.6 8.5H4.6z"/></svg>',
  play: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 2.8v10.4L13 8z"/></svg>',
  folder: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M1.5 3.5h5l1.5 1.5h6.5v8.5h-13z"/></svg>',
};

const icon = (name) => h("span", { class: "icon", html: ICONS[name] });

// ---------- state ----------

const S = {
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
};

function toast(message, kind = "info") {
  const t = h("div", { class: `toast ${kind}`, role: kind === "error" ? "alert" : "status" }, message);
  $("#toasts").append(t);
  setTimeout(() => t.classList.add("leaving"), kind === "error" ? 7000 : 3500);
  setTimeout(() => t.remove(), kind === "error" ? 7400 : 3900);
}

async function attempt(fn, success) {
  try {
    const r = await fn();
    if (success) toast(success, "ok");
    return r;
  } catch (e) {
    toast(String(e), "error");
    return undefined;
  }
}

// ---------- modals ----------

function modal({ title, body, actions }) {
  return new Promise((resolve) => {
    const root = $("#modal-root");
    const close = (v) => {
      root.replaceChildren();
      document.removeEventListener("keydown", onKey);
      resolve(v);
    };
    const onKey = (e) => {
      if (e.key === "Escape") close(null);
    };
    document.addEventListener("keydown", onKey);
    const dlg = h(
      "div",
      { class: "modal", role: "dialog", "aria-modal": "true", "aria-label": title },
      h("h2", {}, title),
      h("div", { class: "modal-body" }, body),
      h(
        "div",
        { class: "modal-actions" },
        actions.map((a) =>
          h(
            "button",
            { class: a.class || "", onclick: () => close(a.value ?? a.label), autofocus: a.focus || false },
            a.label,
          ),
        ),
      ),
    );
    root.replaceChildren(h("div", { class: "scrim", onclick: (e) => e.target === e.currentTarget && close(null) }, dlg));
    dlg.querySelector("[autofocus]")?.focus();
  });
}

const confirmModal = (title, text, okLabel, danger = false) =>
  modal({
    title,
    body: h("p", {}, text),
    actions: [
      { label: "Cancel", value: false, class: "ghost" },
      { label: okLabel, value: true, class: danger ? "danger" : "primary", focus: true },
    ],
  });

function promptModal(title, label, placeholder = "") {
  const input = h("input", { type: "text", placeholder, "aria-label": label });
  const p = modal({
    title,
    body: h("label", { class: "field" }, h("span", {}, label), input),
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "OK", value: "__ok", class: "primary" },
    ],
  });
  setTimeout(() => {
    input.focus();
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") $("#modal-root .primary").click();
    });
  });
  return p.then((v) => (v === "__ok" ? input.value.trim() : null));
}

// ---------- data loading ----------

async function loadGames() {
  S.games = (await attempt(api.listGames)) || [];
  if (!S.gameId && S.games.length) S.gameId = S.games[0].id;
  renderSidebar();
}

async function loadGame() {
  if (!S.gameId) {
    S.details = null;
    renderMain();
    return;
  }
  const d = await attempt(() => api.gameDetails(S.gameId));
  if (!d) return;
  S.details = d;
  if (!d.profiles.includes(S.profileName)) S.profileName = "Default";
  S.profile = await attempt(() => api.getProfile(S.gameId, S.profileName));
  S.plan = null;
  S.health = null;
  renderMain();
  if (S.tab === "files") loadPlan();
  if (S.tab === "health") loadHealth();
}

async function loadPlan() {
  S.plan = "loading";
  renderMain();
  S.plan = (await attempt(() => api.previewPlan(S.gameId, S.profileName))) || null;
  renderMain();
}

async function loadHealth() {
  S.health = "loading";
  renderMain();
  S.health = (await attempt(() => api.doctor(S.gameId))) || null;
  renderMain();
}

async function refreshAll() {
  await loadGames();
  await loadGame();
}

async function saveProfile() {
  await attempt(() => api.saveProfile(S.gameId, S.profile));
  S.plan = null;
  if (S.tab === "files") loadPlan();
}

// ---------- sidebar ----------

function gameStatus(g) {
  if (S.session.game === g.id && S.session.phase !== "idle") return ["live", "Playing"];
  if (g.running) return ["live", "Running"];
  if (g.deployed) return ["warn", "Mods deployed"];
  if (!g.install) return ["muted", "Not found"];
  return ["ok", `${g.modCount} mod${g.modCount === 1 ? "" : "s"}`];
}

function renderSidebar() {
  const list = S.games.length
    ? h(
        "ul",
        { class: "game-list" },
        S.games.map((g) => {
          const [kind, text] = gameStatus(g);
          return h(
            "li",
            {},
            h(
              "button",
              {
                class: `game-item ${g.id === S.gameId ? "selected" : ""}`,
                "aria-current": g.id === S.gameId ? "true" : false,
                onclick: () => {
                  S.gameId = g.id;
                  S.profileName = "Default";
                  renderSidebar();
                  loadGame();
                },
              },
              h("span", { class: "game-name" }, g.name),
              h("span", { class: `status ${kind}` }, h("i", {}), text),
            ),
          );
        }),
      )
    : h(
        "div",
        { class: "sidebar-empty" },
        h("strong", {}, "No games yet"),
        h("p", {}, "Pick the games you want to mod from your Steam libraries."),
        h("button", { class: "primary small", onclick: addFromSteam }, "Add from Steam…"),
        S.startup &&
          h(
            "p",
            { class: "fine" },
            "For other games, put a definition file (.toml) in ",
            h("code", { title: S.startup.gamedefsDir }, S.startup.gamedefsDir),
            " and reload.",
          ),
      );

  $("#sidebar").replaceChildren(
    h("div", { class: "brand" }, h("img", { src: "icon.png", alt: "" }), h("span", {}, "Mod Manager")),
    h("div", { class: "section-label" }, "Games"),
    list,
    S.games.length > 0 ? h("button", { class: "ghost small add-steam", onclick: addFromSteam }, "+ Add from Steam…") : "",
    h(
      "div",
      { class: "sidebar-foot" },
      api.isMock && h("div", { class: "mock-note" }, "Preview with sample data"),
      S.startup &&
        h(
          "button",
          { class: "link", onclick: () => attempt(() => api.openFolder(S.startup.dataDir)) },
          icon("folder"),
          "Open data folder",
        ),
      h("button", { class: "link", onclick: () => location.reload() }, "Reload game definitions"),
      hiddenNotes().size > 0 &&
        h(
          "button",
          {
            class: "link",
            onclick: () => {
              setHiddenNotes(new Set());
              renderSidebar();
              toast("Hidden messages will show again", "ok");
            },
          },
          "Show hidden messages again",
        ),
    ),
  );
}

// ---------- main panel ----------

function header(d) {
  const g = d.game;
  const playing = S.session.game === g.id && S.session.phase !== "idle";
  const otherPlaying = S.session.phase !== "idle" && S.session.game !== g.id;
  const canLaunch = g.canLaunch && g.install && !playing && !otherPlaying && !g.running;
  let why = "";
  if (!g.canLaunch) why = "This game definition has no [launch] section.";
  else if (!g.install) why = "Set the game folder first.";
  else if (g.running) why = "The game is already running outside UMM.";
  else if (otherPlaying) why = "Another game is running through UMM.";

  const profileSelect = h(
    "select",
    {
      "aria-label": "Profile",
      disabled: playing,
      onchange: async (e) => {
        S.profileName = e.target.value;
        await loadGame();
      },
    },
    d.profiles.map((p) => h("option", { value: p, selected: p === S.profileName }, p)),
  );

  return h(
    "header",
    { class: "game-header" },
    h(
      "div",
      { class: "title-row" },
      h(
        "div",
        {},
        h("h1", {}, g.name),
        h(
          "div",
          { class: "install" },
          g.install
            ? h(
                "button",
                { class: "link path", title: "Open the game folder", onclick: () => attempt(() => api.openFolder(g.install)) },
                g.install,
              )
            : h("span", { class: "muted" }, "Game folder not found"),
          g.foundVia && h("span", { class: "chip" }, g.foundVia),
          h("button", { class: "link", onclick: changeInstall, disabled: playing }, g.install ? "Change…" : "Set folder…"),
        ),
        h(
          "div",
          { class: "chips" },
          g.loader && h("span", { class: "chip" }, `${g.loader} · ${g.inject}`),
          g.deployed && !playing && h("span", { class: "chip warn" }, "Mods are deployed"),
          g.running && h("span", { class: "chip live" }, "Game running"),
        ),
      ),
      h(
        "div",
        { class: "launch-box" },
        h(
          "div",
          { class: "profile-row" },
          h("label", { class: "inline" }, "Profile", profileSelect),
          h("button", { class: "ghost small", onclick: newProfile, disabled: playing }, "New"),
          S.profileName !== "Default" &&
            h("button", { class: "ghost small", onclick: deleteProfile, disabled: playing }, "Delete"),
        ),
        playing
          ? h(
              "button",
              { class: "primary launch", disabled: true },
              h("span", { class: "spinner" }),
              phaseLabel(S.session.phase),
            )
          : h("button", { class: "primary launch", disabled: !canLaunch, title: why || false, onclick: launch }, icon("play"), "Launch"),
        why && !playing && h("div", { class: "hint" }, why),
      ),
    ),
  );
}

function phaseLabel(p) {
  return (
    {
      deploying: "Deploying mods…",
      running: "Game running",
      waiting: "Waiting for Steam…",
      reverting: "Restoring folder…",
    }[p] || "Working…"
  );
}

function tabs() {
  const items = [
    ["mods", "Mods"],
    ["files", "Files"],
    ["health", "Health"],
    ["actions", "Actions"],
  ];
  return h(
    "nav",
    { class: "tabs", role: "tablist" },
    items.map(([id, label]) =>
      h(
        "button",
        {
          role: "tab",
          "aria-selected": S.tab === id ? "true" : "false",
          class: S.tab === id ? "active" : "",
          onclick: () => {
            S.tab = id;
            renderMain();
            if (id === "files" && !S.plan) loadPlan();
            if (id === "health" && !S.health) loadHealth();
          },
        },
        label,
        id === "health" && S.session.game === S.gameId && S.session.phase !== "idle" && h("span", { class: "dot live" }),
      ),
    ),
  );
}

// ----- Mods tab -----

let dragFrom = null;

function modsTab(d) {
  const playing = S.session.phase !== "idle" && S.session.game === S.gameId;
  const byId = Object.fromEntries(d.mods.map((m) => [m.id, m]));
  const entries = S.profile?.mods || [];
  const inProfile = new Set(entries.map((e) => e.id));
  const notIn = d.mods.filter((m) => !inProfile.has(m.id));

  const move = async (from, to) => {
    if (to < 0 || to >= entries.length || from === to) return;
    const [e] = entries.splice(from, 1);
    entries.splice(to, 0, e);
    renderMain();
    await saveProfile();
  };

  const rows = entries.map((e, i) => {
    const m = byId[e.id];
    const row = h(
      "li",
      {
        class: `mod-row ${e.enabled ? "" : "disabled"} ${m ? "" : "missing"}`,
        draggable: !playing,
        ondragstart: (ev) => {
          dragFrom = i;
          ev.dataTransfer.effectAllowed = "move";
          row.classList.add("dragging");
        },
        ondragend: () => row.classList.remove("dragging"),
        ondragover: (ev) => {
          if (dragFrom === null) return;
          ev.preventDefault();
          row.classList.add("drop-target");
        },
        ondragleave: () => row.classList.remove("drop-target"),
        ondrop: (ev) => {
          ev.preventDefault();
          row.classList.remove("drop-target");
          const from = dragFrom;
          dragFrom = null;
          if (from !== null) move(from, i);
        },
      },
      h("span", { class: "grip", title: "Drag to reorder" }, icon("grip")),
      h("span", { class: "order" }, i + 1),
      h(
        "label",
        { class: "toggle", title: e.enabled ? "Enabled" : "Disabled" },
        h("input", {
          type: "checkbox",
          checked: e.enabled,
          disabled: playing,
          "aria-label": `Enable ${m?.name || e.id}`,
          onchange: async (ev) => {
            e.enabled = ev.target.checked;
            renderMain();
            await saveProfile();
          },
        }),
        h("span", { class: "track" }),
      ),
      h(
        "div",
        { class: "mod-main" },
        h("div", { class: "mod-name" }, m ? m.name : e.id, m?.version && h("span", { class: "version" }, m.version)),
        h(
          "div",
          { class: "mod-meta" },
          m ? `${e.id} · ${m.files} file${m.files === 1 ? "" : "s"} · ${fmtBytes(m.size)}` : "Not in the store — remove it or import it again",
        ),
      ),
      h(
        "div",
        { class: "row-actions" },
        h("button", { class: "icon-btn", title: "Move up", "aria-label": "Move up", disabled: playing || i === 0, onclick: () => move(i, i - 1) }, icon("up")),
        h(
          "button",
          { class: "icon-btn", title: "Move down", "aria-label": "Move down", disabled: playing || i === entries.length - 1, onclick: () => move(i, i + 1) },
          icon("down"),
        ),
        h(
          "button",
          { class: "icon-btn", title: "Remove from this profile", "aria-label": "Remove from profile", disabled: playing, onclick: async () => {
            entries.splice(i, 1);
            renderMain();
            await saveProfile();
          } },
          "×",
        ),
      ),
    );
    return row;
  });

  return h(
    "section",
    { class: "panel" },
    h(
      "div",
      { class: "toolbar" },
      h("div", {}, h("h2", {}, `Load order · ${S.profileName}`), h("p", { class: "muted" }, "Mods lower in the list win when two ship the same file.")),
      h(
        "div",
        { class: "toolbar-actions" },
        h("button", { onclick: () => importFrom("archive"), disabled: playing || S.busy }, "Import .zip…"),
        h("button", { onclick: () => importFrom("folder"), disabled: playing || S.busy }, "Import folder…"),
      ),
    ),
    entries.length
      ? h("ol", { class: "mod-list" }, rows)
      : h(
          "div",
          { class: "empty drop-hint" },
          h("p", { class: "big" }, "No mods in this profile yet"),
          h("p", {}, "Import a .zip or a folder, or drop one onto this window."),
        ),
    notIn.length > 0 &&
      h(
        "div",
        { class: "store-section" },
        h("h3", {}, "In the store, not in this profile"),
        h(
          "ul",
          { class: "store-list" },
          notIn.map((m) =>
            h(
              "li",
              {},
              h("div", { class: "mod-main" }, h("div", { class: "mod-name" }, m.name), h("div", { class: "mod-meta" }, `${m.id} · ${m.files} files · ${fmtBytes(m.size)}`)),
              h("div", { class: "row-actions" },
                h("button", { class: "small", disabled: playing, onclick: async () => {
                  entries.push({ id: m.id, enabled: true });
                  renderMain();
                  await saveProfile();
                } }, "Add"),
                h("button", { class: "small ghost danger-text", disabled: playing, onclick: () => removeFromStore(m) }, icon("trash"), "Delete"),
              ),
            ),
          ),
        ),
      ),
    S.busy && h("div", { class: "busy-bar" }, h("span", { class: "spinner" }), S.busy),
  );
}

// ----- Files tab -----

function filesTab() {
  if (S.plan === "loading" || S.plan === null)
    return h("section", { class: "panel" }, h("div", { class: "loading" }, h("span", { class: "spinner" }), "Building the plan…"));
  const p = S.plan;
  const runtimeNote =
    S.details.game.inject === "doorstop"
      ? h("p", { class: "muted" }, "Files under BepInEx/ go to this profile's runtime folder, not the game folder.")
      : null;
  return h(
    "section",
    { class: "panel" },
    h(
      "div",
      { class: "stats" },
      stat(p.files.length, "files to deploy"),
      stat(p.conflicts.length, "conflicts", p.conflicts.length ? "warn" : ""),
      stat(p.skipped.length, "skipped"),
    ),
    runtimeNote,
    h("h3", {}, "Conflicts"),
    p.conflicts.length
      ? table(["File", "Used", "Overridden"], p.conflicts.map((c) => [h("code", {}, c.target), h("strong", {}, c.winner), c.overridden.join(", ")]))
      : h("p", { class: "muted" }, "No two enabled mods ship the same file."),
    p.skipped.length > 0 && [
      h("h3", {}, "Skipped files"),
      table(["Mod", "File", "Why"], p.skipped.map((s) => [s.owner, h("code", {}, s.file), s.reason])),
    ],
    h(
      "details",
      { class: "all-files" },
      h("summary", {}, `All ${p.files.length} files`),
      table(["Game path", "From", ""], p.files.map((f) => [h("code", {}, f.target), f.owner, f.writable ? h("span", { class: "chip" }, "copied") : ""])),
    ),
  );
}

function stat(n, label, kind = "") {
  return h("div", { class: `stat ${kind}` }, h("div", { class: "n" }, n), h("div", { class: "l" }, label));
}

function table(headers, rows) {
  return h(
    "div",
    { class: "table-wrap" },
    h("table", {}, h("thead", {}, h("tr", {}, headers.map((x) => h("th", {}, x)))), h("tbody", {}, rows.map((r) => h("tr", {}, r.map((c) => h("td", {}, c)))))),
  );
}

// ----- Health tab: the current or last session, then the doctor checks -----

function reportView(r) {
  if (!r) return null;
  const lines = [
    `Restored ${r.restored} original file${r.restored === 1 ? "" : "s"}, removed ${r.removed} mod file${r.removed === 1 ? "" : "s"}.`,
  ];
  return h(
    "div",
    { class: "report" },
    h("p", {}, lines),
    r.keptGameVersions?.length > 0 && h("p", {}, `Kept the game's newer version of: ${r.keptGameVersions.join(", ")}`),
    r.quarantined?.length > 0 && h("p", {}, `Set aside (changed during play): ${r.quarantined.join(", ")}`),
    r.dirsLeft?.length > 0 && h("p", { class: "muted" }, `Folders left because the game added files: ${r.dirsLeft.join(", ")}`),
  );
}

function sessionView() {
  const s = S.session;
  const mine = s.game === S.gameId;
  const active = mine && s.phase !== "idle";
  return [
    active &&
      h(
        "div",
        { class: "live-card" },
        h("div", { class: "pulse" }),
        h(
          "div",
          {},
          h("h2", {}, phaseLabel(s.phase)),
          h("p", {}, "UMM restores the game folder as soon as the game and every process it started have exited."),
        ),
        h("button", { class: "ghost", onclick: stopWaiting }, "Stop waiting"),
      ),
    !active && S.lastReport[S.gameId] && h("div", { class: "done-card" }, h("h2", {}, "Last session"), reportView(S.lastReport[S.gameId])),
  ];
}

// The session's step-by-step log, only once there is one for this game.
function activityView() {
  const s = S.session;
  if (s.game !== S.gameId || !s.log.length) return null;
  return [h("h3", {}, "Activity"), h("ol", { class: "log" }, s.log.map((l) => h("li", { class: l.kind }, h("time", {}, l.time), l.text)))];
}

function healthTab() {
  const findings =
    S.health === "loading" || S.health === null
      ? h("div", { class: "loading" }, h("span", { class: "spinner" }), "Checking…")
      : h("ul", { class: "findings" }, S.health.map((f) => h("li", { class: f.level }, h("span", { class: "badge" }, f.level), f.message)));
  return h(
    "section",
    { class: "panel" },
    sessionView(),
    h("div", { class: "toolbar" }, h("h2", {}, "Health check"), h("button", { onclick: loadHealth }, "Run again")),
    findings,
    activityView(),
  );
}

// ----- Actions tab -----

function actionsTab(d) {
  const g = d.game;
  const playing = S.session.phase !== "idle" && S.session.game === g.id;
  return h(
    "section",
    { class: "panel" },
    h(
      "div",
      { class: "actions-grid" },
      action("Restore the game folder", "Undo a deployment left in place (after a crash, or after you stopped waiting).", "Restore now", revertNow, !g.deployed || playing),
      action("Open UMM's data for this game", "Mods, profiles, backups and runtime folders.", "Open folder", () => attempt(() => api.openFolder(d.dataDir)), false),
      action(
        "Remove this game from UMM",
        "Permanently deletes its definition and every mod, profile and setting UMM stores for it. Uninstalling the game never does this.",
        "Remove…",
        deleteAllData,
        playing || g.deployed,
        true,
      ),
    ),
  );
}

function action(title, text, label, fn, disabled, danger = false) {
  return h(
    "div",
    { class: "action" },
    h("div", {}, h("strong", {}, title), h("p", { class: "muted" }, text)),
    h("button", { class: danger ? "danger" : "", disabled, onclick: fn }, label),
  );
}

function renderMain() {
  const d = S.details;
  if (!d) {
    $("#main").replaceChildren(
      h(
        "div",
        { class: "empty center" },
        h("img", { src: "icon.png", alt: "", class: "hero-icon" }),
        h("h1", {}, "Universal Mod Manager"),
        h("p", {}, S.games.length ? "Pick a game on the left." : "Add a game to get started."),
        !S.games.length && h("button", { class: "primary", onclick: addFromSteam }, "Add from Steam…"),
      ),
    );
    return;
  }
  const body =
    S.tab === "mods" ? modsTab(d) : S.tab === "files" ? filesTab() : S.tab === "health" ? healthTab() : actionsTab(d);
  $("#main").replaceChildren(header(d), tabs(), body);
}

// ---------- actions ----------

async function changeInstall() {
  const path = await attempt(() => api.pickPath("folder"));
  if (!path) return;
  await attempt(() => api.setInstallPath(S.gameId, path), "Game folder set");
  await refreshAll();
}

async function newProfile() {
  const name = await promptModal("New profile", "Name", "e.g. Vanilla+, Hardcore");
  if (!name) return;
  const copy = await modal({
    title: "Start from",
    body: h("p", {}, `Copy the mods and order of “${S.profileName}”, or start empty?`),
    actions: [
      { label: "Empty", value: "empty", class: "ghost" },
      { label: `Copy ${S.profileName}`, value: "copy", class: "primary", focus: true },
    ],
  });
  if (!copy) return;
  const p = await attempt(() => api.createProfile(S.gameId, name, copy === "copy" ? S.profileName : null), `Created ${name}`);
  if (p) {
    S.profileName = name;
    await loadGame();
  }
}

async function deleteProfile() {
  const ok = await confirmModal(
    `Delete “${S.profileName}”?`,
    "The profile and its runtime folder (configs the game saved for this profile) are deleted. Mods stay in the store.",
    "Delete profile",
    true,
  );
  if (!ok) return;
  await attempt(() => api.deleteProfile(S.gameId, S.profileName), "Profile deleted");
  S.profileName = "Default";
  await loadGame();
}

async function importFrom(kind) {
  const path = await attempt(() => api.pickPath(kind));
  if (path) await importPath(path);
}

async function importPath(path) {
  if (!S.gameId) return;
  S.busy = `Importing ${path.split(/[\\/]/).pop()}…`;
  renderMain();
  const m = await attempt(() => api.importMod(S.gameId, path, S.profileName));
  S.busy = false;
  if (m) toast(`Imported ${m.name} (${m.files} files) into ${S.profileName}`, "ok");
  await refreshAll();
}

async function removeFromStore(m) {
  const ok = await confirmModal(`Delete ${m.name}?`, "Removes the mod from the store and from every profile. This cannot be undone.", "Delete mod", true);
  if (!ok) return;
  await attempt(() => api.removeMod(S.gameId, m.id), `Deleted ${m.name}`);
  await refreshAll();
}

async function revertNow() {
  const r = await attempt(() => api.revert(S.gameId));
  if (r) {
    S.lastReport[S.gameId] = r;
    toast("Game folder restored", "ok");
  }
  await refreshAll();
}

async function deleteAllData() {
  const g = S.details.game;
  const input = h("input", { type: "text", "aria-label": "Game id", placeholder: g.id });
  const v = await modal({
    title: `Remove ${g.name} from UMM?`,
    body: [
      h("p", {}, `This removes the game from UMM and permanently deletes its definition, ${S.details.mods.length} mod(s), every profile and its settings. The game itself is not touched. You can add it again from Steam later.`),
      h("label", { class: "field" }, h("span", {}, `Type ${g.id} to confirm`), input),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Remove game", value: "go", class: "danger" },
    ],
  });
  if (v !== "go") return;
  if (input.value.trim() !== g.id) {
    toast("The id did not match; nothing was deleted.", "error");
    return;
  }
  const done = await attempt(async () => (await api.deleteGameData(g.id), true), `Removed ${g.name}`);
  if (!done) return;
  S.gameId = null;
  S.details = null;
  S.profileName = "Default";
  await refreshAll();
}

async function addFromSteam() {
  $("#modal-root").replaceChildren(
    h(
      "div",
      { class: "scrim" },
      h(
        "div",
        { class: "modal", role: "status" },
        h("div", { class: "loading" }, h("span", { class: "spinner" }), "Looking through your Steam libraries…"),
      ),
    ),
  );
  const list = await attempt(api.steamGames);
  $("#modal-root").replaceChildren();
  if (!list) return;
  if (!list.length) {
    await modal({
      title: "No Steam games found",
      body: h("p", {}, "UMM found no installed games in your Steam libraries."),
      actions: [{ label: "OK", value: true, class: "primary", focus: true }],
    });
    return;
  }

  const fresh = list.filter((g) => !g.definedAs);
  const checks = new Map();
  const filter = h("input", { type: "text", placeholder: "Filter by name", "aria-label": "Filter games" });
  const rows = list.map((g) => {
    const box = h("input", { type: "checkbox", disabled: !!g.definedAs, "aria-label": `Add ${g.name}` });
    if (!g.definedAs) checks.set(g.appId, box);
    return h(
      "label",
      { class: `steam-row ${g.definedAs ? "added" : ""}`, "data-name": g.name.toLowerCase() },
      box,
      h(
        "div",
        { class: "mod-main" },
        h("div", { class: "mod-name" }, g.name),
        h("div", { class: "mod-meta", title: g.path }, `${g.engine} · app ${g.appId}`),
      ),
      g.definedAs ? h("span", { class: "chip" }, "Added") : null,
    );
  });
  filter.addEventListener("input", () => {
    const q = filter.value.trim().toLowerCase();
    for (const r of rows) r.hidden = q && !r.dataset.name.includes(q);
  });

  const choice = await modal({
    title: "Add games from Steam",
    body: [
      h("p", { class: "muted" }, `${list.length} installed game${list.length === 1 ? "" : "s"}, ${fresh.length} not in UMM yet. UMM drafts a game definition for each one you pick; you can edit it later.`),
      list.length > 8 && filter,
      h("div", { class: "steam-list" }, rows),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Add selected", value: "add", class: "primary" },
    ],
  });
  if (choice !== "add") return;
  const ids = [...checks].filter(([, box]) => box.checked).map(([id]) => id);
  if (!ids.length) return toast("No games selected.");

  const results = await attempt(() => api.addSteamGames(ids));
  if (!results) return;
  const added = results.filter((r) => r.ok);
  for (const r of results.filter((r) => !r.ok)) toast(r.message, "error");
  if (added.length) {
    toast(`Added ${added.map((r) => r.message.replace(/^Added /, "")).join(", ")}`, "ok");
    S.gameId = added[0].id;
    S.profileName = "Default";
    await refreshAll();
    await showNotes(added);
  }
}

// Kinds of note the user asked never to see again ("bepinex", "unknown-engine", ...). A
// convenience of this window only, so it lives in the webview's storage, not in UMM's data.
const HIDDEN_NOTES = "umm.hiddenNotes";
function hiddenNotes() {
  try {
    return new Set(JSON.parse(localStorage.getItem(HIDDEN_NOTES) || "[]"));
  } catch {
    return new Set();
  }
}
function setHiddenNotes(keys) {
  try {
    if (keys.size) localStorage.setItem(HIDDEN_NOTES, JSON.stringify([...keys]));
    else localStorage.removeItem(HIDDEN_NOTES);
  } catch {}
}

async function showNotes(added) {
  const hidden = hiddenNotes();
  const many = added.length > 1;
  const seen = new Set();
  const items = [];
  for (const r of added) {
    for (const n of r.notes) {
      if (hidden.has(n.key)) continue;
      // The same kind of note for several games: once, naming them all.
      const dup = items.find((i) => i.key === n.key && i.text === n.text);
      if (dup) {
        dup.games.push(r.name);
        continue;
      }
      seen.add(n.key);
      items.push({ key: n.key, text: n.text, games: [r.name] });
    }
  }
  if (!items.length) return;
  const box = h("input", { type: "checkbox" });
  await modal({
    title: "Check these before adding mods",
    body: [
      h(
        "ul",
        { class: "notes" },
        items.map((i) => h("li", {}, many && h("strong", {}, `${i.games.join(", ")}: `), i.text)),
      ),
      h("label", { class: "check" }, box, items.length === 1 ? "Don't show this message again" : "Don't show these messages again"),
    ],
    actions: [{ label: "OK", value: true, class: "primary", focus: true }],
  });
  if (box.checked) {
    setHiddenNotes(new Set([...hidden, ...seen]));
    renderSidebar();
  }
}

function logLine(text, kind = "") {
  S.session.log.push({ time: fmtTime(), text, kind });
}

async function launch() {
  S.session = { game: S.gameId, phase: "deploying", log: [] };
  logLine(`Launching with profile ${S.profileName}`);
  S.tab = "health";
  renderSidebar();
  renderMain();
  if (!S.health) loadHealth();
  const ok = await attempt(() => api.launch(S.gameId, S.profileName));
  if (ok === undefined) {
    S.session.phase = "idle";
    renderMain();
    renderSidebar();
  }
}

async function stopWaiting() {
  const ok = await confirmModal(
    "Stop waiting for the game?",
    "The game keeps running and its mods stay in place. UMM restores the folder the next time it starts after the game has exited, or when you choose Restore now.",
    "Stop waiting",
  );
  if (ok) await attempt(api.stopWaiting);
}

function onSession(e) {
  const s = S.session;
  if (s.game !== e.game) S.session = { game: e.game, phase: "deploying", log: [] };
  switch (e.kind) {
    case "recovered":
      logLine("Restored the folder after an earlier, interrupted session");
      break;
    case "deployed":
      logLine(
        `Deployed ${e.gameFiles} file(s) to the game folder` +
          (e.runtime ? `; runtime folder: ${e.runtime.placed} placed, ${e.runtime.unchanged} unchanged, ${e.runtime.removed} removed` : ""),
      );
      for (const k of e.runtime?.keptEdited || []) logLine(`Kept your edited ${k}`);
      // The health check shown beside the session should describe the deployed state.
      if (S.tab === "health" && S.gameId === e.game) loadHealth();
      break;
    case "started":
      S.session.phase = "running";
      logLine(`Started the game (process ${e.pid})`);
      break;
    case "waitingForGame":
      S.session.phase = "waiting";
      logLine("Launched through Steam; waiting for the game to appear");
      break;
    case "gameDetected":
      S.session.phase = "running";
      logLine("The game is running");
      break;
    case "exited":
      S.session.phase = "reverting";
      logLine(e.code == null ? "The game exited" : `The game exited (code ${e.code})`);
      break;
    case "reverted":
      S.lastReport[e.game] = e.report;
      logLine(
        `Restored the game folder: ${plural(e.report.restored, "original file")} back, ${plural(e.report.removed, "mod file")} removed`,
        "ok",
      );
      break;
    case "finished":
      S.session.phase = "idle";
      if (e.abandoned) logLine("Stopped waiting. Mods stay deployed until the game exits.", "warn");
      else if (!e.appeared) logLine("The game never started; the folder was restored.", "warn");
      else logLine(`Session over after ${fmtDuration(e.seconds)}`, "ok");
      refreshAll();
      break;
    case "failed":
      S.session.phase = "idle";
      logLine(e.message, "error");
      toast(e.message, "error");
      refreshAll();
      break;
  }
  renderMain();
  renderSidebar();
}

// ---------- startup ----------

function banner(kind, content, actions = []) {
  const b = h("div", { class: `banner ${kind}` }, h("div", {}, content), h("div", { class: "banner-actions" }, actions, h("button", { class: "ghost small", onclick: () => b.remove() }, "Dismiss")));
  $("#banners").append(b);
}

function showStartup(st) {
  const restored = st.recovery.filter((r) => r.ok && r.report);
  const failed = st.recovery.filter((r) => !r.ok);
  if (restored.length)
    banner("ok", `Restored the game folder for ${restored.map((r) => r.game).join(", ")} after an interrupted session.`);
  for (const f of failed)
    banner("warn", `${f.game}: ${f.message}`, [
      h("button", { class: "small", onclick: async (e) => {
        const res = await attempt(api.recoverAll);
        if (res?.every((r) => r.ok)) {
          e.target.closest(".banner").remove();
          toast("Recovered", "ok");
        }
        refreshAll();
      } }, "Try again"),
    ]);
  for (const err of st.defErrors) banner("warn", `Game definition not loaded: ${err}`);
}

function setupDrop() {
  const overlay = $("#drop-overlay");
  api.onDrop?.((p) => {
    if (p.type === "enter" || p.type === "over") overlay.hidden = !S.gameId;
    else if (p.type === "leave") overlay.hidden = true;
    else if (p.type === "drop") {
      overlay.hidden = true;
      if (S.session.phase !== "idle") return toast("Wait for the game session to end before importing.", "error");
      for (const path of p.paths || []) importPath(path);
    }
  });
}

async function main() {
  api.listen("session", onSession);
  api.listen("close-blocked", async () => {
    const ok = await confirmModal(
      "A game is still running",
      "If you close UMM now, the game's mods stay in place until UMM next starts after the game has exited.",
      "Close anyway",
      true,
    );
    if (ok) api.quitAnyway();
  });
  setupDrop();
  renderSidebar();
  renderMain();
  S.startup = await attempt(api.startup);
  if (S.startup) showStartup(S.startup);
  await refreshAll();
}

main();

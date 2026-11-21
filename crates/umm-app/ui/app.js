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
  external: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M9 2h5v5h-1.5V4.6L7.6 9.5 6.5 8.4l4.9-4.9H9zM3 4h4v1.5H4.5v6h6V9H12v4H3z"/></svg>',
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
  // Thunderstore tab: the query, the page shown (null while loading), and an install/update
  // in progress ({ message }).
  ts: { game: null, query: { text: "", category: "", sort: "rating", installedOnly: false, offset: 0, limit: 40 }, page: null, busy: null },
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
            // An explicit `value: null` (Cancel, Close) must resolve to null, not the label.
            { class: a.class || "", onclick: () => close("value" in a ? a.value : a.label), autofocus: a.focus || false },
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
  if (S.loader && S.loader.game !== S.gameId) S.loader = null;
  renderMain();
  if (S.tab === "files") loadPlan();
  if (S.tab === "health") loadHealth();
  if (S.tab === "actions") loadLoaderChoice();
  if (S.tab === "thunderstore") {
    if (d.game.thunderstore) loadTs(S.ts.game !== S.gameId);
    else {
      S.tab = "mods";
      renderMain();
    }
  }
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
  // Mods may have changed (deleted, imported, installed), so the Thunderstore tab's
  // "installed" and "update" marks are out of date until it loads again.
  S.ts.stale = true;
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
        h(
          "p",
          { class: "fine" },
          "For a game not on Steam, ",
          h("button", { class: "link inline-link", onclick: addManually }, "add it manually"),
          " by picking its folder.",
        ),
      );

  $("#sidebar").replaceChildren(
    h("div", { class: "section-label" }, "Games"),
    list,
    S.games.length > 0
      ? h(
          "div",
          { class: "add-games" },
          h("button", { class: "ghost small", onclick: addFromSteam }, "+ Add from Steam…"),
          h("button", { class: "ghost small", onclick: addManually }, "+ Add manually…"),
        )
      : "",
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
      h("button", { class: "link", onclick: linkSettings }, "Thunderstore website button…"),
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
          h(
            "button",
            { class: "chip chip-button", title: "Change the mod loader", onclick: () => { S.tab = "actions"; renderMain(); loadLoaderChoice(); } },
            g.setup,
          ),
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
    ...(S.details?.game.thunderstore ? [["thunderstore", "Thunderstore"]] : []),
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
            if (id === "actions") loadLoaderChoice();
            if (id === "thunderstore" && (!S.ts.page || S.ts.game !== S.gameId || S.ts.stale)) loadTs(true);
          },
        },
        label,
        id === "health" && S.session.game === S.gameId && S.session.phase !== "idle" && h("span", { class: "dot live" }),
      ),
    ),
  );
}

// ----- Mods tab -----


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
    // Reordering uses pointer events, not HTML drag and drop: the app's native file drop
    // (drop an archive on the window to import it) turns HTML drag and drop off in WebView2.
    const grip = h("span", { class: `grip ${playing ? "off" : ""}`, title: playing ? "" : "Drag to reorder" }, icon("grip"));
    if (!playing) {
      grip.addEventListener("pointerdown", (ev) => {
        if (ev.button !== 0) return;
        ev.preventDefault();
        grip.setPointerCapture(ev.pointerId);
        const list = row.parentElement;
        row.classList.add("dragging");
        let target = i;
        // The position the row would end up at: before the first row whose middle is below
        // the pointer, counted without the row being dragged.
        const over = (y) => {
          const rows = [...list.children];
          const before = rows.findIndex((r) => {
            const b = r.getBoundingClientRect();
            return y < b.top + b.height / 2;
          });
          if (before === -1) return rows.length - 1;
          return before > i ? before - 1 : before;
        };
        const mark = () => {
          // Moving up, the row lands above the target row; moving down, below it.
          for (const [k, r] of [...list.children].entries()) {
            r.classList.toggle("drop-target", k === target && target < i);
            r.classList.toggle("drop-after", k === target && target > i);
          }
        };
        const onMove = (mv) => {
          target = over(mv.clientY);
          mark();
        };
        const onUp = () => {
          grip.removeEventListener("pointermove", onMove);
          grip.removeEventListener("pointerup", onUp);
          grip.removeEventListener("pointercancel", onUp);
          row.classList.remove("dragging");
          for (const r of list.children) r.classList.remove("drop-target", "drop-after");
          if (target !== i) move(i, target);
        };
        grip.addEventListener("pointermove", onMove);
        grip.addEventListener("pointerup", onUp);
        grip.addEventListener("pointercancel", onUp);
      });
    }
    const row = h(
      "li",
      { class: `mod-row ${e.enabled ? "" : "disabled"} ${m ? "" : "missing"}` },
      grip,
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
          m ? `${m.thunderstore ? "Thunderstore · " : ""}${e.id} · ${m.files} file${m.files === 1 ? "" : "s"} · ${fmtBytes(m.size)}` : "No longer installed — remove it from the profile or import it again",
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
        h("h3", {}, "Installed for this game, not in this profile"),
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
  const runtimeDirs = {
    bepinex: "BepInEx/",
    shimloader: "shimloader/",
    melonloader: "MelonLoader/, Mods/, Plugins/, UserData/ and UserLibs/",
    "recursive-melonloader": "MelonLoader/, Mods/, Plugins/, UserData/ and UserLibs/",
  }[S.details.game.loader];
  const runtimeNote =
    runtimeDirs && ["doorstop", "args"].includes(S.details.game.inject)
      ? h("p", { class: "muted" }, `Files under ${runtimeDirs} go to this profile's runtime folder, not the game folder.`)
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
      loaderAction(g, playing),
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

// ----- Thunderstore tab -----

let tsSearchTimer = null;

/// Loads the Thunderstore list from the top. `reset` also shows the loading state when the
/// game changed; `append` (Show more) adds the page at the current offset instead.
async function loadTs(reset, append = false) {
  const game = S.gameId;
  if (!append) S.ts.query.offset = 0;
  if (reset) {
    // A new search keeps showing the last results until the new ones arrive.
    if (S.ts.game !== game) S.ts.page = null;
    S.ts.game = game;
    renderMain();
  }
  const page = await attempt(() => api.tsSearch(game, S.ts.query));
  if (S.gameId !== game) return;
  if (!page) {
    S.ts.page = { error: true, items: [], total: 0, categories: [], updates: [] };
  } else if (append && S.ts.page && !S.ts.page.error) {
    S.ts.page = { ...page, items: [...S.ts.page.items, ...page.items] };
  } else {
    S.ts.page = page;
    S.ts.stale = false;
  }
  if (S.tab === "thunderstore") renderMain();
}

function fmtCount(n) {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`;
  return String(n);
}

async function tsRun(title, call) {
  S.ts.busy = { message: title };
  renderMain();
  const r = await attempt(call);
  S.ts.busy = null;
  if (r) {
    const names = r.installed.map((i) => (i.replaced ? `${i.fullName} ${i.replaced} → ${i.version}` : `${i.fullName} ${i.version}`));
    toast(names.length ? `Installed ${names.join(", ")}` : "Everything needed was already installed", "ok");
    for (const m of r.missing) toast(`Dependency ${m} is no longer on Thunderstore; skipped`, "error");
    for (const w of r.warnings || []) toast(w, "error");
    for (const n of r.notes || []) toast(n, "info");
  }
  await refreshAll();
  await loadTs(false);
}

const REVIEW_TEXT = {
  rejected: "This game's Thunderstore moderators rejected it for this game.",
  unreviewed: "It is waiting for this game's Thunderstore moderators to review it.",
};

// Installs a package by Owner-Name. One the game's list does not show (not submitted for this
// game, unreviewed, or rejected by its moderators) is installed only after saying so.
async function installByName(prefill) {
  const game = S.gameId;
  const field = h("input", { type: "text", placeholder: "Owner-Name or Owner-Name@1.2.3", "aria-label": "Package", value: prefill || "" });
  const v = await modal({
    title: "Install an unlisted package",
    body: [
      h("p", { class: "muted" }, "A Thunderstore package this game's list does not show, by the name on its Thunderstore page (Owner-Name). Its dependencies are installed too."),
      field,
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Look up", value: "go", class: "primary", focus: false },
    ],
  });
  const text = field.value.trim();
  if (v !== "go" || !text) return;
  const [name, version] = text.split("@");
  const found = await attempt(() => api.tsLookup(game, name));
  if (!found) return;
  if (found.listed) return tsInstall([text]);
  const p = found.package;
  const why = REVIEW_TEXT[found.reviewStatus] || "It is not listed for this game.";
  const ok = await modal({
    title: `${p.name.replaceAll("_", " ")} is not listed for this game`,
    body: [
      h("p", {}, `${p.fullName} ${version || p.latest}: ${p.description}`),
      h("p", { class: "banner warn" }, `${why} UMM installs it only because you ask, and it will not offer updates for it.`),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Install anyway", value: "go", class: "danger" },
    ],
  });
  if (ok !== "go") return;
  await tsRun("Installing…", () => api.tsInstallUnlisted(game, p.fullName, version, S.profileName));
}

const tsInstall = (pkgs) => tsRun("Installing…", () => api.tsInstall(S.gameId, pkgs, S.profileName));
const tsUpdate = (names) => tsRun("Updating…", () => api.tsUpdate(S.gameId, names, S.profileName));

function tsTab(d) {
  const g = d.game;
  const playing = S.session.phase !== "idle" && S.session.game === g.id;
  const locked = playing || g.deployed || !!S.ts.busy;
  const q = S.ts.query;
  const p = S.ts.page;

  const search = h("input", { type: "search", placeholder: "Search packages", "aria-label": "Search packages", value: q.text });
  search.addEventListener("input", () => {
    q.text = search.value;
    clearTimeout(tsSearchTimer);
    tsSearchTimer = setTimeout(async () => {
      await loadTs(true);
      const box = $(".ts-toolbar input[type=search]");
      if (box) {
        box.focus();
        box.setSelectionRange(box.value.length, box.value.length);
      }
    }, 250);
  });
  const category = h(
    "select",
    { "aria-label": "Category" },
    h("option", { value: "" }, "All categories"),
    (p?.categories || []).map((c) => h("option", { value: c, selected: c === q.category }, c)),
  );
  category.addEventListener("change", () => {
    q.category = category.value;
    loadTs(true);
  });
  const sort = h(
    "select",
    { "aria-label": "Sort" },
    [
      ["rating", "Top rated"],
      ["downloads", "Most downloaded"],
      ["updated", "Recently updated"],
      ["name", "Name"],
    ].map(([v, l]) => h("option", { value: v, selected: v === q.sort }, l)),
  );
  sort.addEventListener("change", () => {
    q.sort = sort.value;
    loadTs(true);
  });
  const installedOnly = h("input", { type: "checkbox", checked: q.installedOnly });
  installedOnly.addEventListener("change", () => {
    q.installedOnly = installedOnly.checked;
    loadTs(true);
  });
  const refresh = async () => {
    const fresh = await attempt(() => api.tsRefresh(g.id));
    if (fresh === false) toast("Thunderstore could not be reached; showing the last list downloaded", "error");
    loadTs(true);
  };

  const banners = [];
  if (S.ts.busy) banners.push(h("div", { class: "ts-banner live" }, h("span", { class: "spinner" }), S.ts.busy.message));
  if (g.deployed || playing) banners.push(h("div", { class: "ts-banner warn" }, "Mods are deployed; installing and updating wait until the game folder is restored."));
  if (p && !p.error && !p.loaderInstalled && p.loaderPackage)
    banners.push(
      h(
        "div",
        { class: "ts-banner" },
        h("span", {}, `${p.loaderName} is not installed yet. It comes with the first mod you install, or install it now.`),
        h("button", { class: "primary small", disabled: locked, onclick: () => tsInstall([p.loaderPackage]) }, `Install ${p.loaderName}`),
      ),
    );
  if (p?.updates?.length)
    banners.push(
      h(
        "div",
        { class: "ts-banner" },
        h("span", {}, `${p.updates.length} update${p.updates.length === 1 ? "" : "s"} available: ${p.updates.map((u) => u[0].split("-").slice(1).join("-")).join(", ")}`),
        h("button", { class: "small", disabled: locked, onclick: () => tsUpdate([]) }, "Update all"),
      ),
    );
  if (p?.stale) banners.push(h("div", { class: "ts-banner warn" }, "Thunderstore could not be reached; this is the last list downloaded."));
  if (p?.loaderWarning)
    banners.push(
      h(
        "div",
        { class: "ts-banner warn" },
        h("span", {}, p.loaderWarning),
        h("button", { class: "small", disabled: locked, onclick: () => installByName(p.loaderSuggestion || "") }, "Install unlisted package…"),
      ),
    );

  let list;
  if (!p) list = h("div", { class: "loading" }, h("span", { class: "spinner" }), "Loading packages… the first time can take a while for big communities.");
  else if (p.error) list = h("p", { class: "muted" }, "The package list could not be loaded.", " ", h("button", { class: "link", onclick: () => loadTs(true) }, "Try again"));
  else if (!p.items.length) list = h("p", { class: "muted" }, "No packages match.");
  else
    list = h(
      "div",
      { class: "ts-list" },
      p.items.map((it) => {
        const action = it.installed
          ? it.update
            ? h("button", { class: "small", disabled: locked, onclick: () => tsUpdate([it.fullName]) }, `Update to ${it.latest}`)
            : h("span", { class: "chip ok" }, `Installed ${it.installed}`)
          : h("button", { class: "primary small", disabled: locked, onclick: () => tsInstall([it.fullName]) }, "Install");
        return h(
          "div",
          { class: `ts-row ${it.deprecated ? "deprecated" : ""}` },
          it.icon ? h("img", { class: "ts-icon", src: it.icon, alt: "", loading: "lazy" }) : h("div", { class: "ts-icon" }),
          h(
            "div",
            { class: "ts-main" },
            h(
              "div",
              { class: "ts-title" },
              h(
                "button",
                {
                  class: "link ts-name",
                  title: "Open its page on thunderstore.io",
                  onclick: () => attempt(() => api.openPackagePage(g.id, it.fullName)),
                },
                it.name.replaceAll("_", " "),
                icon("external"),
              ),
              h("span", { class: "muted" }, ` by ${it.owner}`),
              it.latest && h("span", { class: "muted" }, ` · ${it.latest}`),
              it.deprecated && h("span", { class: "chip warn" }, "Deprecated"),
            ),
            h("div", { class: "ts-desc" }, it.description),
            h(
              "div",
              { class: "ts-meta muted" },
              `${fmtCount(it.downloads)} downloads · ${it.rating} likes`,
              it.size ? ` · ${fmtBytes(it.size)}` : "",
              it.categories.length ? ` · ${it.categories.join(", ")}` : "",
            ),
          ),
          action,
        );
      }),
    );

  return h(
    "section",
    { class: "panel" },
    h(
      "div",
      { class: "ts-share" },
      h("span", { class: "muted small" }, "r2modman profiles:"),
      h("button", { class: "link", disabled: locked, onclick: shareProfile }, `Share “${S.profileName}”…`),
      h("button", { class: "link", disabled: locked, onclick: importProfile }, "Import a profile…"),
    ),
    h(
      "div",
      { class: "ts-toolbar" },
      search,
      category,
      sort,
      h("label", { class: "check inline" }, installedOnly, "Installed"),
      h("button", { class: "ghost small", onclick: refresh, title: "Download the package list again" }, "Refresh"),
      h(
        "button",
        { class: "ghost small", disabled: locked, onclick: () => installByName(""), title: "Install a Thunderstore package this game's list does not show" },
        "Install unlisted package…",
      ),
    ),
    banners,
    p && !p.error && h("p", { class: "muted small" }, `${p.total} package${p.total === 1 ? "" : "s"} · installs into the profile “${S.profileName}” with dependencies`),
    list,
    p && !p.error && p.items.length < p.total &&
      h(
        "div",
        { class: "ts-more" },
        h(
          "button",
          {
            class: "ghost",
            onclick: () => {
              q.offset = p.items.length;
              loadTs(false, true);
            },
          },
          "Show more",
        ),
      ),
  );
}

// ----- r2modman profiles: share and import -----

async function shareProfile() {
  const game = S.gameId;
  const profile = S.profileName;
  const e = await attempt(() => api.exportPreview(game, profile));
  if (!e) return;
  const body = [
    h("p", {}, `${e.mods} mod${e.mods === 1 ? "" : "s"} and ${e.files.length} config file${e.files.length === 1 ? "" : "s"} (${fmtBytes(e.size)}).`),
    e.leftOut.length > 0 && h("p", { class: "muted" }, `Left out, because they are not from Thunderstore: ${e.leftOut.join(", ")}.`),
    e.files.length === 0 && h("p", { class: "muted" }, "No config files yet: the mod loader and the mods write them the first time the game runs with this profile."),
    h("p", { class: "muted" }, "Save as file writes an .r2z file on this computer. Nothing is uploaded."),
    h("p", { class: "muted" }, "Get a code uploads the profile, configs included, to Thunderstore. Anyone with the code can download it, and Thunderstore may delete it after a while."),
  ];
  const choice = await modal({
    title: `Share the profile “${profile}”`,
    body,
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Save as file…", value: "file", class: "" },
      { label: "Get a code", value: "code", class: "primary" },
    ],
  });
  if (choice === "file") {
    const path = await attempt(() => api.exportFile(game, profile));
    if (path) toast(`Saved ${path}`, "ok");
  } else if (choice === "code") {
    const code = await attempt(() => api.exportCode(game, profile));
    if (!code) return;
    const field = h("input", { type: "text", value: code, readonly: true, "aria-label": "Profile code" });
    const copy = await modal({
      title: "Profile code",
      body: [h("p", {}, "Anyone can import this profile in UMM or r2modman with this code."), field],
      actions: [
        { label: "Close", value: null, class: "ghost" },
        { label: "Copy", value: "copy", class: "primary", focus: true },
      ],
    });
    if (copy === "copy") {
      try {
        await navigator.clipboard.writeText(code);
        toast("Code copied", "ok");
      } catch {
        field.select();
        toast("Select the code and copy it", "info");
      }
    }
  }
}

function importSummary(r) {
  const n = r.installed.installed.length;
  toast(`Imported into the new profile “${r.profile}”: ${n} package${n === 1 ? "" : "s"} downloaded, ${r.files} config file${r.files === 1 ? "" : "s"}`, "ok");
  for (const m of r.notes) toast(m, "info");
  for (const x of r.installed.missing) toast(`${x} is no longer on Thunderstore; skipped`, "error");
}

async function importProfile() {
  const game = S.gameId;
  const field = h("input", { type: "text", placeholder: "Profile code", "aria-label": "Profile code" });
  const choice = await modal({
    title: "Import an r2modman profile",
    body: [h("p", {}, "Paste a profile code, or pick an .r2z file. The mods are installed into a new profile."), field],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "From a file…", value: "file", class: "" },
      { label: "Import code", value: "code", class: "primary" },
    ],
  });
  if (!choice) return;
  S.ts.busy = { message: "Importing…" };
  renderMain();
  const r = choice === "code" ? await attempt(() => api.importCode(game, field.value.trim())) : await attempt(() => api.importFile(game));
  S.ts.busy = null;
  if (r) {
    importSummary(r);
    S.profileName = r.profile;
  }
  await refreshAll();
  if (S.tab === "thunderstore") await loadTs(false);
}

// ----- Thunderstore's "Install with Mod Manager" links -----

/// The program in a link handler's command (`"C:\…\r2modman.exe" "%1"`): its path, without
/// the arguments.
function programOf(command) {
  const m = command.match(/^\s*"([^"]+)"/) || command.match(/^\s*(\S+)/);
  return m ? m[1] : command;
}

const LINK_TITLE = "Install from the Thunderstore website";

async function linkSettings() {
  const st = await attempt(api.linkHandler);
  if (!st) return;
  if (!st.supported) {
    await modal({ title: LINK_TITLE, body: h("p", {}, "This is not available on this system."), actions: [{ label: "OK", value: true, class: "primary", focus: true }] });
    return;
  }
  let status;
  if (st.ours) status = h("p", { class: "muted" }, "The button currently opens UMM.");
  else if (st.other) {
    status = [
      h("p", { class: "muted" }, "The button currently opens:"),
      h("p", { class: "muted small path-text" }, programOf(st.other)),
    ];
  } else status = [];
  const v = await modal({
    title: LINK_TITLE,
    body: [h("p", {}, "Mod pages on thunderstore.io have an “Install with Mod Manager” button. Turn this on to make that button open UMM, which then asks which of your games to install the mod into."), ...[status].flat()],
    actions: [
      { label: "Close", value: null, class: "ghost" },
      st.ours ? { label: "Turn off", value: "off", class: "" } : { label: "Turn on", value: "on", class: "primary" },
    ],
  });
  if (v) {
    const r = await attempt(() => api.setLinkHandler(v === "on"), v === "on" ? "The Thunderstore install button now opens UMM" : "The Thunderstore install button no longer opens UMM");
    if (r) renderSidebar();
  }
}

async function handleInstallLink(url) {
  const l = await attempt(() => api.resolveLink(url));
  if (!l) return;
  const what = `${l.name.replaceAll("_", " ")}${l.version ? ` ${l.version}` : ""} by ${l.owner}`;
  if (!l.games.length) {
    await modal({
      title: `Install ${what}?`,
      body: h("p", {}, "None of your games that get mods from Thunderstore has this package. Add the game from Steam, or set its mod loader to the one from Thunderstore in the Actions tab."),
      actions: [{ label: "OK", value: true, class: "primary", focus: true }],
    });
    return;
  }
  const pick = h("select", { "aria-label": "Game" }, l.games.map((g) => h("option", { value: g.id, selected: g.id === S.gameId }, g.name)));
  const ok = await modal({
    title: `Install ${what}?`,
    body: [h("p", {}, "From a Thunderstore link. Its dependencies are installed too, into the current profile of the game you pick."), h("label", { class: "field" }, h("span", {}, "Game"), pick)],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Install", value: "go", class: "primary", focus: true },
    ],
  });
  if (ok !== "go") return;
  if (S.gameId !== pick.value) {
    S.gameId = pick.value;
    S.profileName = "Default";
  }
  S.tab = "thunderstore";
  await refreshAll();
  await tsInstall([l.version ? `${l.fullName}@${l.version}` : l.fullName]);
}

async function loadLoaderChoice() {
  const game = S.gameId;
  const c = await attempt(() => api.loaderChoice(game));
  if (c && S.gameId === game) {
    S.loader = { ...c, game };
    if (S.tab === "actions") renderMain();
  }
}

const SETUP_HELP = {
  thunderstore: "Mods and the mod loader come from Thunderstore, laid out the way r2modman does.",
  bepinex: "BepInEx 5 or 6 through Doorstop, with a pack you import yourself as the mod “bepinex”.",
  generic: "No mod loader: mod files are copied into the game folder as they are.",
};

function loaderAction(g, playing) {
  const c = S.loader;
  if (!c) return h("div", { class: "action" }, h("div", { class: "loading" }, h("span", { class: "spinner" }), "Mod loader…"));
  const select = h(
    "select",
    { "aria-label": "Mod loader" },
    c.available.map((o) => h("option", { value: o.value, selected: o.value === c.current }, o.label)),
    c.current === null && h("option", { value: "", selected: true }, c.currentLabel),
  );
  const notes = [
    c.community && `Thunderstore community: ${c.community}.`,
    c.otherLoader && `On Thunderstore this game uses ${c.otherLoader}, which UMM does not support yet.`,
  ].filter(Boolean);
  const apply = async () => {
    const value = select.value;
    if (!value || value === c.current) return;
    const ok = await confirmModal(
      "Change the mod loader?",
      "UMM rewrites the loader and install rule sections of this game's game.toml. Install rules you wrote there by hand are replaced; the rest of the file is kept.",
      "Change",
    );
    if (!ok) return;
    const r = await attempt(() => api.setLoader(g.id, value), "Mod loader changed");
    if (r) {
      S.loader = { ...r, game: g.id };
      S.plan = null;
      await refreshAll();
    }
  };
  return h(
    "div",
    { class: "action loader-action" },
    h(
      "div",
      {},
      h("strong", {}, "Mod loader"),
      h("p", { class: "muted" }, SETUP_HELP[c.current] || "Set by hand in this game's game.toml."),
      notes.length > 0 && h("p", { class: "muted small" }, notes.join(" ")),
      g.deployed && h("p", { class: "muted small" }, "Restore the game folder before changing it."),
    ),
    h("div", { class: "row" }, select, h("button", { disabled: playing || g.deployed, onclick: apply }, "Apply")),
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
    S.tab === "mods"
      ? modsTab(d)
      : S.tab === "thunderstore"
        ? tsTab(d)
        : S.tab === "files"
          ? filesTab()
          : S.tab === "health"
            ? healthTab()
            : actionsTab(d);
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
    "The profile and its runtime folder (configs the game saved for this profile) are deleted. Its mods stay installed for this game.",
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
  const dependents = m.thunderstore ? (await attempt(() => api.tsDependents(S.gameId, m.id))) || [] : [];
  const needed = dependents.length ? ` ${dependents.join(", ")} ${dependents.length === 1 ? "needs" : "need"} it and may stop working.` : "";
  const ok = await confirmModal(`Delete ${m.name}?`, `Removes the mod from UMM and from every profile. This cannot be undone.${needed}`, "Delete mod", true);
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

// ----- Add game manually: pick a folder, confirm what UMM found -----

async function addManually() {
  const info = await attempt(api.pickGameFolder);
  if (!info) return;
  const name = h("input", { type: "text", value: info.name, "aria-label": "Name" });
  const exe = h(
    "select",
    { "aria-label": "Exe to launch" },
    info.exes.map((e, i) => h("option", { value: e, selected: i === 0 }, e)),
    h("option", { value: "" }, "None (UMM only deploys mods)"),
  );
  const setup = h(
    "select",
    { "aria-label": "Mod loader", disabled: info.setups.length < 2 },
    info.setups.map((o) => h("option", { value: o.value }, o.label)),
  );
  const found = [
    info.engine,
    info.community ? "on Thunderstore" : info.otherLoader ? `${info.otherLoader} on Thunderstore` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const v = await modal({
    title: "Add game manually",
    body: [
      h("p", { class: "muted small path-text" }, info.folder),
      h("p", { class: "muted small" }, `Found: ${found}`),
      info.steamAppId &&
        h("p", { class: "banner warn" }, `This is ${info.steamName} from Steam. Add from Steam sets it up with its Steam details, which this form leaves out.`),
      h("label", { class: "field" }, h("span", {}, "Name"), name),
      h("label", { class: "field" }, h("span", {}, "Exe to launch"), exe),
      h("label", { class: "field" }, h("span", {}, "Mod loader"), setup),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Add game", value: "add", class: "primary", focus: true },
    ],
  });
  if (v !== "add") return;
  const r = await attempt(() => api.addManualGame(info.folder, name.value.trim(), exe.value, setup.value || null));
  if (!r) return;
  toast(r.message, "ok");
  S.gameId = r.id;
  S.profileName = "Default";
  await refreshAll();
  await showNotes([r]);
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
  const res = await attempt(api.steamGames);
  $("#modal-root").replaceChildren();
  if (!res) return;
  const list = res.games;
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
    const setup = g.definedAs
      ? null
      : h(
          "select",
          { class: "setup", "aria-label": `Mod loader for ${g.name}`, title: "Mod loader", disabled: g.setups.length < 2 },
          g.setups.map((o) => h("option", { value: o.value }, o.label)),
        );
    // Changing the loader means the game is wanted.
    setup?.addEventListener("change", () => (box.checked = true));
    if (!g.definedAs) checks.set(g.appId, { box, setup });
    const meta = [g.engine, g.community ? "on Thunderstore" : g.otherLoader ? `${g.otherLoader} on Thunderstore` : null, `app ${g.appId}`]
      .filter(Boolean)
      .join(" · ");
    return h(
      "div",
      { class: `steam-row ${g.definedAs ? "added" : ""}`, "data-name": g.name.toLowerCase() },
      h(
        "label",
        { class: "steam-pick" },
        box,
        h("div", { class: "mod-main" }, h("div", { class: "mod-name" }, g.name), h("div", { class: "mod-meta", title: `${meta}\n${g.path}` }, meta)),
      ),
      g.definedAs ? h("span", { class: "chip" }, "Added") : setup,
    );
  });
  filter.addEventListener("input", () => {
    const q = filter.value.trim().toLowerCase();
    for (const r of rows) r.hidden = q && !r.dataset.name.includes(q);
  });

  const choice = await modal({
    title: "Add games from Steam",
    body: [
      h("p", { class: "muted" }, `${list.length} installed game${list.length === 1 ? "" : "s"}, ${fresh.length} not in UMM yet. UMM drafts a game definition for each one you pick, with the mod loader shown; you can change it later.`),
      res.thunderstoreError && h("p", { class: "banner warn" }, `Thunderstore's game list could not be loaded, so every game is offered as generic. ${res.thunderstoreError}`),
      list.length > 8 && filter,
      h("div", { class: "steam-list" }, rows),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Add selected", value: "add", class: "primary" },
    ],
  });
  if (choice !== "add") return;
  const picks = [...checks].filter(([, c]) => c.box.checked).map(([appId, c]) => ({ appId, setup: c.setup.value }));
  if (!picks.length) return toast("No games selected.");

  const results = await attempt(() => api.addSteamGames(picks));
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
  // One note reads as a sentence; several (more games, or more to check) as a list.
  const single = items.length === 1;
  await modal({
    title: single ? `Added ${items[0].games.join(", ")}` : "Check these before adding mods",
    body: [
      single
        ? h("p", {}, items[0].text)
        : h(
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
  // Text cut short with "…" shows in full on hover, wherever it is.
  document.addEventListener("mouseover", (e) => {
    const el = e.target instanceof Element ? e.target.closest(".mod-meta, .mod-name, .ts-desc, .path, .game-name") : null;
    if (el && !el.title && el.scrollWidth > el.clientWidth) el.title = el.textContent;
  });
  api.listen("install-link", (url) => handleInstallLink(url));
  api.listen("ts-progress", (e) => {
    if (S.ts.busy && e.game === S.gameId) {
      S.ts.busy.message = e.message;
      const b = $(".ts-banner.live");
      if (b) b.lastChild.textContent = e.message;
    }
  });
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
  // Started from a Thunderstore install link.
  const link = await attempt(api.takePendingLink);
  if (link) await handleInstallLink(link);
}

main();

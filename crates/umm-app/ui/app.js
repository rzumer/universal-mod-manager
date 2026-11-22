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
  // Adjusting where a mod's files go.
  sliders: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 3.3h12v1.4H2zM2 7.3h12v1.4H2zM2 11.3h12v1.4H2z"/><rect x="9" y="1.5" width="2.6" height="5" rx=".8"/><rect x="3.8" y="5.5" width="2.6" height="5" rx=".8"/><rect x="8" y="9.5" width="2.6" height="5" rx=".8"/></svg>',};

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
  // Nexus Mods tab: the game's Nexus game, the query and page shown, installed mods' updates,
  // and an install in progress ({ message }).
  nexus: { game: null, picker: null, query: { text: "", sort: "endorsements", offset: 0, count: 40, installed: false }, page: null, updates: null, busy: null },
};

/// Shows a message for long enough to read it: longer for longer text and for errors, longer
/// again when others are showing, and for as long as the pointer is over it.
function toast(message, kind = "info") {
  const t = h("div", { class: `toast ${kind}`, role: kind === "error" ? "alert" : "status" }, message);
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
  if (S.tab === "actions") loadActions();
  if (S.tab === "nexus" && S.nexus.game?.game !== S.gameId) loadNexus(true);
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
  // Mods may have changed (deleted, imported, installed), so the Thunderstore and Nexus
  // tabs' "installed" and "update" marks are out of date until they load again.
  S.ts.stale = true;
  S.nexus.stale = true;
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
      h("button", { class: "link", onclick: linkSettings }, "Install buttons on mod sites…"),
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
            { class: "chip chip-button", title: "Change the mod loader", onclick: () => { S.tab = "actions"; renderMain(); loadActions(); } },
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
    ["nexus", "Nexus Mods"],
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
            if (id === "actions") loadActions();
            if (id === "thunderstore" && (!S.ts.page || S.ts.game !== S.gameId || S.ts.stale)) loadTs(true);
            if (id === "nexus" && (!S.nexus.page || S.nexus.game?.game !== S.gameId || S.nexus.stale)) loadNexus(true);
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
    // A mod whose installer's options are not chosen yet installs nothing: shown off, and
    // locked off, until they are.
    const waiting = m?.installer === "waiting";
    const on = e.enabled && !waiting;
    const row = h(
      "li",
      { class: `mod-row ${on ? "" : "disabled"} ${m ? "" : "missing"}` },
      grip,
      h("span", { class: "order" }, i + 1),
      h(
        "label",
        { class: "toggle", title: waiting ? "Off until its install options are chosen" : on ? "Enabled" : "Disabled" },
        h("input", {
          type: "checkbox",
          checked: on,
          disabled: playing || waiting,
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
          m ? `${m.thunderstore ? "Thunderstore · " : m.nexusUrl ? "Nexus Mods · " : ""}${e.id} · ${m.files} file${m.files === 1 ? "" : "s"} · ${fmtBytes(m.size)}` : "No longer installed — remove it from the profile or import it again",
          m?.installer === "waiting"
            ? h("button", { class: "chip warn chip-button", title: "Nothing of this mod is installed until its installer's options are chosen", onclick: () => fomodDialog(m) }, "Choose its install options")
            : m?.placement?.guessed &&
                h("button", { class: "chip warn chip-button", title: "UMM could not tell where this mod's files go", onclick: () => placementDialog(m) }, "Check where its files go"),
        ),
      ),
      h(
        "div",
        { class: "row-actions" },
        m?.placeable &&
          (m.installer
            ? h("button", { class: "icon-btn", title: "Its installer's options, and where its files go", "aria-label": `Install options for ${m.name}`, disabled: playing, onclick: () => fomodDialog(m) }, icon("sliders"))
            : h("button", { class: "icon-btn", title: "Choose where its files go in the game folder", "aria-label": `Where ${m.name} installs`, disabled: playing, onclick: () => placementDialog(m) }, icon("sliders"))),
        m?.nexusUrl &&
          h("button", { class: "icon-btn", title: "Open its page on Nexus Mods", "aria-label": "Open on Nexus Mods", onclick: () => attempt(() => api.openNexusPage(m.nexusUrl)) }, "↗"),
        m?.thunderstore &&
          h("button", { class: "icon-btn", title: "Open its page on Thunderstore", "aria-label": "Open on Thunderstore", onclick: () => attempt(() => api.openPackagePage(S.gameId, m.thunderstore)) }, "↗"),
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
        h("button", { onclick: () => importFrom("archive"), disabled: playing || S.busy }, "Import archive…"),
        h("button", { onclick: () => importFrom("folder"), disabled: playing || S.busy }, "Import folder…"),
      ),
    ),
    entries.length
      ? h("ol", { class: "mod-list" }, rows)
      : h(
          "div",
          { class: "empty drop-hint" },
          h("p", { class: "big" }, "No mods in this profile yet"),
          h("p", {}, "Import an archive (.zip, .7z or .rar) or a folder, or drop one onto this window."),
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
    lovely: "mods/",
    gdweave: "GDWeave/",
    godotml: "mods/ and addons/",
    "return-of-modding": "ReturnOfModding/",
    umm: "UMM/",
    northstar: "R2Northstar/",
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
    p.settings?.length > 0 && [
      h("h3", {}, "Game settings"),
      h("p", { class: "muted" }, "Set while mods are deployed, and put back afterwards. The rest of each file is left as the game keeps it."),
      table(
        ["File", "Setting", "Now"],
        p.settings.map((s) => [
          h("code", {}, s.path),
          h("code", {}, `[${s.section}] ${s.key}=${s.value}`),
          !s.exists
            ? h("span", { class: "muted" }, s.create ? "file created while deployed" : "the game has not written this file yet; left alone")
            : s.current === s.value
              ? h("span", { class: "muted" }, "already set")
              : s.current == null
                ? "not set"
                : `${s.key}=${s.current}`,
        ]),
      ),
    ],
    p.plugins && [
      h("h3", {}, p.plugins.kind === "mods" ? "Mod list" : "Plugins"),
      h(
        "p",
        { class: "muted" },
        p.plugins.kind === "mods"
          ? "This game uses a list to enable your mods and set their load order."
          : "This game uses a list to enable plugins included with your mods and set their load order.",
        " While you play, UMM adds its mods to the end of your list, if you have one.",
        p.plugins.kind === "dated" ? " Note: this game loads plugins in the order of their file dates." : "",
        " ",
        h("code", {}, p.plugins.path),
      ),
      table(
        ["", p.plugins.kind === "mods" ? "Mod" : "Plugin", "From"],
        p.plugins.entries.map((e, i) => [
          h("span", { class: "muted" }, i + 1),
          e.enabled ? h("code", {}, e.name) : h("code", { class: "muted", title: "Listed but not enabled" }, `${e.name} (off)`),
          e.owner || h("span", { class: "muted" }, "your list"),
        ]),
      ),
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

const LINK_TITLE = "Install buttons on mod sites";

async function linkSettings() {
  const st = await attempt(api.linkHandler);
  if (!st) return;
  if (!st.supported) {
    await modal({ title: LINK_TITLE, body: h("p", {}, "This is not available on this system."), actions: [{ label: "OK", value: true, class: "primary", focus: true }] });
    return;
  }
  let status;
  if (st.ours) status = h("p", { class: "muted" }, "These buttons currently open UMM.");
  else if (st.others.length) {
    status = [
      h("p", { class: "muted" }, "They currently open:"),
      st.others.map((o) => h("p", { class: "muted small path-text" }, programOf(o))),
    ];
  } else status = [];
  const v = await modal({
    title: LINK_TITLE,
    body: [
      h("p", {}, "Turn this on to make these buttons open UMM, which then asks which of your games to install into:"),
      h(
        "ul",
        { class: "plain-list" },
        h("li", {}, "“Install with Mod Manager” on thunderstore.io"),
        h("li", {}, "“Mod Manager Download” on nexusmods.com, shown for games Vortex supports (UMM opens the file’s page in its own window, where you click “Slow download”)"),
      ),
      ...[status].flat(2),
    ],
    actions: [
      { label: "Close", value: null, class: "ghost" },
      st.ours ? { label: "Turn off", value: "off", class: "" } : { label: "Turn on", value: "on", class: "primary" },
    ],
  });
  if (v) {
    const r = await attempt(() => api.setLinkHandler(v === "on"), v === "on" ? "Install buttons on mod sites now open UMM" : "Install buttons on mod sites no longer open UMM");
    if (r) renderSidebar();
  }
}

/// A Nexus Mods download link from the website's "Mod Manager Download" button.
async function handleNxmLink(url) {
  const l = await attempt(() => api.nexusResolveLink(url));
  if (!l) return;
  const what = l.modName ? `${l.modName}${l.fileName && l.fileName !== l.modName ? ` (${l.fileName})` : ""}${l.version ? ` ${l.version}` : ""}` : "this file";
  const ok1 = (text) => modal({ title: `Install ${what}?`, body: h("p", {}, text), actions: [{ label: "OK", value: true, class: "primary", focus: true }] });
  if (!l.games.length) return ok1(`None of your games can take files from nexusmods.com/${l.domain}: add the game first.`);
  const matching = l.games.filter((g) => g.matches);
  const shown = matching.length ? matching : l.games;
  const pick = h("select", { "aria-label": "Game" }, shown.map((g) => h("option", { value: g.id, selected: g.id === S.gameId }, g.name)));
  const go = await modal({
    title: `Install ${what}?`,
    body: [
      h("p", {}, "From a Nexus Mods download link, into the current profile of the game you pick. Its page opens in a UMM window, where you click “Slow download”."),
      h("label", { class: "field" }, h("span", {}, "Game"), pick),
      !matching.length && h("p", { class: "muted small" }, `None of your games is set to nexusmods.com/${l.domain} yet; the game you pick will be.`),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Download", value: "go", class: "primary", focus: true },
    ],
  });
  if (go !== "go") return;
  // A download UMM was waiting for in the browser is not needed any more.
  if (cancelWaiting) cancelWaiting();
  if (S.gameId !== pick.value) {
    S.gameId = pick.value;
    S.profileName = "Default";
    await refreshAll();
  }
  S.tab = "nexus";
  if (S.nexus.game?.game !== S.gameId) loadNexus(true);
  await nexusGet(l.domain, l.modId, l.fileId, l.modName || "the file");
}

async function handleInstallLink(url) {
  if (/^nxm:\/\//i.test(url)) return handleNxmLink(url);
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

// ----- Nexus Mods tab -----

async function loadActions() {
  loadLoaderChoice();
}

let nexusSearchTimer = null;

/// Loads the Nexus tab: the game's Nexus game, then a page of mods (from the top unless
/// `append`) and the installed mods' updates.
async function loadNexus(reset, append = false) {
  const game = S.gameId;
  const N = S.nexus;
  if (reset || N.game?.game !== game) {
    N.game = null;
    N.page = null;
    N.updates = null;
    N.picker = null;
    renderMain();
    const g = await attempt(() => api.nexusGame(game));
    if (S.gameId !== game) return;
    N.game = g ? { ...g, game } : { domain: null, game, error: true };
    if (!N.game.domain) {
      renderMain();
      if (!g) return;
      return nexusFindGames(S.details.game.name);
    }
    attempt(() => api.nexusUpdates(game)).then((u) => {
      if (S.gameId === game && u) {
        N.updates = u;
        if (S.tab === "nexus") renderMain();
      }
    });
  }
  if (!append) {
    N.query.offset = 0;
    N.page = null;
    renderMain();
  }
  const q = { ...N.query };
  N.stale = false;
  const p = await attempt(() => api.nexusSearch(game, q));
  if (S.gameId !== game) return;
  if (!p) N.page = { error: true, items: [], total: 0 };
  else if (append && N.page) N.page = { ...p, items: [...N.page.items, ...p.items] };
  else N.page = p;
  if (S.tab === "nexus") renderMain();
}

async function nexusFindGames(name) {
  const game = S.gameId;
  S.nexus.picker = { name, results: null };
  renderMain();
  const r = await attempt(() => api.nexusFindGames(name));
  if (S.gameId === game && S.nexus.picker) {
    S.nexus.picker.results = r || [];
    renderMain();
  }
}

async function nexusUseGame(domain) {
  const g = await attempt(() => api.setNexusDomain(S.gameId, domain), domain ? "Nexus Mods game saved" : "");
  if (g) loadNexus(true);
}

function nexusPicker(g) {
  const P = S.nexus.picker || { name: g.name, results: null };
  const field = h("input", { type: "search", value: P.name, placeholder: "Game name", "aria-label": "Game name on Nexus Mods" });
  field.addEventListener("keydown", (e) => e.key === "Enter" && nexusFindGames(field.value));
  return h(
    "section",
    { class: "panel" },
    h("h2", {}, "Which game is this on Nexus Mods?"),
    h("p", { class: "muted" }, `UMM did not find “${g.name}” on Nexus Mods by its name. Search for it and pick it; UMM saves the choice in this game’s game.toml.`),
    h("div", { class: "ts-toolbar" }, field, h("button", { onclick: () => nexusFindGames(field.value) }, "Search")),
    P.results === null
      ? h("div", { class: "loading" }, h("span", { class: "spinner" }), "Searching…")
      : P.results.length
        ? h(
            "ul",
            { class: "nexus-games" },
            P.results.map((r) =>
              h(
                "li",
                {},
                h("span", {}, h("strong", {}, r.name), h("span", { class: "muted small" }, ` nexusmods.com/${r.domain} · ${fmtCount(r.modCount || 0)} mods`)),
                h("button", { class: "small", onclick: () => nexusUseGame(r.domain) }, "Use this game"),
              ),
            ),
          )
        : h("p", { class: "muted" }, "No games found by that name."),
  );
}

function nexusTab(d) {
  const g = d.game;
  const N = S.nexus;
  if (!N.game || N.game.game !== g.id) return h("section", { class: "panel" }, h("div", { class: "loading" }, h("span", { class: "spinner" }), "Looking for this game on Nexus Mods…"));
  if (N.game.error)
    return h("section", { class: "panel" }, h("p", { class: "muted" }, "Nexus Mods could not be reached.", " ", h("button", { class: "link", onclick: () => loadNexus(true) }, "Try again")));
  if (!N.game.domain) return nexusPicker(g);

  const playing = S.session.phase !== "idle" && S.session.game === g.id;
  const locked = playing || g.deployed || !!N.busy;
  const q = N.query;
  const p = N.page;
  const updatesByMod = {};
  for (const u of N.updates || []) if (u.state === "newer") (updatesByMod[u.modId] ||= []).push(u);
  const pending = (N.updates || []).filter((u) => u.state === "newer");

  const search = h("input", { type: "search", placeholder: "Search mods by name", "aria-label": "Search mods", value: q.text });
  search.addEventListener("input", () => {
    q.text = search.value;
    clearTimeout(nexusSearchTimer);
    nexusSearchTimer = setTimeout(async () => {
      await loadNexus(false);
      const box = $(".nexus-toolbar input[type=search]");
      if (box) {
        box.focus();
        box.setSelectionRange(box.value.length, box.value.length);
      }
    }, 300);
  });
  const sort = h(
    "select",
    { "aria-label": "Sort" },
    [
      ["endorsements", "Most endorsed"],
      ["downloads", "Most downloaded"],
      ["updated", "Recently updated"],
      ["name", "Name"],
    ].map(([v, l]) => h("option", { value: v, selected: v === q.sort }, l)),
  );
  sort.addEventListener("change", () => {
    q.sort = sort.value;
    loadNexus(false);
  });
  const installedOnly = h("input", { type: "checkbox", checked: q.installed });
  installedOnly.addEventListener("change", () => {
    q.installed = installedOnly.checked;
    loadNexus(false);
  });

  const banners = [];
  if (N.busy) banners.push(h("div", { class: "ts-banner live nexus-live" }, h("span", { class: "spinner" }), N.busy.message));
  if (g.deployed || playing) banners.push(h("div", { class: "ts-banner warn" }, "Mods are deployed; installing and updating wait until the game folder is restored."));
  if (pending.length)
    banners.push(
      h(
        "div",
        { class: "ts-banner" },
        h("span", {}, `${pending.length} update${pending.length === 1 ? "" : "s"}: ${pending.map((u) => `${u.name} ${u.version || ""} → ${u.newVersion}`).join(", ")}`),
        pending.length === 1 && h("button", { class: "small", disabled: locked, onclick: () => nexusGet(N.game.domain, pending[0].modId, pending[0].newFileId, pending[0].name) }, "Update"),
      ),
    );

  let list;
  if (!p) list = h("div", { class: "loading" }, h("span", { class: "spinner" }), "Loading mods…");
  else if (p.error) list = h("p", { class: "muted" }, "The mod list could not be loaded.", " ", h("button", { class: "link", onclick: () => loadNexus(false) }, "Try again"));
  else if (!p.items.length) list = h("p", { class: "muted" }, "No mods match.");
  else
    list = h(
      "div",
      { class: "ts-list" },
      p.items.map((it) => {
        const up = updatesByMod[it.modId];
        const action = up
          ? h("button", { class: "small", disabled: locked, onclick: () => nexusGet(N.game.domain, it.modId, up[0].newFileId, it.name) }, `Update to ${up[0].newVersion}`)
          : it.installed.length
            ? h("div", { class: "row" }, h("span", { class: "chip ok" }, `Installed ${it.installed.join(", ")}`), h("button", { class: "ghost small", disabled: locked, onclick: () => nexusFilesDialog(it) }, "Files…"))
            : h("button", { class: "primary small", disabled: locked, onclick: () => nexusFilesDialog(it) }, "Download…");
        return h(
          "div",
          { class: "ts-row" },
          it.thumbnail ? h("img", { class: "ts-icon nexus-thumb", src: it.thumbnail, alt: "", loading: "lazy" }) : h("div", { class: "ts-icon nexus-thumb" }),
          h(
            "div",
            { class: "ts-main" },
            h(
              "div",
              { class: "ts-title" },
              h("button", { class: "link ts-name", title: "Open its page on nexusmods.com", onclick: () => attempt(() => api.openNexusPage(it.url)) }, it.name, icon("external")),
              it.author && h("span", { class: "muted" }, ` by ${it.author}`),
              it.version && h("span", { class: "muted" }, ` · ${it.version}`),
            ),
            h("div", { class: "ts-desc" }, it.summary),
            h(
              "div",
              { class: "ts-meta muted" },
              `${fmtCount(it.downloads)} downloads · ${fmtCount(it.endorsements)} endorsements`,
              it.category ? ` · ${it.category}` : "",
              it.updatedAt ? ` · updated ${it.updatedAt.slice(0, 10)}` : "",
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
      h("span", { class: "muted small" }, `Nexus Mods game: ${N.game.name || N.game.domain}`),
      h("button", { class: "link", onclick: () => attempt(() => api.openNexusPage(N.game.url)) }, "Open on nexusmods.com", icon("external")),
      h(
        "button",
        {
          class: "link",
          onclick: () => {
            const name = N.game.name || g.name;
            N.game = { domain: null, game: g.id };
            nexusFindGames(name);
          },
        },
        "Change…",
      ),
    ),
    h(
      "div",
      { class: "ts-toolbar nexus-toolbar" },
      search,
      sort,
      h("label", { class: "check inline" }, installedOnly, "Installed"),
      h("button", { class: "ghost small", onclick: () => loadNexus(true), title: "Load the list and updates again" }, "Refresh"),
    ),
    banners,
    p && !p.error && h("p", { class: "muted small" }, `${p.total} mod${p.total === 1 ? "" : "s"} · downloads open in a UMM window on nexusmods.com · into the profile “${S.profileName}”`),
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
              loadNexus(false, true);
            },
          },
          "Show more",
        ),
      ),
  );
}

/// What a mod needs, in its files dialog. `onGet` closes the dialog to get one of them.
function requirementList(reqs, onGet) {
  if (!reqs.length) return null;
  return h(
    "div",
    { class: "nexus-reqs" },
    h("h3", {}, "Needs these mods too"),
    reqs.map((r) =>
      h(
        "div",
        { class: "nexus-req" },
        h(
          "span",
          {},
          r.url
            ? h("button", { class: "link inline-link", title: "Open its page", onclick: () => attempt(() => (r.modId ? api.openNexusPage(r.url) : api.openWebPage(r.url))) }, r.name, " ↗")
            : h("strong", {}, r.name),
          r.notes && h("span", { class: "muted small" }, ` · ${r.notes}`),
          !r.modId && h("span", { class: "muted small" }, " · not on Nexus Mods for this game: install it yourself"),
        ),
        r.installed ? h("span", { class: "chip ok" }, "Installed") : r.modId ? h("button", { class: "small", onclick: () => onGet(r) }, "Get…") : null,
      ),
    ),
  );
}

/// Offers the mods `name` still needs, one at a time.
async function offerRequirements(name, missing) {
  while (missing.length) {
    const v = await modal({
      title: `${name} needs other mods`,
      body: [
        h("p", {}, `Its author lists ${missing.length === 1 ? "this mod" : "these mods"} as needed:`),
        h("ul", { class: "plain-list" }, missing.map((r) => h("li", {}, r.name, r.notes && h("span", { class: "muted small" }, ` · ${r.notes}`)))),
      ],
      actions: [
        { label: "Later", value: null, class: "ghost" },
        { label: `Get ${missing[0].name}…`, value: "get", class: "primary", focus: true },
      ],
    });
    if (v !== "get") return;
    const r = missing.shift();
    await nexusFilesDialog({ modId: r.modId, name: r.name });
  }
}

async function nexusFilesDialog(it) {
  const game = S.gameId;
  const f = await attempt(() => api.nexusFiles(game, it.modId));
  if (!f) return;
  let wanted = null;
  const current = f.files.filter((x) => !x.old);
  const old = f.files.filter((x) => x.old);
  const first = current.find((x) => x.category === "MAIN" && !x.installed) || current.find((x) => !x.installed) || current[0] || old[0];
  const row = (x) =>
    h(
      "label",
      { class: `nexus-file ${x.old ? "old" : ""}` },
      h("input", { type: "radio", name: "nexus-file", value: String(x.fileId), checked: x === first }),
      h(
        "span",
        {},
        h("strong", {}, x.name),
        " ",
        h("span", { class: "muted small" }, [x.version, x.category.toLowerCase().replace("_", " "), x.size != null ? fmtBytes(x.size) : null, x.installed ? "installed" : null].filter(Boolean).join(" · ")),
        x.description && h("span", { class: "nexus-file-desc muted small" }, x.description.replace(/<br\s*\/?>/gi, " ").replace(/<[^>]+>/g, "").slice(0, 300)),
      ),
    );
  const files = h("div", { class: "nexus-files" }, current.map(row), old.length > 0 && h("details", {}, h("summary", { class: "muted small" }, `Older versions (${old.length})`), old.map(row)));
  const body = [
    h("p", { class: "muted" }, "Pick a file. Its page opens in a UMM window: click “Slow download” there (log in first if asked), and UMM installs the file once it has downloaded."),
    files,
    requirementList(f.requirements, (r) => {
      wanted = r;
      $("#modal-root .modal-actions button")?.click();
    }),
  ];
  if (!f.files.length) body.push(h("p", { class: "muted" }, "This mod has no files."));
  const v = await modal({
    title: f.name,
    body,
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      f.files.length > 0 && { label: "Download", value: "go", class: "primary", focus: true },
    ].filter(Boolean),
  });
  if (wanted) return nexusFilesDialog({ modId: wanted.modId, name: wanted.name });
  if (v !== "go") return;
  // The modal is gone; its radio buttons are still in hand.
  const picked = files.querySelector("input:checked");
  if (!picked) return;
  const ok = await nexusGet(f.domain, f.modId, Number(picked.value), f.name);
  const missing = f.requirements.filter((r) => r.modId && !r.installed);
  if (ok && missing.length) await offerRequirements(f.name, missing);
  return ok;
}

/// Closes the dialog waiting for a browser download, if one is open.
let cancelWaiting = null;

/// Shows a dialog while the file is downloaded on the website, in UMM's Nexus Mods window or
/// the user's browser. Resolves to `{ path }` (an archive to install), or null.
function waitForDownload(dir, label) {
  return new Promise((resolve) => {
    const root = $("#modal-root");
    let done = false;
    let inBrowser = false;
    const finish = (v) => {
      if (done) return;
      done = true;
      cancelWaiting = null;
      clearInterval(timer);
      root.replaceChildren();
      resolve(v);
    };
    cancelWaiting = () => finish(null);
    const choose = async () => {
      const p = await attempt(() => api.pickPath("archive"));
      if (p) finish({ path: p });
    };
    const how = h("p", {}, "Its page is open in a UMM window. Click “Slow download” there; log in to Nexus Mods in that window first if the page asks. UMM installs the file as soon as it has downloaded.");
    const waiting = h("div", { class: "loading" }, h("span", { class: "spinner" }), "Waiting for the download…");
    const note = h("p", { class: "muted small" }, "Logging in there keeps you logged in for next time.");
    const reopen = h(
      "p",
      {},
      h("span", { class: "muted" }, "The Nexus Mods window is closed. "),
      h(
        "button",
        {
          class: "small",
          onclick: async () => {
            if ((await attempt(api.nexusDownloadReopen)) === undefined) return;
            reopen.replaceWith(waiting);
          },
        },
        "Reopen it",
      ),
    );
    const browserBtn = h(
      "button",
      {
        class: "ghost",
        onclick: async () => {
          if ((await attempt(api.nexusDownloadInBrowser)) === undefined) return;
          inBrowser = true;
          browserBtn.remove();
          if (reopen.isConnected) reopen.replaceWith(waiting);
          how.textContent = "Its page is open in your browser. Click “Slow download” there; log in to Nexus Mods first if the page asks.";
          waiting.lastChild.textContent = `Waiting for it in ${dir}…`;
          note.textContent = "If your browser saves downloads somewhere else, choose the file once it is done.";
        },
      },
      "Use my browser instead",
    );
    const dlg = h(
      "div",
      { class: "modal", role: "dialog", "aria-modal": "true", "aria-label": `Download ${label}` },
      h("h2", {}, `Download ${label}`),
      h("div", { class: "modal-body" }, how, waiting, note),
      h(
        "div",
        { class: "modal-actions" },
        h("button", { class: "ghost", onclick: () => finish(null) }, "Cancel"),
        browserBtn,
        h("button", { onclick: choose }, "Choose the file…"),
      ),
    );
    root.replaceChildren(h("div", { class: "scrim" }, dlg));
    const timer = setInterval(async () => {
      const p = await api.nexusDownloadPoll().catch(() => null);
      if (!p) return;
      if (p.path) finish({ path: p.path });
      else if (p.failed) {
        note.textContent = "The download did not finish. Try again on the page.";
        note.className = "small error-text";
      } else if (p.closed && !inBrowser && !reopen.isConnected) {
        // Still possible to choose a file or switch to the browser: offer the window again.
        waiting.replaceWith(reopen);
      }
    }, 1000);
  });
}

/// Says what was installed, and asks where its files go when UMM could not tell.
async function nexusToast(r) {
  const v = r.version ? ` ${r.version}` : "";
  toast(r.installerWaiting ? `Downloaded ${r.name}${v}` : r.replaced ? `Updated ${r.name} to${v || " its newest file"}` : `Installed ${r.name}${v}`, "ok");
  await afterInstall({ id: r.id, name: r.name, installer: r.installerWaiting ? "waiting" : null, placement: { guessed: r.placementGuessed } }, !r.replaced);
}

/// Runs a Nexus install for the current game, with its progress at the top of the Nexus tab.
async function nexusRun(title, call) {
  S.nexus.busy = { message: title };
  renderMain();
  const r = await attempt(call);
  S.nexus.busy = null;
  await refreshAll();
  if (S.tab === "nexus") loadNexus(true);
  return r;
}

/// Gets one file of a mod through the website: in UMM's Nexus Mods window, or the browser.
async function nexusGet(domain, modId, fileId, label) {
  const game = S.gameId;
  const dir = await attempt(() => api.nexusDownloadStart(game, domain, modId, fileId));
  if (!dir) return false;
  const got = await waitForDownload(dir, label);
  if (!got) {
    attempt(api.nexusDownloadCancel);
    return false;
  }
  const path = got.path;
  let r = await nexusRun("Installing", () => api.nexusDownloadInstall(S.profileName, path, false));
  if (r?.unrecognised) {
    const ok = await confirmModal(
      "Install this file?",
      `Nexus Mods does not recognise ${path.split(/[\\/]/).pop()} by its contents, so UMM cannot check that it is ${label}. Install it as that file anyway?`,
      "Install",
    );
    r = ok ? await nexusRun("Installing", () => api.nexusDownloadInstall(S.profileName, path, true)) : null;
    if (!ok) attempt(api.nexusDownloadCancel);
  }
  if (r?.installed) await nexusToast(r.installed);
  return !!r?.installed;
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
        : S.tab === "nexus"
          ? nexusTab(d)
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
  return importPaths([path]);
}

/// Imports one mod after another. With a single mod, UMM then asks what it still needs (its
/// install options, or where its files go); with several, those wait in the mod list.
async function importPaths(paths) {
  if (!S.gameId || !paths.length) return;
  const done = [];
  for (const path of paths) {
    S.busy = `Importing ${path.split(/[\\/]/).pop()}…`;
    renderMain();
    const m = await attempt(() => api.importMod(S.gameId, path, S.profileName));
    if (m) done.push(m);
  }
  S.busy = false;
  if (done.length === 1) toast(`Imported ${done[0].name} (${done[0].files} files) into ${S.profileName}`, "ok");
  else if (done.length > 1) {
    const waiting = done.filter((m) => m.installer === "waiting").length;
    toast(`Imported ${done.length} mods into ${S.profileName}${waiting ? `; ${waiting} need${waiting === 1 ? "s" : ""} its install options chosen` : ""}`, "ok");
  }
  await refreshAll();
  if (paths.length === 1) await afterInstall(done[0]);
}

/// Asks what a new mod still needs: its installer's options, or where its files go when UMM
/// could not tell.
/// `fresh`: a new mod, not an update; cancelling its installer removes it again, since
/// nothing of it would ever be installed.
async function afterInstall(m, fresh = true) {
  if (m?.installer === "waiting") {
    const saved = await fomodDialog(m);
    if (!saved && fresh) {
      await attempt(() => api.removeMod(S.gameId, m.id), `Cancelled installing ${m.name}`);
      await refreshAll();
    }
  } else if (m?.placement?.guessed) await placementDialog(m, true);
}

const FOMOD_HINTS = {
  exactly_one: "Choose one",
  at_most_one: "Choose one, or none",
  at_least_one: "Choose one or more",
  all: "All of these are installed",
  any: "Choose any",
};
const FOMOD_KINDS = { required: "Required", recommended: "Recommended", not_usable: "Not usable", could_be_usable: "May not work" };

/// A mod's FOMOD installer: its options step by step, then the game folder its files go
/// under. Resolves to true once saved.
async function fomodDialog(m) {
  const game = S.gameId;
  let view = await attempt(() => api.fomodView(game, m.id, null, null));
  if (!view) return false;
  const key = (step, group) => `${step}\u0000${group}`;
  // The options in effect, by step and group: the saved ones, else the installer's defaults.
  const chosen = new Map(view.choices.map((c) => [key(c.step, c.group), c.plugins]));
  let target = view.target;
  let page = 0;
  let focus = null;
  const images = new Map();
  const choices = () => [...chosen].map(([k, plugins]) => ({ step: k.split("\u0000")[0], group: k.split("\u0000")[1], plugins }));

  return new Promise((resolve) => {
    const root = $("#modal-root");
    const close = (v) => {
      root.replaceChildren();
      document.removeEventListener("keydown", onKey);
      resolve(v);
    };
    const onKey = (e) => {
      if (e.key === "Escape") close(false);
    };
    document.addEventListener("keydown", onKey);
    const dlg = h("div", { class: "modal fomod", role: "dialog", "aria-modal": "true", "aria-label": `Install options for ${m.name}` });
    root.replaceChildren(h("div", { class: "scrim" }, dlg));

    const reload = async () => {
      const v = await attempt(() => api.fomodView(game, m.id, choices(), target));
      if (v) {
        view = v;
        for (const c of v.choices) chosen.set(key(c.step, c.group), c.plugins);
      }
      draw();
    };

    // The option described on the right: the one last pointed at, else the step's first chosen.
    const detail = (step) => {
      const all = step.groups.flatMap((g) => g.options);
      const o = all.find((x) => x.name === focus) || all.find((x) => x.selected) || all[0];
      if (!o) return h("div", { class: "fomod-detail" });
      // The same room for a picture whichever option is shown, so pointing at another
      // option never resizes the dialog and moves the options under the mouse.
      const pictures = all.some((x) => x.image);
      const img = pictures && h("div", { class: "fomod-image" });
      if (o.image && img) {
        const show = (url) => url && img.replaceChildren(h("img", { src: url, alt: "" }));
        if (images.has(o.image)) show(images.get(o.image));
        else
          api.modImage(game, m.id, o.image).then(
            (url) => {
              images.set(o.image, url);
              show(url);
            },
            () => images.set(o.image, null),
          );
      }
      return h("div", { class: "fomod-detail" }, img, h("h3", {}, o.name), h("p", { class: "fomod-desc" }, o.description || "No description."));
    };

    const group = (step, g) => {
      const radio = g.kind === "exactly_one" || g.kind === "at_most_one";
      const name = `fomod-${step.name}-${g.name}`;
      const set = (plugins) => {
        chosen.set(key(step.name, g.name), plugins);
        reload();
      };
      const row = (o) => {
        const locked = o.kind === "not_usable" || o.kind === "required" || g.kind === "all";
        return h(
          "label",
          {
            class: `fomod-option ${o.kind === "not_usable" ? "unusable" : ""}`,
            onmouseenter: () => {
              focus = o.name;
              const d = dlg.querySelector(".fomod-detail");
              d?.replaceWith(detail(step));
            },
          },
          h("input", {
            type: radio ? "radio" : "checkbox",
            name,
            checked: o.selected,
            disabled: locked,
            onchange: (ev) => {
              focus = o.name;
              const now = g.options.filter((x) => x.selected).map((x) => x.name);
              set(radio ? [o.name] : ev.target.checked ? [...now, o.name] : now.filter((n) => n !== o.name));
            },
          }),
          h("span", {}, o.name),
          FOMOD_KINDS[o.kind] && h("span", { class: `chip ${o.kind === "not_usable" ? "warn" : ""}` }, FOMOD_KINDS[o.kind]),
        );
      };
      return h(
        "fieldset",
        { class: "fomod-group" },
        h("legend", {}, g.name || step.name, h("span", { class: "muted small" }, ` · ${FOMOD_HINTS[g.kind]}`)),
        g.problem && h("p", { class: "small error-text" }, g.problem[0].toUpperCase() + g.problem.slice(1) + "."),
        g.options.map(row),
        g.kind === "at_most_one" &&
          h(
            "label",
            { class: "fomod-option" },
            h("input", { type: "radio", name, checked: !g.options.some((o) => o.selected), onchange: () => set([]) }),
            h("span", { class: "muted" }, "None"),
          ),
      );
    };

    const draw = () => {
      const steps = view.steps.filter((s) => s.visible);
      page = Math.min(page, steps.length);
      const last = page === steps.length;
      const step = steps[page];
      const stepOk = last || step.groups.every((g) => !g.problem);
      let body;
      if (!last) {
        body = h("div", { class: "fomod-step" }, h("div", { class: "fomod-groups" }, step.groups.map((g) => group(step, g))), detail(step));
      } else {
        const targetIn = h("input", {
          type: "text",
          list: "fomod-game-folders",
          value: target,
          placeholder: "The game folder itself",
          "aria-label": "Into this game folder",
          onchange: (ev) => {
            target = ev.target.value;
            reload();
          },
        });
        body = h(
          "div",
          {},
          h("label", { class: "field" }, h("span", {}, "Into this game folder"), targetIn, h("datalist", { id: "fomod-game-folders" }, view.gameFolders.map((d) => h("option", { value: d })))),
          h("p", { class: "muted small" }, `UMM suggests ${view.suggestedTarget ? `“${view.suggestedTarget}”` : "the game folder itself"}, from what the game folder already holds. `, view.suggestedTarget !== target && h("button", { class: "link inline-link", onclick: () => ((target = view.suggestedTarget), reload()) }, "Apply")),
          view.needsUnmet && h("p", { class: "banner warn" }, "The mod says it needs something this game does not have (another mod, or a newer version of the game). It may not work."),
          view.missing.length > 0 && h("p", { class: "small error-text" }, `The installer names files the archive does not have: ${view.missing.slice(0, 4).join(", ")}${view.missing.length > 4 ? "…" : ""}`),
          h("p", { class: "muted small" }, `${plural(view.files.length, "file")} to install:`),
          h(
            "div",
            { class: "place-files" },
            view.files.map((f) => h("div", { class: "place-file" }, h("span"), h("code", { class: "place-from" }, f.destination), h("span", { class: "place-to muted" }, target ? `in ${target}` : ""))),
          ),
        );
      }
      dlg.replaceChildren(
        ...[
        h("h2", {}, `${view.name}: install options`),
        h("p", { class: "muted small" }, last ? `Last step of ${steps.length + 1}: where it installs` : `Step ${page + 1} of ${steps.length + 1}: ${step.name}`),
        h("div", { class: "modal-body" }, body),
        h(
          "div",
          { class: "modal-actions" },
          h("button", { class: "ghost", onclick: () => close(false) }, "Cancel"),
          page > 0 && h("button", { onclick: () => ((page -= 1), (focus = null), draw()) }, "Back"),
          last
            ? h(
                "button",
                {
                  class: "primary",
                  disabled: !view.complete,
                  onclick: async () => {
                    const r = await attempt(() => api.fomodApply(game, m.id, choices(), target), `Saved the install options for ${m.name}`);
                    if (r === undefined) return;
                    close(true);
                    await refreshAll();
                  },
                },
                "Save",
              )
            : h("button", { class: "primary", disabled: !stepOk, onclick: () => ((page += 1), (focus = null), draw()) }, "Next"),
        ),
        ].filter(Boolean),
      );
    };
    draw();
  });
}

/// Where a mod's files go: the folder inside it they install from, the game folder they go
/// into, and the files left out. `asked` when UMM could not tell and opened it after an install.
async function placementDialog(m, asked = false) {
  const game = S.gameId;
  let view = await attempt(() => api.placementView(game, m.id, null));
  if (!view) return;
  const p = { ...view.placement, skip: [...view.placement.skip] };
  const list = h("div", { class: "place-files" });
  const summary = h("p", { class: "muted small" });
  const under = (f) => !p.root || f.toLowerCase().startsWith(`${p.root.toLowerCase()}/`);
  const shown = (f) => (p.root ? f.slice(p.root.length + 1) : f);
  let timer = null;
  const refresh = (now = false) => {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      try {
        view = await api.placementView(game, m.id, p);
        draw();
      } catch (e) {
        summary.textContent = String(e);
        summary.className = "small error-text";
      }
    }, now ? 0 : 300);
  };
  const draw = () => {
    const inside = view.files.filter((f) => under(f.file));
    const placed = inside.filter((f) => f.target).length;
    const outside = view.files.length - inside.length;
    summary.className = "muted small";
    summary.textContent =
      `${plural(placed, "file")} go${placed === 1 ? "es" : ""} into the game` +
      (inside.length - placed ? `, ${inside.length - placed} left out` : "") +
      (outside ? `; ${plural(outside, "file")} outside this folder ${outside === 1 ? "is" : "are"} left out` : "") +
      ".";
    list.replaceChildren(
      ...inside.map((f) =>
        h(
          "label",
          { class: `place-file ${f.target ? "" : "out"}` },
          h("input", {
            type: "checkbox",
            checked: !f.skipped,
            "aria-label": `Install ${f.file}`,
            onchange: (ev) => {
              p.skip = p.skip.filter((s) => s.toLowerCase() !== f.file.toLowerCase());
              if (!ev.target.checked) p.skip.push(f.file);
              refresh(true);
            },
          }),
          h("code", { class: "place-from" }, shown(f.file)),
          h("span", { class: "place-to" }, f.target ? h("code", {}, `→ ${f.target}`) : h("span", { class: "muted" }, f.note)),
        ),
      ),
    );
  };
  const rootSel = h(
    "select",
    {
      "aria-label": "Install from",
      onchange: (ev) => {
        p.root = ev.target.value;
        p.skip = p.skip.filter(under);
        refresh(true);
      },
    },
    h("option", { value: "", selected: !p.root }, "The whole mod"),
    view.folders.map((d) => h("option", { value: d, selected: d === p.root }, d)),
  );
  const targetIn = h("input", {
    type: "text",
    list: "place-game-folders",
    value: p.target,
    placeholder: "The game folder itself",
    "aria-label": "Into this game folder",
    oninput: (ev) => {
      p.target = ev.target.value;
      refresh();
    },
  });
  draw();
  const s = view.suggested;
  // The files the suggestion leaves out, as they appear in the list below.
  const inRoot = (f) => (s.root && f.toLowerCase().startsWith(`${s.root.toLowerCase()}/`) ? f.slice(s.root.length + 1) : f);
  const names = s.skip.map(inRoot);
  const leftOut = !names.length
    ? ""
    : names.length <= 3
      ? `, leaving out ${names.slice(0, -1).join(", ")}${names.length > 1 ? " and " : ""}${names[names.length - 1]}`
      : `, leaving out ${names.slice(0, 3).join(", ")} and ${names.length - 3} more`;
  const key = (x) => [x.root.toLowerCase(), x.target.toLowerCase(), ...x.skip.map((f) => f.toLowerCase()).sort()].join("\n");
  const showSuggestion = () => {
    p.root = s.root;
    p.target = s.target;
    p.skip = [...s.skip];
    rootSel.value = s.root;
    targetIn.value = s.target;
    refresh(true);
  };
  const v = await modal({
    title: `Where ${m.name} installs`,
    body: [
      asked && h("p", { class: "banner warn" }, "UMM could not tell where this mod's files go from what is in the game folder. Check them before playing."),
      h("p", { class: "muted" }, "Pick the folder inside the mod to install from, and the game folder its contents go into. Untick files to leave them out."),
      h("label", { class: "field" }, h("span", {}, "Install from"), rootSel),
      h("label", { class: "field" }, h("span", {}, "Into this game folder"), targetIn, h("datalist", { id: "place-game-folders" }, view.gameFolders.map((d) => h("option", { value: d })))),
      h(
        "p",
        { class: "muted small" },
        `UMM suggests: ${s.root ? `“${s.root}”` : "the whole mod"} into ${s.target ? `“${s.target}”` : "the game folder"}${leftOut}${s.guessed ? " (a guess)" : ""}. `,
        h("button", { class: "link inline-link", title: "Fill in the suggestion below; nothing is saved until you press Save", onclick: showSuggestion }, "Apply"),
      ),
      summary,
      list,
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Save", value: "save", class: "primary", focus: true },
    ],
  });
  clearTimeout(timer);
  if (v !== "save") return;
  // Saved as UMM's own when it is the suggestion, so updates keep suggesting afresh.
  if (key(p) === key(s)) await attempt(() => api.setPlacement(game, m.id, null), `${m.name} uses UMM's suggestion`);
  else await attempt(() => api.setPlacement(game, m.id, p), `Saved where ${m.name} installs`);
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
    // Into this game's activity, as a session's own restore is.
    if (S.session.game !== S.gameId) S.session = { game: S.gameId, phase: "idle", log: [] };
    logLine(`Restored the game folder: ${plural(r.restored, "original file")} back, ${plural(r.removed, "mod file")} removed`, "ok");
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
      importPaths(p.paths || []);
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
  api.listen("nexus-progress", (e) => {
    if (S.nexus.busy && e.game === S.gameId) {
      S.nexus.busy.message = e.message;
      const b = $(".nexus-live");
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

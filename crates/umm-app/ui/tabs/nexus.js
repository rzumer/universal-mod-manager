import { api } from "../api.js";
import { afterInstall } from "../actions.js";
import { refreshAll } from "../data.js";
import { $, fmtBytes, h, icon, plural } from "../dom.js";
import { banner, modal } from "../modals.js";
import { header } from "../panel.js";
import { S, attempt, toast } from "../state.js";
import { action, loadLoaderChoice, renderMain } from "./gamebanana.js";
import { storeBanner, storeBanners, storeCheck, storeList, storeMore, storePage, storeRow, storeRun, storeSearch, storeSelect, storeTab } from "./store.js";
import { fmtCount } from "./thunderstore.js";

// ----- Nexus Mods tab -----

/// A Nexus Mods collection link (the website's install button, or a link pasted into the
/// collection search): opens the collection as a checklist, for the game picked.
export async function handleCollectionLink(url) {
  const l = await attempt(() => api.nexusCollectionLink(url));
  if (l === undefined) return;
  if (!l) return toast("Not a Nexus Mods collection link", "error");
  const ok1 = (text) => modal({ title: "Open this collection?", body: h("p", {}, text), actions: [{ label: "OK", value: true, class: "primary", focus: true }] });
  if (!l.games.length) return ok1(`None of your games can take files from nexusmods.com/${l.domain}: add the game first.`);
  const matching = l.games.filter((g) => g.matches);
  let id = matching.length === 1 ? matching[0].id : null;
  if (!id) {
    const shown = matching.length ? matching : l.games;
    const pick = h("select", { "aria-label": "Game" }, shown.map((g) => h("option", { value: g.id, selected: g.id === S.gameId }, g.name)));
    const go = await modal({
      title: "Open this collection?",
      body: [
        h("p", {}, "UMM shows the collection’s mods as a checklist, with what is installed for the game you pick."),
        h("label", { class: "field" }, h("span", {}, "Game"), pick),
        !matching.length && h("p", { class: "muted small" }, `None of your games is set to nexusmods.com/${l.domain} yet. Set the game you pick to it in its Nexus Mods tab first.`),
      ],
      actions: [
        { label: "Cancel", value: null, class: "ghost" },
        { label: "Open", value: "go", class: "primary", focus: true },
      ],
    });
    if (go !== "go") return;
    id = pick.value;
  }
  if (S.gameId !== id) {
    S.gameId = id;
    S.profileName = "Default";
    await refreshAll();
  }
  S.tab = "nexus";
  S.nexus.view = "collections";
  if (S.nexus.game?.game !== S.gameId) loadNexus(true);
  openCollection(l.slug, l.revision);
}

/// Loads a page of the game's collections (from the top unless `append`). A collection link
/// typed into the search opens that collection instead.
export async function loadCollections(append = false) {
  const game = S.gameId;
  const N = S.nexus;
  const q = N.cquery;
  if (/^(https?|nxm):\/\//i.test(q.text.trim())) {
    const link = q.text.trim();
    q.text = "";
    return handleCollectionLink(link);
  }
  if (!append) {
    q.offset = 0;
    N.cpage = null;
    if (S.tab === "nexus") renderMain();
  }
  const p = await attempt(() => api.nexusCollections(game, { ...q, installed: false }));
  if (S.gameId !== game) return;
  N.cpage = storePage(N.cpage, p, append);
  if (S.tab === "nexus") renderMain();
}

/// Shows the collections view's current state: checks the open collection again, or loads
/// the list when there is none yet.
export function refreshCollections() {
  const N = S.nexus;
  if (N.open?.game === S.gameId) reloadCollection();
  else if (!N.cpage) loadCollections();
}

/// Opens a collection as a checklist: its latest revision without `revision`.
export async function openCollection(slug, revision = null) {
  const game = S.gameId;
  const N = S.nexus;
  N.view = "collections";
  N.open = { slug, revision, data: null, game };
  renderMain();
  await reloadCollection();
}

/// Checks the open collection against what is installed again, showing the old checklist
/// until the new one arrives.
export async function reloadCollection() {
  const N = S.nexus;
  const o = N.open;
  if (!o) return;
  const data = await attempt(() => api.nexusCollection(o.game, o.slug, o.revision));
  if (N.open !== o) return;
  if (data) o.data = data;
  else if (!o.data) o.error = true;
  if (S.tab === "nexus") renderMain();
}

/// The files of the open collection UMM can get: the missing ones, and the optional ones
/// when they are included.
export function collectionTodo(c) {
  return c.files.filter((f) => f.state === "missing" && (S.nexus.optional || !f.optional));
}

/// Gets the collection's missing files one after another, each through the website. Files
/// with an installer, or whose place in the game folder UMM could not tell, wait in the mod
/// list instead of asking now.
export async function getCollectionFiles() {
  const N = S.nexus;
  const o = N.open;
  const c = o?.data;
  if (!c || N.walk) return;
  const todo = collectionTodo(c);
  const walk = { at: 0, total: todo.length, got: 0, waiting: 0, place: 0, skipped: 0 };
  N.walk = walk;
  let stopped = false;
  for (const f of todo) {
    if (N.walk !== walk || S.gameId !== o.game) break;
    walk.at++;
    walk.name = f.modName;
    renderMain();
    const label = f.fileName && f.fileName !== f.modName ? `${f.modName} (${f.fileName})` : f.modName;
    const r = await nexusGet(c.domain, f.modId, f.fileId, label, { quiet: true });
    if (r) {
      walk.got++;
      if (r.installerWaiting) walk.waiting++;
      else if (r.placementGuessed) walk.place++;
      continue;
    }
    if (walk.at >= walk.total) break;
    const v = await modal({
      title: `${f.modName} was not installed`,
      body: h("p", {}, `${plural(walk.total - walk.at, "file")} of the collection ${walk.total - walk.at === 1 ? "is" : "are"} still to get.`),
      actions: [
        { label: "Stop", value: null, class: "ghost" },
        { label: "Skip it", value: "skip", class: "primary", focus: true },
      ],
    });
    if (v !== "skip") {
      stopped = true;
      break;
    }
    walk.skipped++;
  }
  if (N.walk === walk) N.walk = null;
  const notes = [
    walk.waiting && `${plural(walk.waiting, "mod")} ${walk.waiting === 1 ? "needs its" : "need their"} install options chosen in the Mods tab`,
    walk.place && `${plural(walk.place, "mod")} ${walk.place === 1 ? "needs its" : "need their"} deployment location checked in the Mods tab`,
    walk.skipped && `${walk.skipped} skipped`,
  ].filter(Boolean);
  toast(`${stopped ? "Stopped: got" : "Got"} ${walk.got} of ${plural(walk.total, "file")} from ${c.name}${notes.length ? `; ${notes.join("; ")}` : ""}`, walk.got ? "ok" : "info");
  await refreshAll();
  await reloadCollection();
}

export async function loadActions() {
  loadLoaderChoice();
}

/// Loads the Nexus tab: the game's Nexus game, then a page of mods (from the top unless
/// `append`) and the installed mods' updates.
export async function loadNexus(reset, append = false) {
  const game = S.gameId;
  const N = S.nexus;
  if (reset || N.game?.game !== game) {
    N.game = null;
    N.page = null;
    N.updates = null;
    N.picker = null;
    // Another game's collections are not this one's.
    if (N.cgame !== game) {
      N.cgame = game;
      N.cpage = null;
      N.open = null;
      N.cquery.offset = 0;
    }
    renderMain();
    const g = await attempt(() => api.nexusGame(game));
    if (S.gameId !== game) return;
    N.game = g ? { ...g, game } : { domain: null, game, error: true };
    if (!N.game.domain) {
      renderMain();
      if (!g) return;
      return nexusFindGames(S.details.game.name);
    }
    if (N.view === "collections") refreshCollections();
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
  N.page = storePage(N.page, p, append);
  if (S.tab === "nexus") renderMain();
}

export async function nexusFindGames(name) {
  const game = S.gameId;
  S.nexus.picker = { name, results: null };
  renderMain();
  const r = await attempt(() => api.nexusFindGames(name));
  if (S.gameId === game && S.nexus.picker) {
    S.nexus.picker.results = r || [];
    renderMain();
  }
}

export async function nexusUseGame(domain) {
  const g = await attempt(() => api.setNexusDomain(S.gameId, domain), domain ? "Nexus Mods game saved" : "");
  if (g) loadNexus(true);
}

export function nexusPicker(g) {
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

export function nexusTab(d) {
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
  // Sets one search option and searches again.
  const set = (key) => (v) => {
    q[key] = v;
    loadNexus(false);
  };

  const header = [
    h(
      "div",
      { class: "seg", role: "group", "aria-label": "Show" },
      [
        ["mods", "Mods"],
        ["collections", "Collections"],
      ].map(([v, l]) =>
        h(
          "button",
          {
            class: N.view === v ? "on" : "",
            "aria-pressed": String(N.view === v),
            onclick: () => {
              if (N.view === v) return;
              N.view = v;
              if (v === "collections" && !N.cpage && !N.open) loadCollections();
              else renderMain();
            },
          },
          l,
        ),
      ),
    ),
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
  ];
  if (N.view === "collections") return collectionsView(g, header, locked, playing);

  const banners = storeBanners(N, g, playing);
  if (pending.length)
    banners.push(
      storeBanner(
        `${plural(pending.length, "update")}: ${pending.map((u) => `${u.name} ${u.version || ""} → ${u.newVersion}`).join(", ")}`,
        pending.length === 1 && h("button", { class: "small", disabled: locked, onclick: () => nexusGet(N.game.domain, pending[0].modId, pending[0].newFileId, pending[0].name) }, "Update"),
      ),
    );

  const row = (it) => {
    const up = updatesByMod[it.modId];
    return storeRow({
      image: it.thumbnail,
      imageClass: "nexus-thumb",
      name: it.name,
      site: "nexusmods.com",
      onOpen: () => attempt(() => api.openNexusPage(it.url)),
      by: it.author,
      version: it.version,
      text: it.summary,
      meta: [`${fmtCount(it.downloads)} downloads`, `${fmtCount(it.endorsements)} endorsements`, it.category, it.updatedAt && `updated ${it.updatedAt.slice(0, 10)}`],
      action: up
        ? h("button", { class: "small", disabled: locked, onclick: () => nexusGet(N.game.domain, it.modId, up[0].newFileId, it.name) }, `Update to ${up[0].newVersion}`)
        : it.installed.length
          ? h("div", { class: "row" }, h("span", { class: "chip ok" }, `Installed ${it.installed.join(", ")}`), h("button", { class: "ghost small", disabled: locked, onclick: () => nexusFilesDialog(it) }, "Files…"))
          : h("button", { class: "primary small", disabled: locked, onclick: () => nexusFilesDialog(it) }, "Download…"),
    });
  };

  return storeTab({
    header,
    toolbar: [
      storeSearch("Search mods", q, () => loadNexus(false)),
      storeSelect(
        "Sort",
        [
          ["endorsements", "Most endorsed"],
          ["downloads", "Most downloaded"],
          ["updated", "Recently updated"],
          ["name", "Name"],
        ],
        q.sort,
        set("sort"),
      ),
      storeCheck("Installed", q.installed, set("installed")),
      h("button", { class: "ghost small", onclick: () => loadNexus(true), title: "Load the list and updates again" }, "Refresh"),
    ],
    banners,
    count: p && !p.error && `${plural(p.total, "mod")} · downloads open in a UMM window on nexusmods.com · into the profile “${S.profileName}”`,
    list: storeList(p, { loading: "Loading mods…", noun: "mod", retry: () => loadNexus(false), row }),
    more: storeMore(p, () => {
      q.offset = p.items.length;
      loadNexus(false, true);
    }),
  });
}

/// The Nexus tab's collections: a search over the game's collections, or the one open as a
/// checklist.
export function collectionsView(g, header, locked, playing) {
  const N = S.nexus;
  if (N.open && N.open.game === g.id) return collectionChecklist(g, header, locked, playing);
  const q = N.cquery;
  const p = N.cpage;
  const row = (c) =>
    storeRow({
      image: c.tile,
      imageClass: "nexus-thumb",
      name: c.name,
      site: "nexusmods.com",
      onOpen: () => attempt(() => api.openNexusPage(c.url)),
      by: c.author,
      version: c.revision && `revision ${c.revision}`,
      text: c.summary,
      meta: [plural(c.modCount, "mod"), c.size && fmtBytes(c.size), `${fmtCount(c.downloads)} downloads`, `${fmtCount(c.endorsements)} endorsements`],
      action: h("button", { class: "small", onclick: () => openCollection(c.slug) }, "List mods…"),
    });
  return storeTab({
    header,
    toolbar: [
      storeSearch("Search collections, or paste a collection link", q, () => loadCollections()),
      storeSelect(
        "Sort",
        [
          ["endorsements", "Most endorsed"],
          ["downloads", "Most downloaded"],
          ["updated", "Recently updated"],
        ],
        q.sort,
        (v) => {
          q.sort = v;
          loadCollections();
        },
      ),
      h("button", { class: "ghost small", onclick: () => loadCollections(), title: "Load the list again" }, "Refresh"),
    ],
    banners: storeBanners(N, g, playing),
    count: p && !p.error && `${plural(p.total, "collection")} · UMM gets a collection’s mods one by one, each from its page on nexusmods.com`,
    list: storeList(p, { loading: "Loading collections…", noun: "collection", retry: () => loadCollections(), row }),
    more: storeMore(p, () => {
      q.offset = p.items.length;
      loadCollections(true);
    }),
  });
}

/// A collection as a checklist of its files: installed, installed at another version, or
/// missing, with a button that gets the missing ones in a row.
export function collectionChecklist(g, header, locked, playing) {
  const N = S.nexus;
  const o = N.open;
  const back = h(
    "button",
    {
      class: "ghost small",
      disabled: !!N.walk,
      onclick: () => {
        N.open = null;
        if (!N.cpage) loadCollections();
        else renderMain();
      },
    },
    "← Collections",
  );
  const c = o.data;
  if (!c)
    return storeTab({
      header,
      toolbar: [back],
      banners: [],
      list: o.error
        ? h("p", { class: "muted" }, "The collection could not be loaded.", " ", h("button", { class: "link", onclick: () => ((o.error = false), renderMain(), reloadCollection()) }, "Try again"))
        : h("div", { class: "loading" }, h("span", { class: "spinner" }), "Loading the collection…"),
    });
  const required = c.files.filter((f) => !f.optional);
  const optional = c.files.filter((f) => f.optional);
  const have = (fs) => fs.filter((f) => f.state === "installed").length;
  const todo = collectionTodo(c);
  const banners = storeBanners(N, g, playing);
  if (N.walk)
    banners.unshift(
      h(
        "div",
        { class: "ts-banner live" },
        h("span", {}, h("span", { class: "spinner" }), ` Getting ${N.walk.at} of ${N.walk.total}: ${N.walk.name || ""}`),
        h("button", { class: "small ghost", onclick: () => (N.walk = null), title: "Stop after the file being got now" }, "Stop after this one"),
      ),
    );
  const stateOf = (f) => {
    if (f.state === "installed") return h("span", { class: "chip ok" }, "Installed");
    const get = (text, title) =>
      h("button", { class: f.state === "missing" ? "primary small" : "small", disabled: locked || !!N.walk, title, onclick: () => nexusGet(c.domain, f.modId, f.fileId, f.modName) }, text);
    if (f.state === "other")
      return h("div", { class: "row" }, h("span", { class: "chip" }, `Installed ${f.installedVersion || "another file"}`), get(`Get ${f.version || "this file"}`, "Get the file this collection lists"));
    return get("Get…", "Download it on its page on nexusmods.com");
  };
  const row = (f) =>
    storeRow({
      image: f.thumbnail,
      imageClass: "nexus-thumb",
      name: f.modName,
      site: "nexusmods.com",
      onOpen: () => attempt(() => api.openNexusPage(f.url)),
      version: f.version,
      chips: f.optional ? h("span", { class: "chip" }, "Optional") : null,
      text: f.fileName !== f.modName ? f.fileName : "",
      meta: [f.size && fmtBytes(f.size)],
      action: stateOf(f),
      dim: f.state === "installed",
    });
  const list = (files) => h("div", { class: "ts-list" }, files.map(row));
  return storeTab({
    header,
    toolbar: [
      back,
      h("strong", { class: "collection-name" }, c.name),
      h("span", { class: "muted small" }, `revision ${c.revision}`),
      h("button", { class: "link", onclick: () => attempt(() => api.openNexusPage(c.url)) }, "Open on nexusmods.com", icon("external")),
      h("span", { class: "spacer" }),
      optional.length > 0 &&
        storeCheck("Include optional mods", N.optional, (v) => {
          N.optional = v;
          renderMain();
        }),
      h("button", { class: "ghost small", disabled: !!N.walk, onclick: () => reloadCollection(), title: "Check what is installed again" }, "Refresh"),
      h(
        "button",
        { class: "primary small", disabled: locked || !!N.walk || !todo.length, onclick: getCollectionFiles },
        todo.length ? `Get ${plural(todo.length, "missing file")}` : "Nothing missing",
      ),
    ],
    banners,
    count: [
      `${have(required)} of ${plural(required.length, "mod")} installed`,
      optional.length && `${have(optional)} of ${optional.length} optional`,
      `each file downloads on its page on nexusmods.com, into the profile “${S.profileName}”`,
    ]
      .filter(Boolean)
      .join(" · "),
    list: [
      list(required),
      optional.length > 0 && h("h3", { class: "collection-optional" }, "Optional"),
      optional.length > 0 && list(optional),
      h(
        "p",
        { class: "muted small" },
        "The collection’s installer choices, load order and extra instructions are on its page on nexusmods.com.",
      ),
    ],
  });
}

/// What a mod needs, in its files dialog. `onGet` closes the dialog to get one of them.
export function requirementList(reqs, onGet) {
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
export async function offerRequirements(name, missing) {
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

export async function nexusFilesDialog(it) {
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
export let cancelWaiting = null;

/// Shows a dialog while the file is downloaded on the website, in UMM's Nexus Mods window or
/// the user's browser. Resolves to `{ path }` (an archive to install), or null.
export function waitForDownload(dir, label) {
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
    const resumable = h("p", { class: "muted small" }, `A resumable download asks where to save the file: save it in ${dir}, where UMM looks for it, or choose the file here once it is done.`);
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
          how.textContent = "Its page is open in your browser. Click “Slow download” there; log in to Nexus Mods first if the page asks. A resumable download asks where to save the file: save it in your Downloads folder.";
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
      h("div", { class: "modal-body" }, how, waiting, resumable, note),
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
export async function nexusToast(r) {
  const v = r.version ? ` ${r.version}` : "";
  toast(r.installerWaiting ? `Downloaded ${r.name}${v}` : r.replaced ? `Updated ${r.name} to${v || " its newest file"}` : `Installed ${r.name}${v}`, "ok");
  await afterInstall({ id: r.id, name: r.name, installer: r.installerWaiting ? "waiting" : null, placement: { guessed: r.placementGuessed } }, !r.replaced);
}

/// Runs a Nexus install for the current game, with its progress at the top of the Nexus tab.
export async function nexusRun(title, call) {
  const r = await storeRun(S.nexus, title, call);
  await refreshAll();
  if (S.tab === "nexus") {
    if (S.nexus.view === "collections" && S.nexus.open) reloadCollection();
    else if (S.nexus.view === "collections") renderMain();
    else loadNexus(true);
  }
  return r;
}

/// Gets one file of a mod through the website: in UMM's Nexus Mods window, or the browser.
/// Resolves to what was installed, or null. `quiet`: no toast, and what the mod still needs
/// (its install options, where its files go) waits in the mod list instead of asking now.
export async function nexusGet(domain, modId, fileId, label, { quiet = false } = {}) {
  const game = S.gameId;
  const dir = await attempt(() => api.nexusDownloadStart(game, domain, modId, fileId));
  if (!dir) return null;
  // An archive that turns out to be something else (another download finished meanwhile, in
  // the Downloads folder) is passed over, and the wait goes on: the window keeps downloading.
  for (;;) {
    const got = await waitForDownload(dir, label);
    if (!got) {
      attempt(api.nexusDownloadCancel);
      return null;
    }
    const path = got.path;
    const file = path.split(/[\\/]/).pop();
    let r = await nexusRun("Installing", () => api.nexusDownloadInstall(S.profileName, path, false));
    if (r?.other) {
      toast(`Passed over ${file}: it is ${r.other} on Nexus Mods, not ${label}. Still waiting for ${label}.`, "info");
      continue;
    }
    if (r?.unrecognised) {
      const v = await modal({
        title: "Install this file?",
        body: h("p", {}, `Nexus Mods does not recognise ${file} by its contents, so UMM cannot check that it is ${label}. Install it as that file anyway?`),
        actions: [
          { label: "Cancel the download", value: "cancel", class: "ghost" },
          { label: "Not this file: keep waiting", value: "wait", class: "" },
          { label: "Install", value: "install", class: "primary", focus: true },
        ],
      });
      if (v === "wait") {
        await attempt(() => api.nexusDownloadPass(path));
        continue;
      }
      if (v !== "install") {
        attempt(api.nexusDownloadCancel);
        return null;
      }
      r = await nexusRun("Installing", () => api.nexusDownloadInstall(S.profileName, path, true));
    }
    if (r?.installed && !quiet) await nexusToast(r.installed);
    return r?.installed || null;
  }
}

import { api } from "../api.js";
import { afterInstall } from "../actions.js";
import { addFromSteam } from "../addgame.js";
import { refreshAll } from "../data.js";
import { $, fmtBytes, h, icon, plural } from "../dom.js";
import { confirmModal, modal } from "../modals.js";
import { header, tabs } from "../panel.js";
import { S, attempt, toast } from "../state.js";
import { actionsTab } from "./actions.js";
import { configTab, filesTab } from "./configs.js";
import { healthTab } from "./health.js";
import { modsTab } from "./mods.js";
import { nexusTab } from "./nexus.js";
import { storeBanner, storeBanners, storeCheck, storeList, storeMore, storePage, storeRow, storeRun, storeSearch, storeSelect, storeTab } from "./store.js";
import { fmtCount } from "./thunderstore.js";
import { tsTab, workshopTab } from "./workshop.js";

// ----- GameBanana tab -----

/// Loads the GameBanana list from the first page. `reset` also looks the game up again (and
/// its updates); `append` (Show more) adds the next page instead.
export async function loadGb(reset, append = false) {
  const game = S.gameId;
  const G = S.gb;
  if (reset || G.game?.game !== game) {
    G.game = null;
    G.page = null;
    G.updates = null;
    G.picker = null;
    renderMain();
    const g = await attempt(() => api.gbGame(game));
    if (S.gameId !== game) return;
    G.game = g ? { ...g, game } : { gameId: null, game, error: true };
    if (!G.game.gameId) {
      renderMain();
      if (!g) return;
      return gbFindGames(S.details.game.name);
    }
    attempt(() => api.gbUpdates(game)).then((u) => {
      if (S.gameId === game && u) {
        G.updates = u;
        if (S.tab === "gamebanana") renderMain();
      }
    });
  }
  if (!append) {
    G.query.page = 1;
    G.page = null;
    renderMain();
  }
  const q = { ...G.query };
  G.stale = false;
  const p = await attempt(() => api.gbSearch(game, q));
  if (S.gameId !== game) return;
  G.page = storePage(G.page, p, append);
  if (S.tab === "gamebanana") renderMain();
}

export async function gbFindGames(name) {
  const game = S.gameId;
  S.gb.picker = { name, results: null };
  renderMain();
  const r = await attempt(() => api.gbFindGames(name));
  if (S.gameId === game && S.gb.picker) {
    S.gb.picker.results = r || [];
    renderMain();
  }
}

export async function gbUseGame(gameId) {
  const g = await attempt(() => api.setGbGame(S.gameId, gameId), "GameBanana game saved");
  if (g) loadGb(true);
}

export function gbPicker(g) {
  const P = S.gb.picker || { name: g.name, results: null };
  const field = h("input", { type: "search", value: P.name, placeholder: "Game name", "aria-label": "Game name on GameBanana" });
  field.addEventListener("keydown", (e) => e.key === "Enter" && gbFindGames(field.value));
  return h(
    "section",
    { class: "panel" },
    h("h2", {}, "Which game is this on GameBanana?"),
    h("p", { class: "muted" }, `UMM did not find “${g.name}” on GameBanana by its name. Search for it and pick it; UMM saves the choice in this game’s game.toml.`),
    h("div", { class: "ts-toolbar" }, field, h("button", { onclick: () => gbFindGames(field.value) }, "Search")),
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
                h(
                  "span",
                  {},
                  h("strong", {}, r.name),
                  " ",
                  h("button", { class: "link small", onclick: () => attempt(() => api.openGbPage(r.url)) }, `gamebanana.com/games/${r.gameId}`, icon("external")),
                ),
                h("button", { class: "small", onclick: () => gbUseGame(r.gameId) }, "Use this game"),
              ),
            ),
          )
        : h("p", { class: "muted" }, "No games found by that name."),
  );
}

export const fmtDate = (unix) => (unix ? new Date(unix * 1000).toISOString().slice(0, 10) : "");

export function gbTab(d) {
  const g = d.game;
  const G = S.gb;
  if (!G.game || G.game.game !== g.id) return h("section", { class: "panel" }, h("div", { class: "loading" }, h("span", { class: "spinner" }), "Looking for this game on GameBanana…"));
  if (G.game.error)
    return h("section", { class: "panel" }, h("p", { class: "muted" }, "GameBanana could not be reached.", " ", h("button", { class: "link", onclick: () => loadGb(true) }, "Try again")));
  if (!G.game.gameId) return gbPicker(g);

  const playing = S.session.phase !== "idle" && S.session.game === g.id;
  const locked = playing || g.deployed || !!G.busy;
  const q = G.query;
  const p = G.page;
  const updatesByMod = {};
  for (const u of G.updates || []) if (u.state === "newer") (updatesByMod[u.modId] ||= []).push(u);
  const pending = (G.updates || []).filter((u) => u.state === "newer");
  const set = (key) => (v) => {
    q[key] = v;
    loadGb(false);
  };
  // Text search has only two orders: by popularity, or best match.
  const searching = q.text.trim() !== "" && !q.installed;

  const header = [
    h("span", { class: "muted small" }, `GameBanana game: ${G.game.name || G.game.gameId}`),
    h("button", { class: "link", onclick: () => attempt(() => api.openGbPage(G.game.url)) }, "Open on gamebanana.com", icon("external")),
    h(
      "button",
      {
        class: "link",
        onclick: () => {
          const name = G.game.name || g.name;
          G.game = { gameId: null, game: g.id };
          gbFindGames(name);
        },
      },
      "Change…",
    ),
  ];

  const banners = storeBanners(G, g, playing);
  if (pending.length)
    banners.push(
      storeBanner(
        `${plural(pending.length, "update")}: ${pending.map((u) => `${u.name} ${u.version || ""} → ${u.newVersion}`).join(", ")}`,
        pending.length === 1 && h("button", { class: "small", disabled: locked, onclick: () => gbGet(pending[0].modId, pending[0].newFileId) }, "Update"),
      ),
    );

  const row = (it) => {
    const up = updatesByMod[it.modId];
    return storeRow({
      image: it.thumbnail,
      imageClass: "nexus-thumb",
      name: it.name,
      site: "gamebanana.com",
      onOpen: () => attempt(() => api.openGbPage(it.url)),
      by: it.author,
      version: it.version,
      chips: [
        it.obsolete && h("span", { class: "chip warn" }, "Obsolete"),
        it.rated && h("span", { class: "chip", title: "Its author gave it content ratings (mature themes, violence, …)" }, "Rated"),
      ].filter(Boolean),
      text: it.category,
      meta: [`${fmtCount(it.likes)} likes`, `${fmtCount(it.views)} views`, it.updated && `updated ${fmtDate(it.updated)}`],
      dim: it.obsolete,
      action: up
        ? h("button", { class: "small", disabled: locked, onclick: () => gbGet(it.modId, up[0].newFileId) }, `Update to ${up[0].newVersion}`)
        : it.installed.length
          ? h("div", { class: "row" }, h("span", { class: "chip ok" }, `Installed ${it.installed.join(", ")}`), h("button", { class: "ghost small", disabled: locked, onclick: () => gbFilesDialog(it) }, "Files…"))
          : h("button", { class: "primary small", disabled: locked || !it.hasFiles, onclick: () => gbFilesDialog(it) }, "Install…"),
    });
  };

  return storeTab({
    header,
    toolbar: [
      storeSearch("Search mods", q, () => loadGb(false)),
      storeSelect(
        "Sort",
        searching
          ? [
              ["downloads", "Most popular"],
              ["name", "Best match"],
            ]
          : [
              ["downloads", "Most downloaded"],
              ["likes", "Most liked"],
              ["updated", "Recently updated"],
              ["newest", "Newest"],
              ["name", "Name"],
            ],
        searching && !["downloads", "likes"].includes(q.sort) ? "name" : q.sort,
        set("sort"),
      ),
      storeCheck("Installed", q.installed, set("installed")),
      h("button", { class: "ghost small", onclick: () => loadGb(true), title: "Load the list and updates again" }, "Refresh"),
    ],
    banners,
    count: p && !p.error && `${plural(p.total, "mod")} · downloads straight from gamebanana.com · into the profile “${S.profileName}”`,
    list: storeList(p, { loading: "Loading mods…", noun: "mod", retry: () => loadGb(false), row }),
    more:
      p &&
      !p.error &&
      storeMore({ ...p, total: p.complete ? p.items.length : p.items.length + 1 }, () => {
        q.page += 1;
        loadGb(false, true);
      }),
  });
}

/// A mod's files, to pick one and install it.
export async function gbFilesDialog(it) {
  const game = S.gameId;
  const f = await attempt(() => api.gbFiles(game, it.modId));
  if (!f) return;
  // Files for other platforms (Mac, Linux, ...) last, and not picked for you.
  const files_ = [...f.files.filter((x) => !x.platform), ...f.files.filter((x) => x.platform)];
  const first = files_.find((x) => !x.installed && !x.platform) || files_.find((x) => !x.platform) || files_[0];
  const row = (x) =>
    h(
      "label",
      { class: `nexus-file ${x.platform ? "old" : ""}` },
      h("input", { type: "radio", name: "gb-file", value: String(x.fileId), checked: x === first }),
      h(
        "span",
        {},
        h("strong", {}, x.description || x.name),
        x.platform && h("span", { class: "chip warn", title: `This file is for ${x.platform}, not this computer` }, `For ${x.platform}`),
        " ",
        h("span", { class: "muted small" }, [x.description && x.name, x.version, fmtBytes(x.size), fmtDate(x.added), `${fmtCount(x.downloads)} downloads`, x.installed ? "installed" : null].filter(Boolean).join(" · ")),
        x.clean === false && h("span", { class: "nexus-file-desc chip warn" }, "GameBanana’s virus scan flagged this file"),
        x.containsExe.length > 0 && h("span", { class: "nexus-file-desc muted small" }, `Contains programs: ${x.containsExe.join(", ")}`),
      ),
    );
  const files = h("div", { class: "nexus-files" }, files_.map(row));
  const body = [
    f.text && h("details", {}, h("summary", { class: "muted small" }, "Description"), h("p", { class: "gb-text small" }, f.text)),
    f.files.length ? files : h("p", { class: "muted" }, "This mod has no files."),
    f.files.length > 0 && h("p", { class: "muted small" }, "UMM downloads the file from gamebanana.com, checks it against GameBanana’s checksum, and installs it."),
    h("p", {}, h("button", { class: "link small", onclick: () => attempt(() => api.openGbPage(f.url)) }, "Open its page on gamebanana.com", icon("external"))),
  ].filter(Boolean);
  const v = await modal({
    title: f.name,
    body,
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      f.files.length > 0 && { label: "Install", value: "go", class: "primary", focus: true },
    ].filter(Boolean),
  });
  if (v !== "go") return;
  // The modal is gone; its radio buttons are still in hand.
  const picked = files.querySelector("input:checked");
  if (!picked) return;
  return gbGet(f.modId, Number(picked.value));
}

/// Downloads and installs one file of a mod, with its progress at the top of the tab.
export async function gbGet(modId, fileId) {
  const game = S.gameId;
  const r = await storeRun(S.gb, "Installing…", () => api.gbInstall(game, modId, fileId, S.profileName));
  await refreshAll();
  // Updates are checked again: the installed file may have been one.
  if (S.tab === "gamebanana" && S.gameId === game) loadGb(true);
  if (!r) return null;
  const v = r.version ? ` ${r.version}` : "";
  toast(r.replaced ? `Updated ${r.name} to${v || " its newest file"}` : `Installed ${r.name}${v}`, "ok");
  await afterInstall({ id: r.id, name: r.name, installer: r.installerWaiting ? "waiting" : null, placement: { guessed: r.placementGuessed } }, !r.replaced);
  return r;
}

export async function loadLoaderChoice() {
  const game = S.gameId;
  const c = await attempt(() => api.loaderChoice(game));
  if (c && S.gameId === game) {
    S.loader = { ...c, game };
    if (S.tab === "actions") renderMain();
  }
}

export const SETUP_HELP = {
  thunderstore: "Mods and the mod loader come from Thunderstore, laid out the way r2modman does.",
  bepinex: "BepInEx 5 or 6 through Doorstop, with a pack you import yourself as the mod “bepinex”.",
  generic: "No mod loader: mod files are copied into the game folder as they are.",
};

export function loaderAction(g, playing) {
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

export function action(title, text, label, fn, disabled, danger = false) {
  return h(
    "div",
    { class: "action" },
    h("div", {}, h("strong", {}, title), h("p", { class: "muted" }, text)),
    h("button", { class: danger ? "danger" : "", disabled, onclick: fn }, label),
  );
}

export function renderMain() {
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
        : S.tab === "gamebanana"
          ? gbTab(d)
        : S.tab === "workshop"
          ? workshopTab(d)
        : S.tab === "files"
          ? filesTab()
        : S.tab === "config"
          ? configTab(d)
          : S.tab === "health"
            ? healthTab()
            : actionsTab(d);
  $("#main").replaceChildren(header(d), tabs(), body);
}

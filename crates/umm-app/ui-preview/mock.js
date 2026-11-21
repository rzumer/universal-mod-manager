// In-memory stand-in for the Tauri backend, used only when ui/index.html is opened in an
// ordinary browser (python -m http.server from crates/umm-app, then /ui/index.html).
// Not part of the app: Tauri serves only the ui/ folder.

const listeners = {};
const emit = (event, payload) => (listeners[event] || []).forEach((cb) => cb(payload));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const games = {
  "example-unity-game": {
    id: "example-unity-game",
    name: "Example Unity Game",
    install: "D:\\SteamLibrary\\steamapps\\common\\Example Game",
    foundVia: "steam",
    deployed: false,
    running: false,
    canLaunch: true,
    loader: "bepinex",
    setup: "BepInEx from Thunderstore",
    community: "example-game",
    inject: "doorstop",
  },
  "example-asi-game": {
    id: "example-asi-game",
    name: "Example ASI Game",
    install: null,
    foundVia: null,
    deployed: false,
    running: false,
    canLaunch: true,
    loader: "asi",
    setup: "Loader from the game definition",
    community: null,
    inject: "files",
  },
};
games["example-unity-game"].thunderstore = true;
games["example-asi-game"].thunderstore = false;

// A small Thunderstore community. Icons are left out (the preview has no network).
const tsPackages = [
  { fullName: "BepInEx-BepInExPack", owner: "BepInEx", name: "BepInExPack", description: "BepInEx pack for Mono Unity games. Preconfigured and ready to use.", categories: ["Libraries"], rating: 1048, downloads: 34_438_007, latest: "5.4.2305", size: 600_000, deps: [] },
  { fullName: "Ann-Lantern_Tweaks", owner: "Ann", name: "Lantern_Tweaks", description: "Brighter, longer-lasting lanterns and a configurable flicker.", categories: ["Mods", "Client-side"], rating: 312, downloads: 1_204_331, latest: "1.2.0", size: 48_000, deps: ["BepInEx-BepInExPack", "Lib-CoreAPI"] },
  { fullName: "Lib-CoreAPI", owner: "Lib", name: "CoreAPI", description: "Shared networking and config helpers used by many mods.", categories: ["Libraries"], rating: 840, downloads: 9_874_002, latest: "2.0.1", size: 220_000, deps: ["BepInEx-BepInExPack"] },
  { fullName: "Bob-Quiet_Footsteps", owner: "Bob", name: "Quiet_Footsteps", description: "Makes footsteps quieter in the fog.", categories: ["Audio", "Client-side"], rating: 95, downloads: 402_118, latest: "0.4.0", size: 12_000, deps: ["BepInEx-BepInExPack"] },
  { fullName: "Old-Map_Pins", owner: "Old", name: "Map_Pins", description: "Pins on the map. Replaced by the base game's own pins.", categories: ["Mods"], rating: 40, downloads: 88_000, latest: "1.0.3", size: 30_000, deps: [], deprecated: true },
];
let linkOurs = false;
let pendingTaken = false;
const tsModId = (full) => full.toLowerCase().replace(/[^a-z0-9]+/g, "-");
const tsInstalledVersion = (game, full) => {
  const m = (mods[game] || []).find((x) => x.thunderstore === full);
  return m ? m.version : null;
};
const tsNewer = (a, b) => a.localeCompare(b, undefined, { numeric: true }) > 0;
function tsInstallOne(game, full) {
  const p = tsPackages.find((x) => x.fullName === full);
  const list = mods[game];
  const existing = list.find((x) => x.thunderstore === full);
  if (existing) existing.version = p.latest;
  else {
    const id = full === "BepInEx-BepInExPack" ? "bepinex" : tsModId(full);
    list.push({ id, name: p.name.replaceAll("_", " "), version: p.latest, files: 3, size: p.size, thunderstore: full });
    profiles[game].Default.mods.push({ id, enabled: true });
  }
}

// `?empty` previews the first-run screen.
if (new URLSearchParams(location.search).has("empty")) for (const k of Object.keys(games)) delete games[k];

const mods = {
  "example-unity-game": [
    { id: "bepinex", name: "BepInEx Pack", version: "5.4.2100", files: 42, size: 3_400_000, thunderstore: "BepInEx-BepInExPack" },
    { id: "better-ui", name: "Better UI", version: "2.1.0", files: 3, size: 220_000 },
    { id: "hd-textures", name: "HD Textures", version: "1.4", files: 312, size: 2_150_000_000 },
    { id: "quality-of-life", name: "Quality of Life Tweaks", version: "0.9.2", files: 5, size: 95_000 },
    { id: "old-map-mod", name: "Old Map Mod", version: "0.3", files: 2, size: 40_000 },
  ],
  "example-asi-game": [],
};

const profiles = {
  "example-unity-game": {
    Default: {
      name: "Default",
      mods: [
        { id: "bepinex", enabled: true },
        { id: "better-ui", enabled: true },
        { id: "hd-textures", enabled: true },
        { id: "quality-of-life", enabled: false },
      ],
    },
    Vanilla: { name: "Vanilla", mods: [{ id: "bepinex", enabled: true }] },
  },
  "example-asi-game": { Default: { name: "Default", mods: [] } },
};

const clone = (x) => JSON.parse(JSON.stringify(x));
const status = (g) => ({ ...g, modCount: mods[g.id].length });

const SETUP_LABELS = {
  thunderstore: "BepInEx from Thunderstore",
  bepinex: "BepInEx with your own pack",
  generic: "Generic (no mod loader)",
};
const setupOptions = (values) => values.map((value) => ({ value, label: SETUP_LABELS[value] }));
// Per game: the setup its definition says (null = a loader no setup covers), and one chosen in UMM.
const loaders = {
  "example-unity-game": { defined: "thunderstore", override: null, community: "example-game", otherLoader: null },
  "example-asi-game": { defined: null, override: null, community: null, otherLoader: null },
};
function loaderChoice(game) {
  const l = loaders[game];
  const current = l.defined;
  return {
    current,
    currentLabel: SETUP_LABELS[current] || "Set by hand in game.toml",
    available: setupOptions([...(l.community ? ["thunderstore"] : []), "bepinex", "generic"]),
    community: l.community,
    otherLoader: l.otherLoader,
  };
}

const handlers = {
  startup: () => ({
    dataDir: "C:\\Users\\you\\AppData\\Roaming\\umm",
    recovery: [],
    defErrors: [],
  }),
  recover_all: () => [],
  list_games: () => Object.values(games).map(status),
  game_details: ({ game }) => ({
    game: status(games[game]),
    profiles: Object.keys(profiles[game]).sort(),
    mods: mods[game].map((m) => ({ ...m, source: null, importedUnix: 0 })),
    dataDir: `C:\\Users\\you\\AppData\\Roaming\\umm\\games\\${game}`,
  }),
  get_profile: ({ game, profile }) => clone(profiles[game][profile]),
  save_profile: ({ game, profile }) => {
    profiles[game][profile.name] = clone(profile);
  },
  create_profile: ({ game, name, copyFrom }) => {
    if (profiles[game][name]) throw `a profile named \`${name}\` already exists`;
    const p = copyFrom ? clone(profiles[game][copyFrom]) : { mods: [] };
    p.name = name;
    profiles[game][name] = p;
    return p;
  },
  delete_profile: ({ game, profile }) => {
    delete profiles[game][profile];
  },
  preview_plan: ({ game, profile }) => {
    const on = profiles[game][profile].mods.filter((m) => m.enabled).map((m) => m.id);
    const files = [];
    if (on.includes("bepinex"))
      files.push(
        { target: "winhttp.dll", owner: "bepinex", writable: false },
        { target: "doorstop_config.ini", owner: "bepinex", writable: false },
        { target: "BepInEx/core/BepInEx.Preloader.dll", owner: "bepinex", writable: false },
      );
    if (on.includes("better-ui"))
      files.push({ target: "BepInEx/plugins/better-ui/BetterUI.dll", owner: "better-ui", writable: false });
    if (on.includes("hd-textures"))
      files.push({ target: "Example_Data/sharedassets0.assets", owner: "hd-textures", writable: false });
    if (on.includes("quality-of-life"))
      files.push({ target: "BepInEx/config/qol.cfg", owner: "quality-of-life", writable: true });
    const conflicts =
      on.includes("better-ui") && on.includes("hd-textures")
        ? [{ target: "Example_Data/ui.bundle", winner: "hd-textures", overridden: ["better-ui"] }]
        : [];
    if (conflicts.length) files.push({ target: "Example_Data/ui.bundle", owner: "hd-textures", writable: false });
    return {
      files,
      conflicts,
      skipped: on.includes("hd-textures")
        ? [{ owner: "hd-textures", file: "ExampleGame.exe", reason: "ExampleGame.exe is protected by the game definition" }]
        : [],
    };
  },
  pick_path: () => null,
  import_mod: () => {
    throw "The preview cannot open files. In the app, this opens a file picker.";
  },
  remove_mod: ({ game, id }) => {
    mods[game] = mods[game].filter((m) => m.id !== id);
  },
  set_install_path: () => {
    throw "The preview cannot pick folders.";
  },
  launch: ({ game }) => {
    (async () => {
      await sleep(600);
      emit("session", { kind: "deployed", game, gameFiles: 3, runtime: { placed: 4, unchanged: 38, removed: 0, keptEdited: [] } });
      await sleep(500);
      games[game].deployed = true;
      emit("session", { kind: "started", game, pid: 18244 });
      await sleep(6000);
      emit("session", { kind: "exited", game, code: 0 });
      await sleep(700);
      games[game].deployed = false;
      const report = { restored: 1, removed: 3, quarantined: [], keptGameVersions: [], dirsLeft: [], errors: [] };
      emit("session", { kind: "reverted", game, report });
      emit("session", { kind: "finished", game, abandoned: false, appeared: true, seconds: 7 });
    })();
  },
  stop_waiting: () => {},
  revert: () => null,
  doctor: ({ game }) =>
    games[game].install
      ? [
          { level: "ok", message: `installed at ${games[game].install} (found via steam)` },
          { level: "ok", message: "no deployment in progress" },
          { level: "info", message: "2 quarantined file(s) (files changed while deployed); review or delete them" },
          { level: "ok", message: `${mods[game].length} of ${mods[game].length} stored mod(s) match their recorded hashes` },
        ]
      : [{ level: "warn", message: "game not found; mods and profiles are kept. Set the folder with Change…" }],
  delete_game_data: ({ game }) => {
    delete games[game];
    delete mods[game];
    delete profiles[game];
  },
  open_folder: () => {},
  quit_anyway: () => {},
  steam_games: () => ({
    thunderstoreError: null,
    games: [
      { appId: 1001, name: "Example Unity Game", path: String.raw`D:\SteamLibrary\steamapps\common\Example Game`, engine: "Unity (Mono)", definedAs: "example-unity-game", community: "example-game", otherLoader: null, setups: [] },
      { appId: 1005, name: "Lantern Keepers", path: String.raw`D:\SteamLibrary\steamapps\common\LanternKeepers`, engine: "Unity (Mono)", definedAs: null, community: "lantern-keepers", otherLoader: null, setups: setupOptions(["thunderstore", "bepinex", "generic"]) },
      { appId: 1006, name: "Moss Garden", path: String.raw`D:\SteamLibrary\steamapps\common\MossGarden`, engine: "Unity (Mono)", definedAs: null, community: null, otherLoader: null, setups: setupOptions(["generic", "bepinex"]) },
      { appId: 1002, name: "Brickworks Tycoon", path: String.raw`D:\SteamLibrary\steamapps\common\Brickworks`, engine: "Unity (IL2CPP)", definedAs: null, community: null, otherLoader: "melonloader", setups: setupOptions(["generic"]) },
      { appId: 1007, name: "RV There Yet? — The Extremely Long Road Trip Edition: Director's Cut", path: String.raw`D:\SteamLibrary\steamapps\common\RV There Yet The Extremely Long Road Trip Edition`, engine: "Unreal Engine", definedAs: null, community: null, otherLoader: "shimloader", setups: setupOptions(["generic"]) },
      { appId: 1003, name: "Night Courier", path: String.raw`C:\Program Files (x86)\Steam\steamapps\common\NightCourier`, engine: "Unreal Engine", definedAs: null, community: null, otherLoader: null, setups: setupOptions(["generic"]) },
      { appId: 1004, name: "Pixel Harbor", path: String.raw`C:\Program Files (x86)\Steam\steamapps\common\PixelHarbor`, engine: "Unknown engine", definedAs: null, community: null, otherLoader: null, setups: setupOptions(["generic"]) },
    ],
  }),
  add_steam_games: ({ picks }) =>
    picks.map(({ appId, setup }) => {
      const g = handlers.steam_games().games.find((x) => x.appId === appId);
      const id = g.name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
      const label = SETUP_LABELS[setup];
      games[id] = { id, name: g.name, install: g.path, foundVia: "steam", deployed: false, running: false, canLaunch: true, loader: setup === "generic" ? null : "bepinex", inject: setup === "generic" ? null : "doorstop", setup: label, community: g.community };
      loaders[id] = { defined: setup, override: null, community: g.community, otherLoader: g.otherLoader };
      mods[id] = [];
      profiles[id] = { Default: { name: "Default", mods: [] } };
      const notes = {
        thunderstore: [{ key: "thunderstore", text: "This game is set up for BepInEx 5. You can download mods directly from the Thunderstore tab." }],
        bepinex: [{ key: "bepinex", text: "Set up for BepInEx 5 through Doorstop. Import the BepInEx 5 pack (5.4.22 or later) with the mod id `bepinex`." }],
        generic: {
          "Unity (Mono)": [{ key: "generic-unity", text: "Unity game set up as generic: mod files are copied as they are. To use BepInEx mods, change the mod loader in the Actions tab." }],
          "Unity (IL2CPP)": [{ key: "unity-il2cpp", text: "IL2CPP Unity game: its mods need BepInEx 6 or MelonLoader, which UMM does not support yet. Set up as generic: mod files are copied as they are." }],
          "Unreal Engine": [{ key: "unreal", text: "Unreal Engine game: loose .pak/.utoc/.ucas files go to NightCourier/Content/Paks/~mods/. Everything else keeps its path." }],
          "Unknown engine": [{ key: "unknown-engine", text: "Engine not recognised: mod files keep their paths relative to the game folder." }],
        }[g.engine],
      }[setup] || [];
      return { appId, ok: true, id, name: g.name, message: `Added ${g.name} (${label})`, notes };
    }),
  ts_search: ({ game, query }) => {
    const words = (query.text || "").toLowerCase().split(/\s+/).filter(Boolean);
    let hits = tsPackages
      .filter((p) => !p.deprecated || query.includeDeprecated || tsInstalledVersion(game, p.fullName))
      .filter((p) => !query.category || p.categories.includes(query.category))
      .filter((p) => !query.installedOnly || tsInstalledVersion(game, p.fullName))
      .filter((p) => words.every((w) => `${p.name} ${p.owner} ${p.description}`.toLowerCase().includes(w)));
    const by = { downloads: (a, b) => b.downloads - a.downloads, name: (a, b) => a.name.localeCompare(b.name) }[query.sort] || ((a, b) => b.rating - a.rating);
    hits = [...hits].sort(by);
    const items = hits.slice(query.offset, query.offset + query.limit).map((p) => {
      const installed = tsInstalledVersion(game, p.fullName);
      return { ...p, icon: "", website: "", updated: "", installed, update: !!installed && tsNewer(p.latest, installed), modId: tsModId(p.fullName), deprecated: !!p.deprecated };
    });
    const updates = tsPackages
      .map((p) => [p.fullName, tsInstalledVersion(game, p.fullName), p.latest])
      .filter(([, have, latest]) => have && tsNewer(latest, have));
    return {
      total: hits.length,
      items,
      categories: [...new Set(tsPackages.flatMap((p) => p.categories))].sort(),
      stale: false,
      loaderPackage: "BepInEx-BepInExPack",
      loaderInstalled: !!tsInstalledVersion(game, "BepInEx-BepInExPack"),
      // `?mismatch` previews a pack built for the other architecture.
      ...(new URLSearchParams(location.search).has("mismatch")
        ? {
            loaderWarning: "BepInExPack AmongUs is 32-bit, but Example Unity Game is 64-bit, so BepInEx will not load. You can try installing an unlisted BepInEx pack built for 64-bit.",
            loaderSuggestion: "BepInEx-BepInExPack_IL2CPP",
          }
        : { loaderWarning: null, loaderSuggestion: null }),
      updates,
    };
  },
  ts_refresh: () => true,
  ts_lookup: ({ name }) => {
    const listed = tsPackages.find((p) => p.fullName.toLowerCase() === name.toLowerCase());
    if (listed) return { package: { ...listed, icon: "", website: "", updated: "", installed: null, update: false, modId: tsModId(listed.fullName) }, listed: true, reviewStatus: null };
    if (name !== "BepInEx-BepInExPack_IL2CPP") throw `could not reach Thunderstore (https://thunderstore.io/api/experimental/package/${name.replace("-", "/")}/): 404 Not Found`;
    return {
      package: { fullName: name, owner: "BepInEx", name: "BepInExPack_IL2CPP", description: "BepInEx pack for IL2CPP x64 Unity games.", categories: [], rating: 5, downloads: 1_204_000, latest: "6.0.788", size: 0, icon: "", website: "", updated: "", installed: null, update: false, modId: tsModId(name), deprecated: false },
      listed: false,
      reviewStatus: "rejected",
    };
  },
  ts_install_unlisted: async ({ game, name }) => {
    emit("ts-progress", { game, message: "Downloading BepInExPack_IL2CPP 6.0.788" });
    await sleep(600);
    return { installed: [{ fullName: name, version: "6.0.788", replaced: null }], missing: [], warnings: [], notes: ["Turned off BepInEx-BepInExPack in this profile; BepInEx-BepInExPack_IL2CPP replaces it"] };
  },
  ts_install: async ({ game, packages }) => {
    const order = [];
    const visit = (full) => {
      const p = tsPackages.find((x) => x.fullName === full);
      p.deps.forEach(visit);
      if (!order.includes(full)) order.push(full);
    };
    packages.forEach((full) => visit(full.split("@")[0]));
    const todo = order.filter((f) => !tsInstalledVersion(game, f));
    const installed = [];
    for (const [i, full] of todo.entries()) {
      emit("ts-progress", { game, message: `Downloading ${full.split("-").slice(1).join("-")} (${i + 1} of ${todo.length})` });
      await sleep(600);
      tsInstallOne(game, full);
      installed.push({ fullName: full, version: tsPackages.find((p) => p.fullName === full).latest, replaced: null });
    }
    return { installed, missing: [] };
  },
  ts_update: async ({ game, packages }) => {
    const installed = [];
    for (const p of tsPackages) {
      const have = tsInstalledVersion(game, p.fullName);
      if (!have || !tsNewer(p.latest, have) || (packages.length && !packages.includes(p.fullName))) continue;
      emit("ts-progress", { game, message: `Downloading ${p.name} ${p.latest}` });
      await sleep(600);
      tsInstallOne(game, p.fullName);
      installed.push({ fullName: p.fullName, version: p.latest, replaced: have });
    }
    return { installed, missing: [] };
  },
  ts_dependents: ({ game, modId }) => {
    const m = mods[game].find((x) => x.id === modId);
    if (!m?.thunderstore) return [];
    return mods[game].filter((x) => x.thunderstore && tsPackages.find((p) => p.fullName === x.thunderstore)?.deps.includes(m.thunderstore)).map((x) => x.thunderstore);
  },
  profile_export_preview: ({ game, profile }) => {
    const ids = profiles[game][profile].mods.map((m) => m.id);
    const list = mods[game].filter((m) => ids.includes(m.id));
    return { mods: list.filter((m) => m.thunderstore).length, files: ["BepInEx/config/BepInEx.cfg", "BepInEx/config/lanterns.cfg"], leftOut: list.filter((m) => !m.thunderstore).map((m) => m.name), size: 4_812 };
  },
  profile_export_code: () => "01923b7e-5f2a-7c41-9d3e-8a6b1c2d3e4f",
  profile_export_file: ({ profile }) => String.raw`C:\Users\you\Documents\${profile}.r2z`,
  profile_import_code: async ({ game, code }) => {
    if (!code) throw "`` does not look like a profile code";
    emit("ts-progress", { game, message: "Downloading the profile" });
    await sleep(800);
    const name = `Friends${Object.keys(profiles[game]).includes("Friends") ? " (2)" : ""}`;
    profiles[game][name] = { name, mods: [{ id: "bepinex", enabled: true }, { id: "better-ui", enabled: true }] };
    return { profile: name, installed: { installed: [{ fullName: "Bob-Quiet_Footsteps", version: "0.4.0", replaced: null }], missing: [] }, notes: [], files: 3, skipped: [] };
  },
  profile_import_file: () => null,
  link_handler: () => ({ supported: true, ours: linkOurs, other: linkOurs ? null : String.raw`"C:\Users\you\AppData\Local\Programs\r2modman\r2modman.exe" "%1"` }),
  set_link_handler: ({ on }) => {
    linkOurs = on;
    return handlers.link_handler();
  },
  take_pending_link: () => {
    const l = new URLSearchParams(location.search).get("link");
    return pendingTaken ? null : ((pendingTaken = true), l);
  },
  resolve_link: ({ url }) => {
    const m = url.match(/^ror2mm:\/\/v1\/install\/thunderstore\.io\/([A-Za-z0-9_]+)\/([A-Za-z0-9_]+)\/?([0-9.]*)/);
    if (!m) throw `not a Thunderstore install link: ${url}`;
    const fullName = `${m[1]}-${m[2]}`;
    const known = tsPackages.some((p) => p.fullName === fullName);
    return { fullName, owner: m[1], name: m[2], version: m[3] || null, games: known ? [{ id: "example-unity-game", name: "Example Unity Game" }] : [] };
  },
  loader_choice: ({ game }) => loaderChoice(game),
  set_loader: ({ game, setup }) => {
    if (games[game].deployed) throw "mods are deployed; revert first";
    const l = loaders[game];
    l.defined = setup;
    games[game].setup = SETUP_LABELS[setup];
    games[game].loader = setup === "generic" ? null : "bepinex";
    games[game].thunderstore = setup === "thunderstore";
    return loaderChoice(game);
  },
  pick_game_folder: () => ({
    folder: String.raw`D:\Games\Lantern Keepers`,
    engine: "Unity (Mono)",
    name: "Lantern Keepers",
    exes: ["LanternKeepers.exe", "Launcher.exe"],
    community: "lantern-keepers",
    otherLoader: null,
    setups: setupOptions(["thunderstore", "bepinex", "generic"]),
    steamAppId: null,
    steamName: null,
  }),
  add_manual_game: ({ folder, name, exe, setup }) => {
    if (!name) throw "give the game a name";
    const id = name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
    if (games[id]) throw `a game named \`${id}\` already exists`;
    games[id] = { id, name, install: folder, foundVia: "manual", deployed: false, running: false, canLaunch: !!exe, loader: setup === "generic" ? null : "bepinex", inject: null, setup: SETUP_LABELS[setup], community: "lantern-keepers", thunderstore: setup === "thunderstore" };
    loaders[id] = { defined: setup, community: "lantern-keepers", otherLoader: null };
    mods[id] = [];
    profiles[id] = { Default: { name: "Default", mods: [] } };
    const notes = setup === "thunderstore" ? [{ key: "thunderstore", text: "This game is set up for BepInEx 5. You can download mods directly from the Thunderstore tab." }] : [];
    return { appId: 0, ok: true, id, name, message: `Added ${name} (${SETUP_LABELS[setup]})`, notes };
  },
};

export const backend = {
  async invoke(cmd, args = {}) {
    await sleep(120);
    const fn = handlers[cmd];
    if (!fn) throw `mock: no handler for ${cmd}`;
    return fn(args);
  },
  listen(event, cb) {
    (listeners[event] ||= []).push(cb);
    return Promise.resolve(() => {});
  },
  onDrop() {
    return Promise.resolve(() => {});
  },
};

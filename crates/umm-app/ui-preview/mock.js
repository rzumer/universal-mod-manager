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
const linkOurs = { ror2mm: false, nxm: false };
const linkOthers = {
  ror2mm: String.raw`"C:\Users\you\AppData\Local\Programs\r2modman\r2modman.exe" "%1"`,
  nxm: String.raw`"C:\Program Files\Black Tree Gaming Ltd\Vortex\Vortex.exe" -d "%1"`,
};
const q = new URLSearchParams(location.search);
// `?premium` starts signed in with a Premium account; `?nopick` makes the Unity game unknown
// to Nexus; `?unrecognised` makes downloads unknown by hash.
let nexusAccount = q.has("premium") ? { name: "Ann", premium: true } : null;
const nexusDomains = { "example-unity-game": null, "example-asi-game": null };
const nexusFiles = [
  { fileId: 4100, name: "Lantern Light", version: "2.0.1", category: "MAIN", size: 1_800_000, date: 300, description: "Everything in one archive.", old: false, group: 1 },
  { fileId: 4101, name: "Warm Colours", version: "2.0.1", category: "OPTIONAL", size: 220_000, date: 290, description: "Orange light instead of white.", old: false, group: 2 },
  { fileId: 3900, name: "Lantern Light", version: "1.9", category: "OLD_VERSION", size: 1_700_000, date: 200, description: null, old: true, group: 1 },
];
const nexusMods = [
  { modId: 77, name: "Lantern Light", summary: "Warmer, brighter lanterns that light the whole road.", author: "Ann", version: "2.0.1", endorsements: 914, downloads: 23_762, updatedAt: "2026-10-03T22:17:02Z", category: "Visuals" },
  { modId: 12, name: "Quieter Footsteps", summary: "Footsteps at half volume, so the music carries.", author: "Bob", version: "1.2", endorsements: 410, downloads: 9_120, updatedAt: "2025-03-14T00:07:39Z", category: "Audio" },
  { modId: 31, name: "Road Signs Reworked", summary: "Readable road signs with the real town names.", author: "Cy", version: "0.8", endorsements: 88, downloads: 1_204, updatedAt: "2024-07-06T14:59:11Z", category: "Visuals" },
  { modId: 45, name: "Lantern Oil Never Runs Out", summary: "For those who would rather not count oil.", author: "Dee", version: "1.0", endorsements: 52, downloads: 3_301, updatedAt: "2023-01-01T00:00:00Z", category: "Cheats" },
];
let nexusPending = null;
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
    { id: "better-ui", name: "Better UI", version: "2.1.0", files: 3, size: 220_000, placeable: true },
    { id: "hd-textures", name: "HD Textures", version: "1.4", files: 312, size: 2_150_000_000, placeable: true, placement: { root: "HD Textures 1.4", target: "Example_Data", skip: ["HD Textures 1.4/Nexus page.url", "HD Textures 1.4/README.txt"], guessed: false, chosen: false } },
    { id: "quality-of-life", name: "Quality of Life Tweaks", version: "0.9.2", files: 5, size: 95_000 },
    { id: "old-map-mod", name: "Old Map Mod", version: "0.3", files: 2, size: 40_000, placeable: true, placement: { root: "OldMap", target: "", skip: [], guessed: true, chosen: false } },
    { id: "lantern-light", name: "Lantern Light", version: "1.9", files: 6, size: 1_700_000, nexusUrl: "https://www.nexusmods.com/exampleunitygame/mods/77", nexusMod: 77, nexusFile: 3900, placeable: true },
  ],
  "example-asi-game": [],
};

/// What UMM suggests for each mod, fixed once first asked.
const modSuggestions = {};

/// Files inside some mods, for the "Where its files go" dialog.
const modFiles = {
  "hd-textures": ["HD Textures 1.4/sharedassets0.assets", "HD Textures 1.4/ui.bundle", "HD Textures 1.4/StreamingAssets/sky.bundle", "HD Textures 1.4/README.txt", "HD Textures 1.4/Nexus page.url"],
  "old-map-mod": ["OldMap/map.dat", "OldMap/icons/pin.png"],
  "better-ui": ["BetterUI/BetterUI.dll", "BetterUI/BetterUI.pdb", "BetterUI/CHANGELOG.md"],
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
        { id: "lantern-light", enabled: true },
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
  placement_view: ({ game, id, placement }) => {
    const m = mods[game].find((x) => x.id === id);
    const files = modFiles[id] || ["Mod/plugin.dll", "Mod/readme.txt"];
    const p = placement || m.placement || { root: "", target: "", skip: [], guessed: false, chosen: false };
    if (p.root && !files.some((f) => f.toLowerCase().startsWith(p.root.toLowerCase() + "/"))) throw `placement: the mod has no folder \`${p.root}\``;
    if (p.target.includes("..")) throw `placement: game folder \`${p.target}\`: the path leaves the folder it is relative to`;
    const folders = [...new Set(files.flatMap((f) => f.split("/").slice(0, -1).map((_, i, a) => a.slice(0, i + 1).join("/"))))].sort();
    const placed = files.map((f) => {
      const skipped = p.skip.some((s) => s.toLowerCase() === f.toLowerCase());
      const inside = !p.root || f.toLowerCase().startsWith(p.root.toLowerCase() + "/");
      if (skipped) return { file: f, target: null, note: "left out: documentation or similar", skipped: true };
      if (!inside) return { file: f, target: null, note: "outside the folder this mod installs from", skipped: false };
      const rel = p.root ? f.slice(p.root.length + 1) : f;
      return { file: f, target: p.target ? `${p.target.replace(/\/$/, "")}/${rel}` : rel, note: null, skipped: false };
    });
    const suggested = (modSuggestions[id] ??= { root: "", target: "", skip: [], guessed: false, chosen: false, ...(m.placement || {}), chosen: false });
    return { placement: { ...p }, suggested, folders, gameFolders: ["BepInEx", "BepInEx/plugins", "Example_Data", "Example_Data/StreamingAssets", "MonoBleedingEdge"], files: placed };
  },
  set_placement: ({ game, id, placement }) => {
    const m = mods[game].find((x) => x.id === id);
    m.placement = placement ? { ...placement, guessed: false, chosen: true } : { ...(modSuggestions[id] || m.placement), chosen: false };
    return { ...m, source: null, importedUnix: 0 };
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
  open_package_page: () => {},
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
      loaderName: "BepInEx",
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
  link_handler: () => ({ supported: true, ours: linkOurs.ror2mm && linkOurs.nxm, others: Object.keys(linkOurs).filter((k) => !linkOurs[k]).map((k) => linkOthers[k]) }),
  set_link_handler: ({ on }) => {
    linkOurs.ror2mm = linkOurs.nxm = on;
    return handlers.link_handler();
  },
  nexus_resolve_link: ({ url }) => {
    const m = url.match(/^nxm:\/\/([a-z0-9_-]+)\/mods\/(\d+)\/files\/(\d+)/i);
    if (!m) throw "not a Nexus Mods download link UMM can use";
    const f = nexusFiles.find((x) => x.fileId === Number(m[3]));
    nexusPending = { game: "example-unity-game", modId: Number(m[2]), fileId: Number(m[3]), at: Infinity };
    return {
      domain: m[1].toLowerCase(), modName: "Lantern Light", fileName: f?.name || null, version: f?.version || null,
      expired: /expires=1(&|$)/.test(url), signedIn: !!nexusAccount,
      games: Object.values(games).map((g) => ({ id: g.id, name: g.name, matches: g.id === "example-unity-game" })),
    };
  },
  nexus_install_link: async ({ game, profile }) => {
    const p = nexusPending;
    nexusPending = null;
    return nexusInstall(game, profile, p.modId, p.fileId);
  },
  nexus_account: () => nexusAccount,
  nexus_sign_in: ({ key }) => {
    if (key.trim().length < 8) throw "Nexus Mods did not accept the API key; check it, or create a new one";
    nexusAccount = { name: "Ann", premium: true };
    return nexusAccount;
  },
  nexus_sign_out: () => {
    nexusAccount = null;
    return null;
  },
  nexus_game: ({ game }) => {
    const d = nexusDomains[game];
    const guess = !d && game === "example-unity-game" && !q.has("nopick") ? "exampleunitygame" : null;
    const domain = d || guess;
    return { domain, name: domain ? (domain === "exampleunitygame" ? "Example Unity Game" : domain) : null, modCount: domain ? nexusMods.length : null, guessed: !!guess, url: domain ? `https://www.nexusmods.com/${domain}` : null };
  },
  nexus_find_games: ({ name }) =>
    [
      { domain: "exampleasigame", name: "Example ASI Game", modCount: 12 },
      { domain: "exampleasigame2", name: "Example ASI Game 2", modCount: 3 },
      { domain: "exampleunitygame", name: "Example Unity Game", modCount: 4 },
    ].filter((g) => g.name.toLowerCase().includes(name.toLowerCase().slice(0, 7))),
  set_nexus_domain: ({ game, domain }) => {
    const d = domain.trim().toLowerCase();
    if (d && !/^[a-z0-9_-]+$/.test(d)) throw `\`${d}\` is not a Nexus Mods game name (lower-case letters, digits, - and _)`;
    nexusDomains[game] = d || null;
    return handlers.nexus_game({ game });
  },
  nexus_search: ({ game, query }) => {
    let items = nexusMods.filter((m) => m.name.toLowerCase().includes(query.text.toLowerCase()));
    if (query.installed) items = items.filter((m) => mods[game].some((x) => x.nexusMod === m.modId));
    const by = { downloads: (m) => -m.downloads, updated: (m) => -Date.parse(m.updatedAt), name: (m) => m.name, endorsements: (m) => -m.endorsements }[query.sort];
    items = [...items].sort((a, b) => (by(a) < by(b) ? -1 : by(a) > by(b) ? 1 : 0));
    const domain = handlers.nexus_game({ game }).domain;
    return {
      total: items.length,
      items: items.slice(query.offset, query.offset + query.count).map((m) => ({
        ...m,
        thumbnail: null,
        url: `https://www.nexusmods.com/${domain}/mods/${m.modId}`,
        installed: mods[game].filter((x) => x.nexusMod === m.modId).map((x) => x.version),
      })),
    };
  },
  nexus_files: ({ game, modId }) => {
    const m = nexusMods.find((x) => x.modId === modId);
    const have = mods[game].filter((x) => x.nexusMod === modId).map((x) => x.nexusFile);
    const files = modId === 77 ? nexusFiles : [{ fileId: modId * 100, name: m.name, version: m.version, category: "MAIN", size: 150_000, date: 100, description: null, old: false }];
    const has = (id) => mods[game].some((x) => x.nexusMod === id);
    const requirements =
      modId === 31
        ? [
            { name: "Lantern Light", modId: 77, notes: "For the lamp-post signs", url: "https://www.nexusmods.com/exampleunitygame/mods/77", installed: has(77) },
            { name: "Quieter Footsteps", modId: 12, notes: null, url: "https://www.nexusmods.com/exampleunitygame/mods/12", installed: has(12) },
            { name: "Unity Explorer", modId: null, notes: "Any recent version", url: "https://example.org/unity-explorer", installed: false },
          ]
        : [];
    return { domain: handlers.nexus_game({ game }).domain, modId, name: m.name, files: files.map((f) => ({ ...f, installed: have.includes(f.fileId) })), requirements };
  },
  nexus_download_start: ({ game, modId, fileId }) => {
    nexusPending = { game, modId, fileId, at: Date.now() };
    return String.raw`C:\Users\you\Downloads`;
  },
  nexus_download_poll: () => (nexusPending && Date.now() - nexusPending.at > 2500 ? String.raw`C:\Users\you\Downloads\Lantern Light-77-2-0-1-300.zip` : null),
  nexus_download_cancel: () => {
    nexusPending = null;
    return null;
  },
  nexus_download_install: async ({ profile, path, trust }) => {
    if (!nexusPending) throw "no download is waiting";
    if (q.has("unrecognised") && !trust) return { installed: null, unrecognised: true };
    const p = nexusPending;
    nexusPending = null;
    return { installed: await nexusInstall(p.game, profile, p.modId, p.fileId), unrecognised: false };
  },
  nexus_install_file: ({ game, profile, modId, fileId }) => nexusInstall(game, profile, modId, fileId),
  nexus_updates: ({ game }) =>
    mods[game].filter((m) => m.nexusMod).map((m) => {
      const newer = m.nexusMod === 77 && m.nexusFile === 3900;
      return { id: m.id, name: m.name, modId: m.nexusMod, version: m.version, state: newer ? "newer" : "current", newFileId: newer ? 4100 : null, newVersion: newer ? "2.0.1" : null };
    }),
  nexus_update: ({ game, profile, modId }) => {
    if (!nexusAccount?.premium) throw "Nexus Mods only lets Premium accounts download from inside an app";
    return nexusInstall(game, profile, 77, 4100);
  },
  open_nexus_page: () => null,
  open_web_page: () => null,
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

async function nexusInstall(game, profile, modId, fileId) {
  const m = nexusMods.find((x) => x.modId === modId);
  const f = (modId === 77 ? nexusFiles : []).find((x) => x.fileId === fileId) || { fileId, name: m.name, version: m.version, category: "MAIN", size: 150_000, group: fileId };
  for (const step of ["Checking the file with Nexus Mods", "Unpacking"]) {
    emit("nexus-progress", { game, message: step });
    await sleep(600);
  }
  const main = f.category === "MAIN" || f.category === "OLD_VERSION";
  const id = main ? m.name.toLowerCase().replace(/[^a-z0-9]+/g, "-") : `${m.name} ${f.name}`.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  const list = mods[game];
  const existing = list.find((x) => x.id === id);
  if (existing) Object.assign(existing, { version: f.version, nexusFile: f.fileId });
  else {
    list.push({ id, name: main ? m.name : `${m.name} - ${f.name}`, version: f.version, files: 6, size: f.size, nexusUrl: `https://www.nexusmods.com/exampleunitygame/mods/${modId}`, nexusMod: modId, nexusFile: f.fileId });
    profiles[game][profile].mods.push({ id, enabled: true });
  }
  nexusDomains[game] ||= "exampleunitygame";
  return { id, name: existing?.name || m.name, version: f.version, replaced: !!existing };
}

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

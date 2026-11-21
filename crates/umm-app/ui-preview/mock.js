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
    loader: "bepinex5",
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
    inject: "files",
  },
};
// `?empty` previews the first-run screen.
if (new URLSearchParams(location.search).has("empty")) for (const k of Object.keys(games)) delete games[k];

const mods = {
  "example-unity-game": [
    { id: "bepinex", name: "BepInEx Pack", version: "5.4.23", files: 42, size: 3_400_000 },
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

const handlers = {
  startup: () => ({
    dataDir: "C:\\Users\\you\\AppData\\Roaming\\umm",
    gamedefsDir: "C:\\Users\\you\\AppData\\Roaming\\umm\\gamedefs",
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
  steam_games: () => [
    { appId: 1001, name: "Example Unity Game", path: String.raw`D:\SteamLibrary\steamapps\common\Example Game`, engine: "Unity (Mono)", definedAs: "example-unity-game" },
    { appId: 1002, name: "Brickworks Tycoon", path: String.raw`D:\SteamLibrary\steamapps\common\Brickworks`, engine: "Unity (IL2CPP)", definedAs: null },
    { appId: 1003, name: "Night Courier", path: String.raw`C:\Program Files (x86)\Steam\steamapps\common\NightCourier`, engine: "Unreal Engine", definedAs: null },
    { appId: 1004, name: "Pixel Harbor", path: String.raw`C:\Program Files (x86)\Steam\steamapps\common\PixelHarbor`, engine: "Unknown engine", definedAs: null },
  ],
  add_steam_games: ({ appIds }) =>
    appIds.map((appId) => {
      const g = handlers.steam_games().find((x) => x.appId === appId);
      const id = g.name.toLowerCase().replace(/[^a-z0-9]+/g, "-");
      games[id] = { id, name: g.name, install: g.path, foundVia: "steam", deployed: false, running: false, canLaunch: true, loader: null, inject: null };
      mods[id] = [];
      profiles[id] = { Default: { name: "Default", mods: [] } };
      const notes = {
        "Unity (IL2CPP)": [{ key: "unity-il2cpp", text: "IL2CPP Unity game: BepInEx 5 does not support it. Mods are deployed as plain files." }],
        "Unreal Engine": [{ key: "unreal", text: "Unreal Engine game: loose .pak/.utoc/.ucas files go to NightCourier/Content/Paks/~mods/. Everything else keeps its path." }],
        "Unknown engine": [{ key: "unknown-engine", text: "Engine not recognised: mod files keep their paths relative to the game folder." }],
      }[g.engine] || [];
      return { appId, ok: true, id, name: g.name, message: `Added ${g.name} (${g.engine})`, notes };
    }),
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

// In-memory stand-in for the Tauri backend, used only when ui/index.html is opened in an
// ordinary browser (python -m http.server from crates/umm-app, then /ui/index.html).
// Not part of the app: Tauri serves only the ui/ folder.

const listeners = {};
const emit = (event, payload) => (listeners[event] || []).forEach((cb) => cb(payload));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
/// A square icon in two colours, as a `data:` URL.
const mockIcon = (bg, fg) =>
  "data:image/svg+xml;base64," +
  btoa(`<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" rx="12" fill="${bg}"/><path d="M14 46 L28 24 L38 38 L44 30 L52 46 Z" fill="${fg}"/><circle cx="44" cy="20" r="6" fill="${fg}"/></svg>`);

const games = {
  "example-unity-game": {
    id: "example-unity-game",
    name: "Example Unity Game",
    install: "D:\\SteamLibrary\\steamapps\\common\\Example Game",
    foundVia: "steam",
    // `?deployed` leaves mods deployed, as after a crash.
    deployed: new URLSearchParams(location.search).has("deployed"),
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
// `?nopick` makes the Unity game unknown to Nexus; `?unrecognised` makes downloads unknown
// by hash.
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
// The Configs tab's files: a BepInEx config of each setting type, and a plain one.
const mockConfigs = {
  "BepInEx/config/BepInEx.cfg": {
    file: { path: "BepInEx/config/BepInEx.cfg", name: "BepInEx.cfg", version: null, guid: null, size: 4200, modifiedUnix: 1795400000, bepinex: true },
    sections: [
      { name: "Logging.Console", entries: [
        { key: "Enabled", value: "true", description: "Enables showing a console for log output.", kind: "Boolean", default: "false", acceptable: [], range: null },
        { key: "LogLevels", value: "Fatal, Error, Warning, Message, Info", description: "Only displays the specified log levels in the console output.", kind: "LogLevel", default: "Fatal, Error, Warning, Message, Info", acceptable: ["None", "Fatal", "Error", "Warning", "Message", "Info", "Debug", "All"], range: null },
      ] },
    ],
  },
  "BepInEx/config/com.example.betterui.cfg": {
    file: { path: "BepInEx/config/com.example.betterui.cfg", name: "Better UI", version: "2.1.0", guid: "com.example.betterui", size: 900, modifiedUnix: 1795410000, bepinex: true },
    sections: [
      { name: "General", entries: [
        { key: "Show Clock", value: "true", description: "Show the clock in the top right corner.", kind: "Boolean", default: "true", acceptable: [], range: null },
        { key: "Scale", value: "1.25", description: "How big the interface is.", kind: "Single", default: "1", acceptable: [], range: ["0.5", "2"] },
        { key: "Toggle Key", value: "F8", description: "Shows or hides the interface.", kind: "KeyboardShortcut", default: "F8", acceptable: [], range: null },
      ] },
      { name: "Colors", entries: [
        { key: "Theme", value: "Dark", description: "", kind: "Theme", default: "Dark", acceptable: ["Dark", "Light", "System"], range: null },
      ] },
    ],
  },
  "BepInEx/config/lantern/settings.json": {
    file: { path: "BepInEx/config/lantern/settings.json", name: "settings.json", version: null, guid: null, size: 60, modifiedUnix: 1795300000, bepinex: false },
    text: '{\n  "brightness": 1.4,\n  "warm": true\n}\n',
  },
};
function mockConfigView(path) {
  const c = mockConfigs[path];
  if (!c) throw `${path} is not a config file`;
  const text = c.sections
    ? c.sections.map((s) => `[${s.name}]\n\n` + s.entries.map((e) => `## ${e.description}\n# Setting type: ${e.kind}\n# Default value: ${e.default}\n${e.key} = ${e.value}\n`).join("\n")).join("\n")
    : c.text;
  return { file: { ...c.file }, text, sections: c.sections ? structuredClone(c.sections) : null };
}
const weiduState = {
  state: "needsChoices",
  // `?noweidu`: a mod comes without its installer, and no WeiDU is at hand.
  needsWeidu: q.has("noweidu"),
  installed: [],
  files: 0,
  output: "",
  error: null,
  ranUnix: 0,
  mods: [
    { modId: "ascension", setup: "setup-ascension.exe", chosen: false, components: [] },
    { modId: "tweaks-anthology", setup: "setup-cdtweaks.exe", chosen: false, components: [] },
  ],
};
const patcherState = { exe: "Patcher.exe", approved: false, state: "notRun", files: [], output: "", error: null, ranUnix: 0 };
const nexusCollections = [
  { slug: "lnt3rn", name: "Lantern Road", summary: "Brighter roads, quieter nights: the essentials for a first evening walk.", author: "Ann", endorsements: 703, downloads: 13_472, revision: 7, modCount: 5, size: 52_000_000, tile: null },
  { slug: "qu13t5", name: "Quiet Country", summary: "Calmer sounds everywhere.", author: "Bob", endorsements: 120, downloads: 2_210, revision: 2, modCount: 2, size: 300_000, tile: null },
];
// The files of "Lantern Road", in its order.
const collectionFiles = [
  { modId: 77, fileId: 4100, optional: false, modName: "Lantern Light", fileName: "Lantern Light", version: "2.0.1", size: 1_800_000 },
  { modId: 88, fileId: 8800, optional: false, modName: "Better Rocks", fileName: "Better Rocks", version: "2.0", size: 48_000_000 },
  { modId: 12, fileId: 1200, optional: false, modName: "Quieter Footsteps", fileName: "Quieter Footsteps", version: "1.2", size: 150_000 },
  { modId: 31, fileId: 3100, optional: false, modName: "Road Signs Reworked", fileName: "Road Signs Reworked", version: "0.8", size: 150_000 },
  { modId: 77, fileId: 4101, optional: true, modName: "Lantern Light", fileName: "Warm Colours", version: "2.0.1", size: 220_000 },
];
let pendingTaken = false;
const tsModId = (full) => full.toLowerCase().replace(/[^a-z0-9]+/g, "-");
const tsInstalledVersion = (game, full) => {
  const m = (mods[game] || []).find((x) => x.thunderstore === full);
  return m ? m.version : null;
};
const tsNewer = (a, b) => a.localeCompare(b, undefined, { numeric: true }) > 0;
/// The version a profile is pinned to for a package, if any.
function tsPin(game, profile, full) {
  const m = (mods[game] || []).find((x) => x.thunderstore === full);
  return (m && profiles[game][profile]?.mods.find((e) => e.id === m.id)?.version) || null;
}

function tsSetPin(game, profile, full, version) {
  const m = (mods[game] || []).find((x) => x.thunderstore === full);
  const e = m && profiles[game][profile]?.mods.find((x) => x.id === m.id);
  if (!e) return;
  if (version) e.version = version;
  else delete e.version;
}

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
    { id: "rock-textures", name: "Better Rocks", version: "2.0", files: 9, size: 48_000_000, placeable: true, installer: "waiting", nexusUrl: "https://www.nexusmods.com/exampleunitygame/mods/88", nexusMod: 88, nexusFile: 8800 },
    { id: "anniversary-patcher", name: "Anniversary Patcher", version: "1.5", files: 6, size: 6_600_000, placeable: true, patcher: "Patcher.exe" },
    ...(q.has("weidu")
      ? [
          { id: "ascension", name: "Ascension", version: "2.0.24", files: 120, size: 2_400_000, placeable: true, weidu: "setup-ascension.exe" },
          { id: "tweaks-anthology", name: "Tweaks Anthology", version: "v19", files: 900, size: 9_100_000, placeable: true, weidu: "setup-cdtweaks.exe" },
        ]
      : []),
    { id: "lantern-light", name: "Lantern Light", version: "1.9", files: 6, size: 1_700_000, nexusUrl: "https://www.nexusmods.com/exampleunitygame/mods/77", nexusMod: 77, nexusFile: 3900, placeable: true },
  ],
  "example-asi-game": [],
};

/// What UMM suggests for each mod, fixed once first asked.
const modSuggestions = {};

/// A FOMOD installer, run the way UMM runs one: steps, groups, options and their files.
const rockInstaller = {
  name: "Better Rocks",
  steps: [
    {
      name: "Resolution",
      groups: [
        {
          name: "Textures",
          kind: "exactly_one",
          options: [
            { name: "2K", description: "Sharper rocks, for cards with 4 GB of memory or more.\n\nRecommended for most players.", image: "Better Rocks/fomod/images/2k.png", kind: "recommended", files: ["textures/rock.dds", "textures/rock_n.dds"] },
            { name: "1K", description: "Lighter on memory.", image: null, kind: "optional", files: ["textures/rock.dds"] },
          ],
        },
      ],
    },
    {
      name: "Extras",
      visibleWith: "2K",
      groups: [
        {
          name: "Moss",
          kind: "any",
          options: [
            { name: "Moss", description: "Moss on the north side of rocks.", image: null, kind: "optional", files: ["textures/moss.dds"] },
            { name: "Lichen", description: "Needs the Lichen mod, which is not installed.", image: null, kind: "not_usable", files: [] },
          ],
        },
        {
          name: "Paths",
          kind: "at_most_one",
          options: [
            { name: "Dusty paths", description: "Dust between the rocks.", image: null, kind: "optional", files: ["textures/dust.dds"] },
            { name: "Muddy paths", description: "Mud between the rocks.", image: null, kind: "optional", files: ["textures/mud.dds"] },
          ],
        },
      ],
    },
  ],
};
let rockSaved = null;

function runRockInstaller(choices, target) {
  const pick = (s, g) => choices?.find((c) => c.step === s && c.group === g)?.plugins;
  let flag = null;
  const files = ["Rocks.esp"];
  const kept = [];
  const steps = rockInstaller.steps.map((s) => {
    const visible = !s.visibleWith || flag === s.visibleWith;
    const groups = s.groups.map((g) => {
      const given = pick(s.name, g.name);
      let sel = g.options.map((o) => o.kind !== "not_usable" && (given ? given.includes(o.name) : o.kind === "recommended"));
      if (!given && g.kind === "exactly_one" && !sel.some(Boolean)) sel[0] = true;
      const count = sel.filter(Boolean).length;
      const problem = visible && g.kind === "exactly_one" && count !== 1 ? "choose one option" : null;
      if (visible) {
        g.options.forEach((o, i) => sel[i] && files.push(...o.files));
        kept.push({ step: s.name, group: g.name, plugins: g.options.filter((_, i) => sel[i]).map((o) => o.name) });
      }
      return { name: g.name, kind: g.kind, problem, options: g.options.map((o, i) => ({ name: o.name, description: o.description, image: o.image, kind: o.kind, selected: visible && sel[i] })) };
    });
    if (s.name === "Resolution") flag = groups[0].options.find((o) => o.selected)?.name || null;
    return { name: s.name, visible, groups };
  });
  return {
    name: rockInstaller.name,
    image: null,
    steps,
    complete: steps.every((s) => s.groups.every((g) => !g.problem)),
    needsUnmet: false,
    files: [...new Set(files)].map((d) => ({ source: `Better Rocks/${d}`, destination: d })),
    missing: [],
    choices: kept,
    target: target ?? rockSaved?.target ?? "Example_Data",
    suggestedTarget: "Example_Data",
    gameFolders: ["BepInEx", "Example_Data", "Example_Data/StreamingAssets", "MonoBleedingEdge"],
  };
}

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
        { id: "rock-textures", enabled: true },
        { id: "anniversary-patcher", enabled: true },
        ...(q.has("weidu") ? [{ id: "ascension", enabled: true }, { id: "tweaks-anthology", enabled: true }] : []),
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
      // `?plugins` previews a game with a plugin list (a Bethesda game).
      plugins: q.has("plugins")
        ? {
            path: String.raw`C:\Users\you\AppData\Local\Skyrim Special Edition\plugins.txt`,
            kind: q.get("plugins") || "plugins",
            entries: [
              { name: "ccBGSSSE001-Fish.esm", enabled: true, owner: null },
              { name: "OldThing.esp", enabled: false, owner: null },
              { name: "BetterRocks.esm", enabled: true, owner: "rock-textures" },
              { name: "BetterRocks.esp", enabled: true, owner: "rock-textures" },
              { name: "LanternLight.esp", enabled: true, owner: "lantern-light" },
            ],
          }
        : null,
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
  config_files: ({ game }) => ({
    dir: String.raw`C:\Users\you\AppData\Roaming\umm\games\example-unity-game\runtime\Default`,
    files: game === "example-unity-game" && !q.has("noconfigs") ? Object.values(mockConfigs).map((c) => c.file) : [],
  }),
  read_config: ({ path }) => mockConfigView(path),
  set_config_values: ({ path, changes }) => {
    const c = mockConfigs[path];
    for (const ch of changes) {
      const e = c.sections.find((s) => s.name === ch.section)?.entries.find((x) => x.key === ch.key);
      if (!e) throw `this file has no setting ${ch.key} in [${ch.section}]`;
      e.value = ch.value;
    }
    c.file.modifiedUnix = Math.floor(Date.now() / 1000);
    return mockConfigView(path);
  },
  set_config_text: ({ path, text }) => {
    mockConfigs[path].text = text;
    return mockConfigView(path);
  },
  weidu_status: () => (q.has("weidu") ? structuredClone(weiduState) : null),
  weidu_components: async ({ id, language }) => {
    await sleep(500);
    const lang = language ?? weiduState.mods.find((m) => m.modId === id)?.language ?? 0;
    const fr = lang === 1 ? " (fr)" : "";
    const components =
      id === "ascension"
        ? [[0, `Ascension (core)${fr}`], [10, `Tougher Irenicus${fr}`], [20, `Final Battle -> Original Ending${fr}`], [21, `Final Battle -> Extended Ending${fr}`]]
        : [[1, `Remove Helmet Animations${fr}`], [2050, `Higher HP on Level Up${fr}`], [3000, `Stores -> Sell Everything${fr}`], [3001, `Stores -> Buy Everything${fr}`]];
    const m = weiduState.mods.find((x) => x.modId === id);
    return {
      languages: [[0, "English"], [1, "Francais"]],
      language: lang,
      components,
      chosen: m?.chosen ? { language: m.language ?? 0, components: m.components.map(([n]) => n) } : null,
    };
  },
  set_weidu_choice: ({ id, language, components }) => {
    const m = weiduState.mods.find((x) => x.modId === id);
    Object.assign(m, { chosen: true, language, components });
    if (weiduState.state === "current") weiduState.state = "outdated";
    else if (weiduState.state === "needsChoices") weiduState.state = "notRun";
    return structuredClone(weiduState);
  },
  set_weidu_exe: () => {
    weiduState.needsWeidu = false;
    return null;
  },
  run_weidu: async () => {
    await sleep(1200);
    Object.assign(weiduState, {
      installed: weiduState.mods.filter((m) => m.chosen).map((m) => m.modId),
      state: "current",
      files: 214,
      error: null,
      ranUnix: Math.floor(Date.now() / 1000),
      output: "=== Ascension (setup-ascension.exe) ===\nSUCCESSFULLY INSTALLED      Ascension (core)\n=== Tweaks Anthology (setup-cdtweaks.exe) ===\nSUCCESSFULLY INSTALLED      Remove Helmet Animations\nSUCCESSFULLY INSTALLED      Higher HP on Level Up",
    });
    return structuredClone(weiduState);
  },
  patcher_status: ({ id }) => (id === "anniversary-patcher" ? { ...patcherState } : null),
  run_patcher: async () => {
    await sleep(900);
    if (q.has("patchfail")) {
      Object.assign(patcherState, { approved: true, state: "failed", error: "it changed none of the game's files (an unsupported game version?)", output: "Invalid executable", ranUnix: Math.floor(Date.now() / 1000) });
      throw "Patcher.exe did not patch the game: it changed none of the game's files (an unsupported game version?)";
    }
    Object.assign(patcherState, { approved: true, state: "current", files: ["Fallout3.exe"], output: "Patched Fallout3.exe (1.7.0.4 -> 1.7.0.3)", error: null, ranUnix: Math.floor(Date.now() / 1000) });
    return { ...patcherState };
  },
  fomod_view: ({ choices, target }) => runRockInstaller(choices ?? rockSaved?.choices, target),
  fomod_apply: ({ game, id, choices, target }) => {
    const v = runRockInstaller(choices, target);
    if (!v.complete) throw "every shown group needs its options chosen first";
    rockSaved = { choices: v.choices, target };
    const m = mods[game].find((x) => x.id === id);
    m.installer = "done";
    return { ...m, source: null, importedUnix: 0 };
  },
  // A tiny picture, standing in for an option's image.
  // A drawn icon for the example game and the mods from Thunderstore or Nexus Mods; none for
  // the rest, which show a letter.
  game_icon: async ({ game }) => (game === "example-unity-game" ? mockIcon("#2563eb", "#93c5fd") : null),
  mod_icon: async ({ id }) => {
    await sleep(150);
    return { bepinex: mockIcon("#7c3aed", "#ddd6fe"), "lantern-light": mockIcon("#d97706", "#fde68a"), "rock-textures": mockIcon("#57534e", "#d6d3d1") }[id] || null;
  },
  mod_image: () =>
    "data:image/svg+xml;base64," +
    btoa('<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="320" height="180" fill="#6b7280"/><circle cx="110" cy="120" r="50" fill="#9ca3af"/><circle cx="210" cy="110" r="60" fill="#4b5563"/></svg>'),
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
  revert: ({ game }) => {
    if (!games[game].deployed) return null;
    games[game].deployed = false;
    return { restored: 1, removed: 4, quarantined: [], keptGameVersions: [], dirsLeft: [], errors: [] };
  },
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
  // Versions, as the profile sees them: the latest and a few before it; for BepInEx, the
  // oldest is too old for a mod.
  ts_versions: ({ game, name, profile }) => {
    const p = tsPackages.find((x) => x.fullName === name);
    const latest = tsInstalledVersion(game, name);
    const pin = tsPin(game, profile, name);
    const list =
      name === "BepInEx-BepInExPack" ? ["5.4.2305", "5.4.2100", "5.4.2000", "5.4.1900"] : [p.latest, "1.1.0", "1.0.0"].filter((v, i, a) => a.indexOf(v) === i);
    return list.map((version) => ({
      version,
      size: p.size,
      installed: version === (pin || latest),
      pinned: version === pin,
      stored: version === latest || version === pin,
      neededBy: name === "BepInEx-BepInExPack" && version === "5.4.1900" ? [{ name: "Ann-Lantern_Tweaks", version: "5.4.2000" }] : [],
    }));
  },
  // Pins the profile; the version BepInEx is too old for asks first, then downgrades the mod.
  ts_install_version: async ({ game, name, version, profile, choice }) => {
    if (name === "BepInEx-BepInExPack" && version === "5.4.1900" && !choice)
      return {
        installed: [],
        missing: [],
        warnings: [],
        notes: [],
        conflicts: [{ dependent: "Ann-Lantern_Tweaks", dependentVersion: "1.2.0", pinned: name, pin: version, needs: "5.4.2000", canUnpin: false }],
      };
    emit("ts-progress", { game, message: `Downloading ${name.split("-").slice(1).join("-")} ${version}` });
    await sleep(600);
    const notes = [];
    tsSetPin(game, profile, name, version);
    if (choice === "downgrade") {
      tsSetPin(game, profile, "Ann-Lantern_Tweaks", "1.0.0");
      notes.push("Pinned Ann-Lantern_Tweaks at 1.0.0, which works with this profile's pins");
    }
    return { installed: [{ fullName: name, version, replaced: null }], missing: [], warnings: [], notes, conflicts: [] };
  },
  // Stated dependencies (the sample packages' deps) the profile does not meet.
  ts_dependency_issues: ({ game, profile }) => {
    const p = profiles[game]?.[profile];
    const list = mods[game] || [];
    const issues = [];
    for (const e of p?.mods || []) {
      const m = list.find((x) => x.id === e.id);
      if (!e.enabled || !m?.thunderstore) continue;
      for (const dep of tsPackages.find((x) => x.fullName === m.thunderstore)?.deps || []) {
        const needs = tsPackages.find((x) => x.fullName === dep)?.latest || "1.0.0";
        const dm = list.find((x) => x.thunderstore === dep);
        const de = dm && p.mods.find((x) => x.id === dm.id);
        const problem = !dm ? "notInstalled" : !de ? "notInProfile" : !de.enabled ? "disabled" : null;
        if (problem) issues.push({ modId: e.id, dependency: dep, needs, problem, have: null });
      }
    }
    return issues;
  },
  ts_follow_latest: async ({ game, name, profile }) => {
    await sleep(300);
    tsSetPin(game, profile, name, null);
    return { installed: [], missing: [], warnings: [], notes: [], conflicts: [] };
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
    return {
      domain: m[1].toLowerCase(), modName: "Lantern Light", fileName: f?.name || null, version: f?.version || null,
      modId: Number(m[2]), fileId: Number(m[3]),
      games: Object.values(games).map((g) => ({ id: g.id, name: g.name, matches: g.id === "example-unity-game" })),
    };
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
  nexus_collections: ({ game, query }) => {
    const domain = handlers.nexus_game({ game }).domain;
    let items = nexusCollections.filter((c) => c.name.toLowerCase().includes(query.text.toLowerCase()));
    const by = { downloads: (c) => -c.downloads, updated: (c) => -c.revision, endorsements: (c) => -c.endorsements }[query.sort];
    items = [...items].sort((a, b) => by(a) - by(b));
    return { total: items.length, items: items.slice(query.offset, query.offset + query.count).map((c) => ({ ...c, url: `https://www.nexusmods.com/games/${domain}/collections/${c.slug}` })) };
  },
  nexus_collection: ({ game, slug, revision }) => {
    const c = nexusCollections.find((x) => x.slug === slug);
    if (!c) throw `Nexus Mods has no collection ${slug}`;
    const domain = handlers.nexus_game({ game }).domain;
    const files = (slug === "lnt3rn" ? collectionFiles : collectionFiles.slice(2, 4)).map((f) => {
      const same = mods[game].find((x) => x.nexusFile === f.fileId);
      const other = !same && !f.optional && mods[game].find((x) => x.nexusMod === f.modId && !collectionFiles.some((c) => c.fileId === x.nexusFile));
      return {
        ...f,
        thumbnail: null,
        url: `https://www.nexusmods.com/${domain}/mods/${f.modId}`,
        state: same ? "installed" : other ? "other" : "missing",
        installedId: (same || other)?.id || null,
        installedVersion: other ? other.version : null,
      };
    });
    return { slug, name: c.name, revision: revision || c.revision, domain, url: `https://www.nexusmods.com/games/${domain}/collections/${slug}`, files };
  },
  nexus_collection_link: ({ url }) => {
    const m = url.match(/^(?:nxm:\/\/|https:\/\/(?:www\.|next\.)?nexusmods\.com\/(?:games\/)?)([a-z0-9_-]+)\/collections\/([a-z0-9]+)(?:\/revisions\/(\d+))?/i);
    if (!m) return null;
    return { domain: m[1].toLowerCase(), slug: m[2], revision: m[3] ? Number(m[3]) : null, games: Object.values(games).map((g) => ({ id: g.id, name: g.name, matches: g.id === "example-unity-game" })) };
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
  nexus_download_poll: () => {
    // `?closewindow` previews the user closing the Nexus Mods window.
    if (nexusPending && q.has("closewindow") && !nexusPending.reopened) return { path: null, failed: false, closed: Date.now() - nexusPending.at > 1200 };
    if (!nexusPending || Date.now() - nexusPending.at < 2500) return { path: null, failed: false, closed: false };
    return { path: String.raw`C:\Users\you\AppData\Local\Temp\umm-downloads\1\Lantern Light-77-2-0-1-300.zip`, failed: false, closed: false };
  },
  nexus_download_reopen: () => {
    if (nexusPending) Object.assign(nexusPending, { reopened: true, at: Date.now() });
    return null;
  },
  nexus_download_in_browser: () => {
    if (nexusPending) nexusPending.at = Date.now();
    return null;
  },
  nexus_download_cancel: () => {
    nexusPending = null;
    return null;
  },
  nexus_download_pass: () => null,
  nexus_download_install: async ({ profile, path, trust }) => {
    if (!nexusPending) throw "no download is waiting";
    if (q.has("unrecognised") && !trust) return { installed: null, unrecognised: true };
    // `?othermod`: the first archive found is another mod's, passed over.
    if (q.has("othermod") && !nexusPending.passed) {
      nexusPending.passed = true;
      return { installed: null, unrecognised: false, other: "Fallout Script Extender (FOSE)" };
    }
    const p = nexusPending;
    nexusPending = null;
    return { installed: await nexusInstall(p.game, profile, p.modId, p.fileId), unrecognised: false };
  },
  nexus_updates: ({ game }) =>
    mods[game].filter((m) => m.nexusMod).map((m) => {
      const newer = m.nexusMod === 77 && m.nexusFile === 3900;
      return { id: m.id, name: m.name, modId: m.nexusMod, version: m.version, state: newer ? "newer" : "current", newFileId: newer ? 4100 : null, newVersion: newer ? "2.0.1" : null };
    }),
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
  // `?installer` previews a downloaded mod with a FOMOD installer.
  const waiting = q.has("installer") && !existing;
  if (waiting) Object.assign(list.find((x) => x.id === id), { installer: "waiting", placeable: true });
  return { id, name: existing?.name || m.name, version: f.version, replaced: !!existing, installerWaiting: waiting };
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

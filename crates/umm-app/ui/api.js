// Thin wrapper over Tauri's global API. When the page is opened in an ordinary browser
// (for UI work), a mock backend stands in; it is never loaded inside the app.

const tauri = window.__TAURI__;
let backend;

if (tauri) {
  backend = {
    invoke: (cmd, args) => tauri.core.invoke(cmd, args),
    listen: (event, cb) => tauri.event.listen(event, (e) => cb(e.payload)),
    onDrop: (cb) =>
      tauri.webview.getCurrentWebview().onDragDropEvent((e) => cb(e.payload)),
    isMock: false,
  };
} else {
  const mock = await import("../ui-preview/mock.js");
  backend = { ...mock.backend, isMock: true };
}

export const api = {
  isMock: backend.isMock,
  listen: backend.listen,
  onDrop: backend.onDrop,
  startup: () => backend.invoke("startup"),
  recoverAll: () => backend.invoke("recover_all"),
  listGames: () => backend.invoke("list_games"),
  gameDetails: (game) => backend.invoke("game_details", { game }),
  getProfile: (game, profile) => backend.invoke("get_profile", { game, profile }),
  saveProfile: (game, profile) => backend.invoke("save_profile", { game, profile }),
  createProfile: (game, name, copyFrom) =>
    backend.invoke("create_profile", { game, name, copyFrom }),
  deleteProfile: (game, profile) => backend.invoke("delete_profile", { game, profile }),
  previewPlan: (game, profile) => backend.invoke("preview_plan", { game, profile }),
  pickPath: (kind) => backend.invoke("pick_path", { kind }),
  fomodView: (game, id, choices, target) => backend.invoke("fomod_view", { game, id, choices, target }),
  fomodApply: (game, id, choices, target) => backend.invoke("fomod_apply", { game, id, choices, target }),
  modImage: (game, id, path) => backend.invoke("mod_image", { game, id, path }),
  placementView: (game, id, placement) => backend.invoke("placement_view", { game, id, placement }),
  setPlacement: (game, id, placement) => backend.invoke("set_placement", { game, id, placement }),
  importMod: (game, path, profile, id) =>
    backend.invoke("import_mod", { game, path, profile, id: id || null }),
  removeMod: (game, id) => backend.invoke("remove_mod", { game, id }),
  setInstallPath: (game, path) => backend.invoke("set_install_path", { game, path }),
  launch: (game, profile) => backend.invoke("launch", { game, profile }),
  stopWaiting: () => backend.invoke("stop_waiting"),
  revert: (game, discardIfMissing = false) =>
    backend.invoke("revert", { game, discardIfMissing }),
  doctor: (game) => backend.invoke("doctor", { game }),
  deleteGameData: (game) => backend.invoke("delete_game_data", { game }),
  openFolder: (path) => backend.invoke("open_folder", { path }),
  openPackagePage: (game, fullName) => backend.invoke("open_package_page", { game, fullName }),
  quitAnyway: () => backend.invoke("quit_anyway"),
  steamGames: () => backend.invoke("steam_games"),
  // picks: [{ appId, setup }] where setup is "thunderstore" | "bepinex" | "generic" | null
  addSteamGames: (picks) => backend.invoke("add_steam_games", { picks }),
  loaderChoice: (game) => backend.invoke("loader_choice", { game }),
  setLoader: (game, setup) => backend.invoke("set_loader", { game, setup }),
  pickGameFolder: () => backend.invoke("pick_game_folder"),
  addManualGame: (folder, name, exe, setup) => backend.invoke("add_manual_game", { folder, name, exe, setup }),
  tsSearch: (game, query) => backend.invoke("ts_search", { game, query }),
  tsRefresh: (game) => backend.invoke("ts_refresh", { game }),
  // packages: "Owner-Name" or "Owner-Name@1.2.3"
  tsInstall: (game, packages, profile) => backend.invoke("ts_install", { game, packages, profile }),
  tsUpdate: (game, packages, profile) => backend.invoke("ts_update", { game, packages, profile }),
  tsDependents: (game, modId) => backend.invoke("ts_dependents", { game, modId }),
  tsLookup: (game, name) => backend.invoke("ts_lookup", { game, name }),
  tsInstallUnlisted: (game, name, version, profile) =>
    backend.invoke("ts_install_unlisted", { game, name, version: version || null, profile }),
  exportPreview: (game, profile) => backend.invoke("profile_export_preview", { game, profile }),
  exportCode: (game, profile) => backend.invoke("profile_export_code", { game, profile }),
  exportFile: (game, profile) => backend.invoke("profile_export_file", { game, profile }),
  importCode: (game, code) => backend.invoke("profile_import_code", { game, code }),
  importFile: (game) => backend.invoke("profile_import_file", { game }),
  linkHandler: () => backend.invoke("link_handler"),
  setLinkHandler: (on) => backend.invoke("set_link_handler", { on }),
  nexusGame: (game) => backend.invoke("nexus_game", { game }),
  nexusFindGames: (name) => backend.invoke("nexus_find_games", { name }),
  setNexusDomain: (game, domain) => backend.invoke("set_nexus_domain", { game, domain }),
  nexusSearch: (game, query) => backend.invoke("nexus_search", { game, query }),
  nexusFiles: (game, modId) => backend.invoke("nexus_files", { game, modId }),
  nexusDownloadStart: (game, domain, modId, fileId) => backend.invoke("nexus_download_start", { game, domain, modId, fileId }),
  nexusDownloadPoll: () => backend.invoke("nexus_download_poll"),
  nexusDownloadCancel: () => backend.invoke("nexus_download_cancel"),
  nexusDownloadInBrowser: () => backend.invoke("nexus_download_in_browser"),
  nexusDownloadReopen: () => backend.invoke("nexus_download_reopen"),
  nexusDownloadInstall: (profile, path, trust) => backend.invoke("nexus_download_install", { profile, path, trust }),
  nexusResolveLink: (url) => backend.invoke("nexus_resolve_link", { url }),
  nexusUpdates: (game) => backend.invoke("nexus_updates", { game }),
  openNexusPage: (url) => backend.invoke("open_nexus_page", { url }),
  openWebPage: (url) => backend.invoke("open_web_page", { url }),
  takePendingLink: () => backend.invoke("take_pending_link"),
  resolveLink: (url) => backend.invoke("resolve_link", { url }),
};

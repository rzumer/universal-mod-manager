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
  quitAnyway: () => backend.invoke("quit_anyway"),
  steamGames: () => backend.invoke("steam_games"),
  addSteamGames: (appIds) => backend.invoke("add_steam_games", { appIds }),
};

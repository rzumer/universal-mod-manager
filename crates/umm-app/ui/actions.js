import { api } from "./api.js";
import { logLine } from "./addgame.js";
import { loadGame, refreshAll } from "./data.js";
import { fomodDialog } from "./dialogs/fomod.js";
import { patcherDialog, placementDialog } from "./dialogs/install.js";
import { $, h, plural } from "./dom.js";
import { confirmModal, modal, promptModal } from "./modals.js";
import { S, attempt, toast } from "./state.js";
import { renderMain } from "./tabs/gamebanana.js";

// ---------- actions ----------

export async function changeInstall() {
  const path = await attempt(() => api.pickPath("folder"));
  if (!path) return;
  await attempt(() => api.setInstallPath(S.gameId, path), "Game folder set");
  await refreshAll();
}

export async function newProfile() {
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

export async function deleteProfile() {
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

export async function importFrom(kind) {
  const path = await attempt(() => api.pickPath(kind));
  if (path) await importPath(path);
}

export async function importPath(path) {
  return importPaths([path]);
}

/// Imports one mod after another. With a single mod, UMM then asks what it still needs (its
/// install options, or where its files go); with several, those wait in the mod list.
export async function importPaths(paths) {
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
    toast(`Imported ${done.length} mods into ${S.profileName}${waiting ? `; ${waiting} ${waiting === 1 ? "needs its" : "need their"} install options chosen` : ""}`, "ok");
  }
  await refreshAll();
  if (paths.length === 1) await afterInstall(done[0]);
}

/// Asks what a new mod still needs: its installer's options, or where its files go when UMM
/// could not tell.
/// `fresh`: a new mod, not an update; cancelling its installer removes it again, since
/// nothing of it would ever be installed.
export async function afterInstall(m, fresh = true) {
  // A patcher does nothing until it has run: offered right away.
  const mod = S.details?.mods?.find((x) => x.id === m?.id);
  if (fresh && mod?.patcher) return patcherDialog(mod);
  if (m?.installer === "waiting") {
    const saved = await fomodDialog(m);
    if (!saved && fresh) {
      await attempt(() => api.removeMod(S.gameId, m.id), `Cancelled installing ${m.name}`);
      await refreshAll();
    }
  } else if (m?.placement?.guessed) await placementDialog(m, true);
}

export async function removeFromStore(m) {
  const dependents = m.thunderstore ? (await attempt(() => api.tsDependents(S.gameId, m.id))) || [] : [];
  const needed = dependents.length ? ` ${dependents.join(", ")} ${dependents.length === 1 ? "needs" : "need"} it and may stop working.` : "";
  const ok = await confirmModal(`Delete ${m.name}?`, `Removes the mod from UMM and from every profile. This cannot be undone.${needed}`, "Delete mod", true);
  if (!ok) return;
  await attempt(() => api.removeMod(S.gameId, m.id), `Deleted ${m.name}`);
  await refreshAll();
}

export async function revertNow() {
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

export async function deleteAllData() {
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

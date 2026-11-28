import { api } from "./api.js";
import { launch } from "./addgame.js";
import { $, h } from "./dom.js";
import { forgetMissingIcons, renderSidebar } from "./icons.js";
import { modal } from "./modals.js";
import { tabs } from "./panel.js";
import { S, attempt } from "./state.js";
import { loadConfigs } from "./tabs/configs.js";
import { loadGb, renderMain } from "./tabs/gamebanana.js";
import { loadActions, loadNexus } from "./tabs/nexus.js";
import { loadTs, tsInstall } from "./tabs/thunderstore.js";
import { loadWorkshop } from "./tabs/workshop.js";

// ---------- data loading ----------

/// Reloads UMM's window (and so the game definitions), coming back to the game, profile and
/// tab shown.
export function reloadDefinitions() {
  try {
    sessionStorage.setItem("umm-view", JSON.stringify({ game: S.gameId, profile: S.profileName, tab: S.tab }));
  } catch {}
  location.reload();
}

export async function loadGames() {
  S.games = (await attempt(api.listGames)) || [];
  // What was shown before "Reload game definitions", if the game is still there.
  let kept = null;
  try {
    kept = JSON.parse(sessionStorage.getItem("umm-view") || "null");
    sessionStorage.removeItem("umm-view");
  } catch {}
  if (kept && S.games.some((g) => g.id === kept.game)) {
    S.gameId = kept.game;
    if (kept.profile) S.profileName = kept.profile;
    if (kept.tab) S.tab = kept.tab;
  }
  if (!S.gameId && S.games.length) S.gameId = S.games[0].id;
  renderSidebar();
}

export async function loadGame() {
  if (!S.gameId) {
    S.details = null;
    renderMain();
    return;
  }
  const d = await attempt(() => api.gameDetails(S.gameId));
  if (!d) return;
  S.details = d;
  if (S.tab === "workshop" && !workshopShown(d.game)) S.tab = "mods";
  if (!d.profiles.includes(S.profileName)) S.profileName = "Default";
  S.profile = await attempt(() => api.getProfile(S.gameId, S.profileName));
  await loadDepIssues();
  S.plan = null;
  S.health = null;
  if (S.loader && S.loader.game !== S.gameId) S.loader = null;
  renderMain();
  if (S.tab === "files") loadPlan();
  if (S.tab === "health") loadHealth();
  if (S.tab === "config") loadConfigs();
  if (S.tab === "actions") loadActions();
  if (S.tab === "nexus" && S.nexus.game?.game !== S.gameId) loadNexus(true);
  if (S.tab === "gamebanana" && S.gb.game?.game !== S.gameId) loadGb(true);
  // Workshop items show in the Mods tab too.
  if (d.game.steam && S.workshop.game !== S.gameId) loadWorkshop(false);
  if (d.game.steam && d.game.workshopAvailable == null && S.workshopAvailable[S.gameId] == null && !workshopChecking.has(S.gameId)) checkWorkshopAvailable(S.gameId);
  if (S.tab === "workshop") {
    if (workshopShown(d.game)) {
      if (S.workshop.game !== S.gameId) loadWorkshop(false);
    } else {
      S.tab = "mods";
      renderMain();
    }
  }
  if (S.tab === "thunderstore") {
    if (d.game.thunderstore) loadTs(S.ts.game !== S.gameId);
    else {
      S.tab = "mods";
      renderMain();
    }
  }
}

/// Whether the game's Steam Workshop tab shows: only once it is known to have a Workshop.
export function workshopShown(g) {
  return !!g?.steam && (S.workshopAvailable[g.id] ?? g.workshopAvailable) === true;
}

/// Asks Steam whether a Steam game has a Workshop, when this machine does not say; its tab
/// shows when it has one.
export const workshopChecking = new Set();
export async function checkWorkshopAvailable(game) {
  workshopChecking.add(game);
  const has = await api.workshopAvailable(game).catch(() => null);
  workshopChecking.delete(game);
  S.workshopAvailable[game] = has;
  if (S.gameId === game && has === true) renderMain();
}

export async function loadPlan() {
  S.plan = "loading";
  renderMain();
  S.plan = (await attempt(() => api.previewPlan(S.gameId, S.profileName))) || null;
  renderMain();
}

export async function loadHealth() {
  S.health = "loading";
  renderMain();
  S.health = (await attempt(() => api.doctor(S.gameId))) || null;
  renderMain();
}

export async function refreshAll() {
  // Mods may have changed (deleted, imported, installed), so the Thunderstore and Nexus
  // tabs' "installed" and "update" marks are out of date until they load again.
  S.ts.stale = true;
  S.nexus.stale = true;
  S.gb.stale = true;
  // A launch may have run a patcher again (after a game update): checked again, showing the
  // last state meanwhile.
  S.patchersGen++;
  forgetMissingIcons();
  await loadGames();
  await loadGame();
}

/// The stated dependencies the profile's enabled Thunderstore packages do not meet, for the
/// Mods tab's warnings. Quietly none when they cannot be worked out.
export async function loadDepIssues() {
  S.depIssues = [];
  if (!S.details?.game.thunderstore || !S.profile) return;
  try {
    S.depIssues = await api.tsDependencyIssues(S.gameId, S.profileName);
  } catch {
    S.depIssues = [];
  }
}

export const DEP_PROBLEM = {
  notInstalled: () => "not installed",
  notInProfile: () => "not in this profile",
  disabled: () => "disabled",
  tooOld: (i) => `${i.have} in this profile`,
};

export const depLine = (i) => `${i.dependency.split("-").slice(1).join("-").replaceAll("_", " ")} ${i.needs} or later: ${DEP_PROBLEM[i.problem](i)}`;

/// A package's unmet dependencies, and installing or enabling them.
export async function depIssuesDialog(m, issues) {
  const v = await modal({
    title: `${m.name} is missing ${issues.length === 1 ? "a dependency" : "dependencies"}`,
    body: [
      h("p", { class: "muted" }, "Its Thunderstore package says it needs:"),
      h("ul", {}, issues.map((i) => h("li", {}, depLine(i)))),
      h("p", { class: "muted" }, "Some packages state dependencies they do not really need, so it can stay as it is. Otherwise, install and enable them in this profile."),
    ],
    actions: [
      { label: "Leave it", value: null, class: "ghost" },
      { label: issues.length === 1 ? "Install and enable it" : "Install and enable them", value: "go", class: "primary" },
    ],
  });
  if (v !== "go") return;
  await tsInstall([...new Set(issues.map((i) => i.dependency))]);
}

export async function saveProfile() {
  await attempt(() => api.saveProfile(S.gameId, S.profile));
  await loadDepIssues();
  renderMain();
  S.plan = null;
  if (S.tab === "files") loadPlan();
}

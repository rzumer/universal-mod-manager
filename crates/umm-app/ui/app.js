import { api } from "./api.js";
import { importPaths } from "./actions.js";
import { onSession } from "./addgame.js";
import { loadGames, refreshAll } from "./data.js";
import { $, h } from "./dom.js";
import { renderSidebar } from "./icons.js";
import { handleInstallLink } from "./links.js";
import { banner, confirmModal } from "./modals.js";
import { S, attempt, toast } from "./state.js";
import { action, renderMain } from "./tabs/gamebanana.js";
import { storeProgress } from "./tabs/store.js";
import { loadWorkshop, steamWindowPage, workshopAnswer } from "./tabs/workshop.js";

// ---------- startup ----------

function showStartup(st) {
  const restored = st.recovery.filter((r) => r.ok && r.report);
  const failed = st.recovery.filter((r) => !r.ok);
  if (restored.length)
    banner("ok", `Restored the game folder for ${restored.map((r) => r.game).join(", ")} after an interrupted session.`);
  for (const f of failed)
    banner("warn", `${f.game}: ${f.message}`, [
      h("button", { class: "small", onclick: async (e) => {
        const res = await attempt(api.recoverAll);
        if (res?.every((r) => r.ok)) {
          e.target.closest(".banner").remove();
          toast("Recovered", "ok");
        }
        refreshAll();
      } }, "Try again"),
    ]);
  for (const err of st.defErrors) banner("warn", `Game definition not loaded: ${err}`);
}

function setupDrop() {
  const overlay = $("#drop-overlay");
  api.onDrop?.((p) => {
    if (p.type === "enter" || p.type === "over") overlay.hidden = !S.gameId;
    else if (p.type === "leave") overlay.hidden = true;
    else if (p.type === "drop") {
      overlay.hidden = true;
      if (S.session.phase !== "idle") return toast("Wait for the game session to end before importing.", "error");
      importPaths(p.paths || []);
    }
  });
}

async function main() {
  api.listen("session", onSession);
  // Text cut short with "…" shows in full on hover, wherever it is.
  document.addEventListener("mouseover", (e) => {
    const el = e.target instanceof Element ? e.target.closest(".mod-meta, .mod-name, .ts-desc, .path, .game-name") : null;
    if (el && !el.title && el.scrollWidth > el.clientWidth) el.title = el.textContent;
  });
  api.listen("install-link", (url) => handleInstallLink(url));
  api.listen("ts-progress", (e) => storeProgress(S.ts, e));
  api.listen("nexus-progress", (e) => storeProgress(S.nexus, e));
  api.listen("gb-progress", (e) => storeProgress(S.gb, e));
  api.listen("workshop-action", (e) => workshopAnswer(e));
  api.listen("steam-window-page", () => steamWindowPage());
  // Steam's record of a game's Workshop items changed (an item subscribed to, downloaded or
  // removed, from UMM's Steam Workshop window or Steam itself).
  api.listen("workshop-changed", async (game) => {
    if (game !== S.gameId) return;
    await loadWorkshop(false, true);
    await loadGames();
  });
  api.listen("close-blocked", async () => {
    const ok = await confirmModal(
      "A game is still running",
      "If you close UMM now, the game's mods stay in place until UMM next starts after the game has exited.",
      "Close anyway",
      true,
    );
    if (ok) api.quitAnyway();
  });
  setupDrop();
  renderSidebar();
  renderMain();
  S.startup = await attempt(api.startup);
  if (S.startup) showStartup(S.startup);
  await refreshAll();
  // Started from a Thunderstore install link.
  const link = await attempt(api.takePendingLink);
  if (link) await handleInstallLink(link);
}

main();

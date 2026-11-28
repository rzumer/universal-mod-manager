import { api } from "../api.js";
import { deleteAllData, revertNow } from "../actions.js";
import { h } from "../dom.js";
import { S, attempt } from "../state.js";
import { action, loaderAction } from "./gamebanana.js";

// ----- Actions tab -----

export function actionsTab(d) {
  const g = d.game;
  const playing = S.session.phase !== "idle" && S.session.game === g.id;
  return h(
    "section",
    { class: "panel" },
    h(
      "div",
      { class: "actions-grid" },
      loaderAction(g, playing),
      action("Restore the game folder", "Undo a deployment left in place (after a crash, or after you stopped waiting).", "Restore now", revertNow, !g.deployed || playing),
      action("Open UMM's data for this game", "Mods, profiles, backups and runtime folders.", "Open folder", () => attempt(() => api.openFolder(d.dataDir)), false),
      action(
        "Remove this game from UMM",
        "Permanently deletes its definition and every mod, profile and setting UMM stores for it. Uninstalling the game never does this.",
        "Remove…",
        deleteAllData,
        playing || g.deployed,
        true,
      ),
    ),
  );
}

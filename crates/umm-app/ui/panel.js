import { api } from "./api.js";
import { changeInstall, deleteProfile, newProfile } from "./actions.js";
import { launch } from "./addgame.js";
import { loadGame, loadHealth, loadPlan, workshopShown } from "./data.js";
import { h, icon } from "./dom.js";
import { S, attempt } from "./state.js";
import { loadConfigs } from "./tabs/configs.js";
import { loadGb, renderMain } from "./tabs/gamebanana.js";
import { loadActions, loadNexus, refreshCollections } from "./tabs/nexus.js";
import { loadTs } from "./tabs/thunderstore.js";
import { loadWorkshop } from "./tabs/workshop.js";

// ---------- main panel ----------

export function header(d) {
  const g = d.game;
  const playing = S.session.game === g.id && S.session.phase !== "idle";
  const otherPlaying = S.session.phase !== "idle" && S.session.game !== g.id;
  const canLaunch = g.canLaunch && g.install && !playing && !otherPlaying && !g.running;
  let why = "";
  if (!g.canLaunch) why = "This game definition has no [launch] section.";
  else if (!g.install) why = "Set the game folder first.";
  else if (g.running) why = "The game is already running outside UMM.";
  else if (otherPlaying) why = "Another game is running through UMM.";

  const profileSelect = h(
    "select",
    {
      "aria-label": "Profile",
      disabled: playing,
      onchange: async (e) => {
        S.profileName = e.target.value;
        await loadGame();
      },
    },
    d.profiles.map((p) => h("option", { value: p, selected: p === S.profileName }, p)),
  );

  return h(
    "header",
    { class: "game-header" },
    h(
      "div",
      { class: "title-row" },
      h(
        "div",
        {},
        h("h1", {}, g.name),
        h(
          "div",
          { class: "install" },
          g.install
            ? h(
                "button",
                { class: "link path", title: "Open the game folder", onclick: () => attempt(() => api.openFolder(g.install)) },
                g.install,
              )
            : h("span", { class: "muted" }, "Game folder not found"),
          g.foundVia && h("span", { class: "chip" }, g.foundVia),
          h("button", { class: "link", onclick: changeInstall, disabled: playing }, g.install ? "Change…" : "Set folder…"),
        ),
        h(
          "div",
          { class: "chips" },
          h(
            "button",
            { class: "chip chip-button", title: "Change the mod loader", onclick: () => { S.tab = "actions"; renderMain(); loadActions(); } },
            g.setup,
          ),
          g.deployed && !playing && h("span", { class: "chip warn" }, "Mods are deployed"),
          g.running && h("span", { class: "chip live" }, "Game running"),
        ),
      ),
      h(
        "div",
        { class: "launch-box" },
        h(
          "div",
          { class: "profile-row" },
          h("label", { class: "inline" }, "Profile", profileSelect),
          h("button", { class: "ghost small", onclick: newProfile, disabled: playing }, "New"),
          S.profileName !== "Default" &&
            h("button", { class: "ghost small", onclick: deleteProfile, disabled: playing }, "Delete"),
        ),
        playing
          ? h(
              "button",
              { class: "primary launch", disabled: true },
              h("span", { class: "spinner" }),
              phaseLabel(S.session.phase),
            )
          : h("button", { class: "primary launch", disabled: !canLaunch, title: why || false, onclick: launch }, icon("play"), "Launch"),
        why && !playing && h("div", { class: "hint" }, why),
      ),
    ),
  );
}

export function phaseLabel(p) {
  return (
    {
      deploying: "Deploying mods…",
      running: "Game running",
      waiting: "Waiting for Steam…",
      reverting: "Restoring folder…",
    }[p] || "Working…"
  );
}

export function tabs() {
  const items = [
    ["mods", "Mods"],
    ...(S.details?.game.thunderstore ? [["thunderstore", "Thunderstore"]] : []),
    ["nexus", "Nexus Mods"],
    ["gamebanana", "GameBanana"],
    ...(workshopShown(S.details?.game) ? [["workshop", "Steam Workshop"]] : []),
    ["files", "Files"],
    ["config", "Configs"],
    ["health", "Health"],
    ["actions", "Actions"],
  ];
  return h(
    "nav",
    { class: "tabs", role: "tablist" },
    items.map(([id, label]) =>
      h(
        "button",
        {
          role: "tab",
          "aria-selected": S.tab === id ? "true" : "false",
          class: S.tab === id ? "active" : "",
          onclick: () => {
            S.tab = id;
            renderMain();
            if (id === "files" && !S.plan) loadPlan();
            if (id === "health" && !S.health) loadHealth();
            if (id === "config") loadConfigs();
            if (id === "actions") loadActions();
            if (id === "thunderstore" && (!S.ts.page || S.ts.game !== S.gameId || S.ts.stale)) loadTs(true);
            if (id === "nexus" && (!S.nexus.page || S.nexus.game?.game !== S.gameId || S.nexus.stale)) loadNexus(true);
            if (id === "gamebanana" && (!S.gb.page || S.gb.game?.game !== S.gameId || S.gb.stale)) loadGb(true);
            if (id === "workshop" && S.workshop.game !== S.gameId) loadWorkshop(false);
            // What is installed may have changed in another tab.
            else if (id === "nexus" && S.nexus.view === "collections") refreshCollections();
          },
        },
        label,
        id === "health" && S.session.game === S.gameId && S.session.phase !== "idle" && h("span", { class: "dot live" }),
      ),
    ),
  );
}

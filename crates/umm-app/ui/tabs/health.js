import { stopWaiting } from "../addgame.js";
import { loadHealth } from "../data.js";
import { $, h } from "../dom.js";
import { phaseLabel } from "../panel.js";
import { S } from "../state.js";

// ----- Health tab: the current or last session, then the doctor checks -----

export function reportView(r) {
  if (!r) return null;
  const lines = [
    `Restored ${r.restored} original file${r.restored === 1 ? "" : "s"}, removed ${r.removed} mod file${r.removed === 1 ? "" : "s"}.`,
  ];
  return h(
    "div",
    { class: "report" },
    h("p", {}, lines),
    r.keptGameVersions?.length > 0 && h("p", {}, `Kept the game's newer version of: ${r.keptGameVersions.join(", ")}`),
    r.quarantined?.length > 0 && h("p", {}, `Set aside (changed during play): ${r.quarantined.join(", ")}`),
    r.dirsLeft?.length > 0 && h("p", { class: "muted" }, `Folders left because the game added files: ${r.dirsLeft.join(", ")}`),
  );
}

export function sessionView() {
  const s = S.session;
  const mine = s.game === S.gameId;
  const active = mine && s.phase !== "idle";
  return [
    active &&
      h(
        "div",
        { class: "live-card" },
        h("div", { class: "pulse" }),
        h(
          "div",
          {},
          h("h2", {}, phaseLabel(s.phase)),
          h("p", {}, "UMM restores the game folder as soon as the game and every process it started have exited."),
        ),
        h("button", { class: "ghost", onclick: stopWaiting }, "Stop waiting"),
      ),
    !active && S.lastReport[S.gameId] && h("div", { class: "done-card" }, h("h2", {}, "Last session"), reportView(S.lastReport[S.gameId])),
  ];
}

// The session's step-by-step log, only once there is one for this game.
export function activityView() {
  const s = S.session;
  if (s.game !== S.gameId || !s.log.length) return null;
  return [h("h3", {}, "Activity"), h("ol", { class: "log" }, s.log.map((l) => h("li", { class: l.kind }, h("time", {}, l.time), l.text)))];
}

export function healthTab() {
  const findings =
    S.health === "loading" || S.health === null
      ? h("div", { class: "loading" }, h("span", { class: "spinner" }), "Checking…")
      : h("ul", { class: "findings" }, S.health.map((f) => h("li", { class: f.level }, h("span", { class: "badge" }, f.level), f.message)));
  return h(
    "section",
    { class: "panel" },
    sessionView(),
    h("div", { class: "toolbar" }, h("h2", {}, "Health check"), h("button", { onclick: loadHealth }, "Run again")),
    findings,
    activityView(),
  );
}

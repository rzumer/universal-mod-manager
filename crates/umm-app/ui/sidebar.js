import { $ } from "./dom.js";
import { S } from "./state.js";

// ---------- sidebar ----------

export function gameStatus(g) {
  if (S.session.game === g.id && S.session.phase !== "idle") return ["live", "Playing"];
  if (g.running) return ["live", "Running"];
  if (g.deployed) return ["warn", "Mods deployed"];
  if (!g.install) return ["muted", "Not found"];
  return ["ok", `${g.modCount} mod${g.modCount === 1 ? "" : "s"}`];
}

import { api } from "./api.js";
import { loadHealth, refreshAll } from "./data.js";
import { $, fmtDuration, fmtTime, h, plural } from "./dom.js";
import { renderSidebar } from "./icons.js";
import { banner, confirmModal, modal } from "./modals.js";
import { S, attempt, toast } from "./state.js";
import { renderMain } from "./tabs/gamebanana.js";

// ----- Add game manually: pick a folder, confirm what UMM found -----

export async function addManually() {
  const info = await attempt(api.pickGameFolder);
  if (!info) return;
  const name = h("input", { type: "text", value: info.name, "aria-label": "Name" });
  const exe = h(
    "select",
    { "aria-label": "Exe to launch" },
    info.exes.map((e, i) => h("option", { value: e, selected: i === 0 }, e)),
    h("option", { value: "" }, "None (UMM only deploys mods)"),
  );
  const setup = h(
    "select",
    { "aria-label": "Mod loader", disabled: info.setups.length < 2 },
    info.setups.map((o) => h("option", { value: o.value }, o.label)),
  );
  const found = [
    info.engine,
    info.community ? "on Thunderstore" : info.otherLoader ? `${info.otherLoader} on Thunderstore` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const v = await modal({
    title: "Add game manually",
    body: [
      h("p", { class: "muted small path-text" }, info.folder),
      h("p", { class: "muted small" }, `Found: ${found}`),
      info.steamAppId &&
        h("p", { class: "banner warn" }, `This is ${info.steamName} from Steam. Add from Steam sets it up with its Steam details, which this form leaves out.`),
      h("label", { class: "field" }, h("span", {}, "Name"), name),
      h("label", { class: "field" }, h("span", {}, "Exe to launch"), exe),
      h("label", { class: "field" }, h("span", {}, "Mod loader"), setup),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Add game", value: "add", class: "primary", focus: true },
    ],
  });
  if (v !== "add") return;
  const r = await attempt(() => api.addManualGame(info.folder, name.value.trim(), exe.value, setup.value || null));
  if (!r) return;
  toast(r.message, "ok");
  S.gameId = r.id;
  S.profileName = "Default";
  await refreshAll();
  await showNotes([r]);
}

export async function addFromSteam() {
  $("#modal-root").replaceChildren(
    h(
      "div",
      { class: "scrim" },
      h(
        "div",
        { class: "modal", role: "status" },
        h("div", { class: "loading" }, h("span", { class: "spinner" }), "Looking through your Steam libraries…"),
      ),
    ),
  );
  const res = await attempt(api.steamGames);
  $("#modal-root").replaceChildren();
  if (!res) return;
  const list = res.games;
  if (!list.length) {
    await modal({
      title: "No Steam games found",
      body: h("p", {}, "UMM found no installed games in your Steam libraries."),
      actions: [{ label: "OK", value: true, class: "primary", focus: true }],
    });
    return;
  }

  const fresh = list.filter((g) => !g.definedAs);
  const checks = new Map();
  const filter = h("input", { type: "text", placeholder: "Filter by name", "aria-label": "Filter games" });
  const rows = list.map((g) => {
    const box = h("input", { type: "checkbox", disabled: !!g.definedAs, "aria-label": `Add ${g.name}` });
    const setup = g.definedAs
      ? null
      : h(
          "select",
          { class: "setup", "aria-label": `Mod loader for ${g.name}`, title: "Mod loader", disabled: g.setups.length < 2 },
          g.setups.map((o) => h("option", { value: o.value }, o.label)),
        );
    // Changing the loader means the game is wanted.
    setup?.addEventListener("change", () => (box.checked = true));
    if (!g.definedAs) checks.set(g.appId, { box, setup });
    const meta = [g.engine, g.community ? "on Thunderstore" : g.otherLoader ? `${g.otherLoader} on Thunderstore` : null, `app ${g.appId}`]
      .filter(Boolean)
      .join(" · ");
    return h(
      "div",
      { class: `steam-row ${g.definedAs ? "added" : ""}`, "data-name": g.name.toLowerCase() },
      h(
        "label",
        { class: "steam-pick" },
        box,
        h("div", { class: "mod-main" }, h("div", { class: "mod-name" }, g.name), h("div", { class: "mod-meta", title: `${meta}\n${g.path}` }, meta)),
      ),
      g.definedAs ? h("span", { class: "chip" }, "Added") : setup,
    );
  });
  filter.addEventListener("input", () => {
    const q = filter.value.trim().toLowerCase();
    for (const r of rows) r.hidden = q && !r.dataset.name.includes(q);
  });

  const choice = await modal({
    title: "Add games from Steam",
    body: [
      h("p", { class: "muted" }, `${list.length} installed game${list.length === 1 ? "" : "s"}, ${fresh.length} not in UMM yet. UMM drafts a game definition for each one you pick, with the mod loader shown; you can change it later.`),
      res.thunderstoreError && h("p", { class: "banner warn" }, `Thunderstore's game list could not be loaded, so every game is offered as generic. ${res.thunderstoreError}`),
      list.length > 8 && filter,
      h("div", { class: "steam-list" }, rows),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Add selected", value: "add", class: "primary" },
    ],
  });
  if (choice !== "add") return;
  const picks = [...checks].filter(([, c]) => c.box.checked).map(([appId, c]) => ({ appId, setup: c.setup.value }));
  if (!picks.length) return toast("No games selected.");

  const results = await attempt(() => api.addSteamGames(picks));
  if (!results) return;
  const added = results.filter((r) => r.ok);
  for (const r of results.filter((r) => !r.ok)) toast(r.message, "error");
  if (added.length) {
    toast(`Added ${added.map((r) => r.message.replace(/^Added /, "")).join(", ")}`, "ok");
    S.gameId = added[0].id;
    S.profileName = "Default";
    await refreshAll();
    await showNotes(added);
  }
}

// Kinds of note the user asked never to see again ("bepinex", "unknown-engine", ...). A
// convenience of this window only, so it lives in the webview's storage, not in UMM's data.
export const HIDDEN_NOTES = "umm.hiddenNotes";
export function hiddenNotes() {
  try {
    return new Set(JSON.parse(localStorage.getItem(HIDDEN_NOTES) || "[]"));
  } catch {
    return new Set();
  }
}
export function setHiddenNotes(keys) {
  try {
    if (keys.size) localStorage.setItem(HIDDEN_NOTES, JSON.stringify([...keys]));
    else localStorage.removeItem(HIDDEN_NOTES);
  } catch {}
}

export async function showNotes(added) {
  const hidden = hiddenNotes();
  const many = added.length > 1;
  const seen = new Set();
  const items = [];
  for (const r of added) {
    for (const n of r.notes) {
      if (hidden.has(n.key)) continue;
      // The same kind of note for several games: once, naming them all.
      const dup = items.find((i) => i.key === n.key && i.text === n.text);
      if (dup) {
        dup.games.push(r.name);
        continue;
      }
      seen.add(n.key);
      items.push({ key: n.key, text: n.text, games: [r.name] });
    }
  }
  if (!items.length) return;
  const box = h("input", { type: "checkbox" });
  // One note reads as a sentence; several (more games, or more to check) as a list.
  const single = items.length === 1;
  await modal({
    title: single ? `Added ${items[0].games.join(", ")}` : "Check these before adding mods",
    body: [
      single
        ? h("p", {}, items[0].text)
        : h(
            "ul",
            { class: "notes" },
            items.map((i) => h("li", {}, many && h("strong", {}, `${i.games.join(", ")}: `), i.text)),
          ),
      h("label", { class: "check" }, box, items.length === 1 ? "Don't show this message again" : "Don't show these messages again"),
    ],
    actions: [{ label: "OK", value: true, class: "primary", focus: true }],
  });
  if (box.checked) {
    setHiddenNotes(new Set([...hidden, ...seen]));
    renderSidebar();
  }
}

export function logLine(text, kind = "") {
  S.session.log.push({ time: fmtTime(), text, kind });
}

export async function launch() {
  S.session = { game: S.gameId, phase: "deploying", log: [] };
  logLine(`Launching with profile ${S.profileName}`);
  S.tab = "health";
  renderSidebar();
  renderMain();
  if (!S.health) loadHealth();
  const ok = await attempt(() => api.launch(S.gameId, S.profileName));
  if (ok === undefined) {
    S.session.phase = "idle";
    renderMain();
    renderSidebar();
  }
}

export async function stopWaiting() {
  const ok = await confirmModal(
    "Stop waiting for the game?",
    "The game keeps running and its mods stay in place. UMM restores the folder the next time it starts after the game has exited, or when you choose Restore now.",
    "Stop waiting",
  );
  if (ok) await attempt(api.stopWaiting);
}

export function onSession(e) {
  const s = S.session;
  if (s.game !== e.game) S.session = { game: e.game, phase: "deploying", log: [] };
  switch (e.kind) {
    case "recovered":
      logLine("Restored the folder after an earlier, interrupted session");
      break;
    case "deployed":
      logLine(
        `Deployed ${e.gameFiles} file(s) to the game folder` +
          (e.runtime ? `; runtime folder: ${e.runtime.placed} placed, ${e.runtime.unchanged} unchanged, ${e.runtime.removed} removed` : ""),
      );
      for (const k of e.runtime?.keptEdited || []) logLine(`Kept your edited ${k}`);
      // The health check shown beside the session should describe the deployed state.
      if (S.tab === "health" && S.gameId === e.game) loadHealth();
      break;
    case "extenderLoader":
      S.session.loader = e.loader;
      if (e.startedSteam) logLine("Started Steam first, so the game can connect to it");
      logLine(`Starting the game with ${e.loader}, so that its script extender loads`);
      break;
    case "proton":
      logLine("Starting the game through Steam, with Proton");
      for (const n of e.notes) logLine(n);
      break;
    case "patched":
      logLine(`Swapped in the patched ${e.files.join(", ")}`);
      break;
    case "preparing":
      logLine(e.message);
      break;
    case "started":
      S.session.phase = "running";
      logLine(`Started the game (process ${e.pid})`);
      break;
    case "waitingForGame":
      S.session.phase = "waiting";
      logLine(S.session.loader ? `Started ${S.session.loader}; waiting for the game to appear` : "Launched through Steam; waiting for the game to appear");
      break;
    case "gameDetected":
      S.session.phase = "running";
      logLine("The game is running");
      break;
    case "exited":
      S.session.phase = "reverting";
      logLine(e.code == null ? "The game exited" : `The game exited (code ${e.code})`);
      break;
    case "reverted":
      S.lastReport[e.game] = e.report;
      logLine(
        `Restored the game folder: ${plural(e.report.restored, "original file")} back, ${plural(e.report.removed, "mod file")} removed`,
        "ok",
      );
      break;
    case "finished":
      S.session.phase = "idle";
      if (e.abandoned) logLine("Stopped waiting. Mods stay deployed until the game exits.", "warn");
      else if (!e.appeared) logLine("The game never started; the folder was restored.", "warn");
      else logLine(`Session over after ${fmtDuration(e.seconds)}`, "ok");
      refreshAll();
      break;
    case "failed":
      S.session.phase = "idle";
      logLine(e.message, "error");
      toast(e.message, "error");
      refreshAll();
      break;
  }
  renderMain();
  renderSidebar();
}

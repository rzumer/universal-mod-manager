import { api } from "./api.js";
import { refreshAll } from "./data.js";
import { $, h } from "./dom.js";
import { renderSidebar } from "./icons.js";
import { modal } from "./modals.js";
import { S, attempt } from "./state.js";
import { cancelWaiting, handleCollectionLink, loadNexus, nexusGet } from "./tabs/nexus.js";
import { tsInstall } from "./tabs/thunderstore.js";

// ----- Thunderstore's "Install with Mod Manager" links -----

/// The program in a link handler's command (`"C:\…\r2modman.exe" "%1"`): its path, without
/// the arguments.
export function programOf(command) {
  const m = command.match(/^\s*"([^"]+)"/) || command.match(/^\s*(\S+)/);
  return m ? m[1] : command;
}

export const LINK_TITLE = "Install buttons on mod sites";

export async function linkSettings() {
  const st = await attempt(api.linkHandler);
  if (!st) return;
  if (!st.supported) {
    await modal({ title: LINK_TITLE, body: h("p", {}, "This is not available on this system."), actions: [{ label: "OK", value: true, class: "primary", focus: true }] });
    return;
  }
  let status;
  if (st.ours) status = h("p", { class: "muted" }, "These buttons currently open UMM.");
  else if (st.others.length) {
    status = [
      h("p", { class: "muted" }, "They currently open:"),
      st.others.map((o) => h("p", { class: "muted small path-text" }, programOf(o))),
    ];
  } else status = [];
  const v = await modal({
    title: LINK_TITLE,
    body: [
      h("p", {}, "Turn this on to make these buttons open UMM, which then asks which of your games to install into:"),
      h(
        "ul",
        { class: "plain-list" },
        h("li", {}, "“Install with Mod Manager” on thunderstore.io"),
        h("li", {}, "“Mod Manager Download” on nexusmods.com, shown for games Vortex supports (UMM opens the file’s page in its own window, where you click “Slow download”)"),
      ),
      ...[status].flat(2),
    ],
    actions: [
      { label: "Close", value: null, class: "ghost" },
      st.ours ? { label: "Turn off", value: "off", class: "" } : { label: "Turn on", value: "on", class: "primary" },
    ],
  });
  if (v) {
    const r = await attempt(() => api.setLinkHandler(v === "on"), v === "on" ? "Install buttons on mod sites now open UMM" : "Install buttons on mod sites no longer open UMM");
    if (r) renderSidebar();
  }
}

/// A Nexus Mods download link from the website's "Mod Manager Download" button.
export async function handleNxmLink(url) {
  const l = await attempt(() => api.nexusResolveLink(url));
  if (!l) return;
  const what = l.modName ? `${l.modName}${l.fileName && l.fileName !== l.modName ? ` (${l.fileName})` : ""}${l.version ? ` ${l.version}` : ""}` : "this file";
  const ok1 = (text) => modal({ title: `Install ${what}?`, body: h("p", {}, text), actions: [{ label: "OK", value: true, class: "primary", focus: true }] });
  if (!l.games.length) return ok1(`None of your games can take files from nexusmods.com/${l.domain}: add the game first.`);
  const matching = l.games.filter((g) => g.matches);
  const shown = matching.length ? matching : l.games;
  const pick = h("select", { "aria-label": "Game" }, shown.map((g) => h("option", { value: g.id, selected: g.id === S.gameId }, g.name)));
  const go = await modal({
    title: `Install ${what}?`,
    body: [
      h("p", {}, "From a Nexus Mods download link, into the current profile of the game you pick. Its page opens in a UMM window, where you click “Slow download”."),
      h("label", { class: "field" }, h("span", {}, "Game"), pick),
      !matching.length && h("p", { class: "muted small" }, `None of your games is set to nexusmods.com/${l.domain} yet; the game you pick will be.`),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Download", value: "go", class: "primary", focus: true },
    ],
  });
  if (go !== "go") return;
  // A download UMM was waiting for in the browser is not needed any more.
  if (cancelWaiting) cancelWaiting();
  if (S.gameId !== pick.value) {
    S.gameId = pick.value;
    S.profileName = "Default";
    await refreshAll();
  }
  S.tab = "nexus";
  if (S.nexus.game?.game !== S.gameId) loadNexus(true);
  await nexusGet(l.domain, l.modId, l.fileId, l.modName || "the file");
}

export async function handleInstallLink(url) {
  if (/^nxm:\/\/[^/]+\/collections\//i.test(url)) return handleCollectionLink(url);
  if (/^nxm:\/\//i.test(url)) return handleNxmLink(url);
  const l = await attempt(() => api.resolveLink(url));
  if (!l) return;
  const what = `${l.name.replaceAll("_", " ")}${l.version ? ` ${l.version}` : ""} by ${l.owner}`;
  if (!l.games.length) {
    await modal({
      title: `Install ${what}?`,
      body: h("p", {}, "None of your games that get mods from Thunderstore has this package. Add the game from Steam, or set its mod loader to the one from Thunderstore in the Actions tab."),
      actions: [{ label: "OK", value: true, class: "primary", focus: true }],
    });
    return;
  }
  const pick = h("select", { "aria-label": "Game" }, l.games.map((g) => h("option", { value: g.id, selected: g.id === S.gameId }, g.name)));
  const ok = await modal({
    title: `Install ${what}?`,
    body: [h("p", {}, "From a Thunderstore link. Its dependencies are installed too, into the current profile of the game you pick."), h("label", { class: "field" }, h("span", {}, "Game"), pick)],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Install", value: "go", class: "primary", focus: true },
    ],
  });
  if (ok !== "go") return;
  if (S.gameId !== pick.value) {
    S.gameId = pick.value;
    S.profileName = "Default";
  }
  S.tab = "thunderstore";
  await refreshAll();
  await tsInstall([l.version ? `${l.fullName}@${l.version}` : l.fullName]);
}

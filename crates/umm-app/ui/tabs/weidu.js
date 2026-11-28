import { api } from "../api.js";
import { launch } from "../addgame.js";
import { $, h, icon, plural } from "../dom.js";
import { modal } from "../modals.js";
import { S, attempt } from "../state.js";
import { renderMain } from "./gamebanana.js";

// ----- WeiDU mods -----

/// The profile's WeiDU mods: their install, which runs their installers on a copy of the
/// game, and the components chosen.
export function weiduPanel(entries, byId, playing) {
  const mods = entries.filter((e) => e.enabled && byId[e.id]?.weidu);
  if (!mods.length) return null;
  const W = S.weidu;
  const key = `${S.gameId}/${S.profileName}/${mods.map((e) => e.id).join(",")}`;
  if ((W.key !== key || W.gen !== S.patchersGen) && !W.loading) {
    const gen = S.patchersGen;
    const game = S.gameId;
    if (W.key !== key) W.status = null;
    Object.assign(W, { key, loading: true });
    api
      .weiduStatus(game, S.profileName)
      .then((st) => Object.assign(W, { status: st, gen, loading: false }))
      .catch(() => Object.assign(W, { gen, loading: false }))
      .finally(() => S.tab === "mods" && S.gameId === game && renderMain());
  }
  const st = W.status;
  const unchosen = (st?.mods || []).filter((m) => !m.chosen).length;
  const [cls, text] = !st
    ? ["", "Checking the WeiDU mods…"]
    : {
        needsChoices: ["warn", `Choose the components of ${plural(unchosen, "WeiDU mod")} below: each installs as soon as they are chosen.`],
        notRun: ["", "Ready to install the WeiDU mods with components chosen."],
        outdated: ["", "The game or the WeiDU mods changed: they install again, with your components, at the next launch."],
        failed: ["warn", `The WeiDU mods could not be installed: ${st.error || ""}`],
        current: ["ok", `${plural(st.installed.length, "WeiDU mod")} installed (${plural(st.files, "file")} changed or added), in the game only while mods are deployed.`],
      }[st.state];
  // Mods with components chosen install; the others wait, left out.
  const ready = st && unchosen < (st.mods || []).length && !st.needsWeidu;
  const firstRun = st && st.state === "notRun";
  const still = st && unchosen > 0 && st.state !== "needsChoices" ? ` ${plural(unchosen, "mod")} still to choose.` : "";
  const getWeidu = async () => {
    const p = await attempt(() => api.pickPath("exe"));
    if (!p) return;
    if ((await attempt(() => api.setWeiduExe(p), "WeiDU saved")) === undefined) return;
    S.weidu.gen = -1;
    renderMain();
  };
  const names = Object.fromEntries(mods.map((e) => [e.id, byId[e.id].name]));
  return h(
    "div",
    { class: `weidu-panel ${cls}` },
    h(
      "div",
      { class: "weidu-head" },
      h("span", {}, text + still),
      h(
        "div",
        { class: "row" },
        st?.output && h("button", { class: "ghost small", onclick: () => weiduLog(st) }, "Log"),
        st && h("button", { class: firstRun && ready ? "primary small" : "small", disabled: playing || !!S.busy || !ready, title: ready ? "" : "Choose every mod's components first", onclick: runWeidu }, st.ranUnix ? "Install again" : "Install WeiDU mods"),
      ),
    ),
    st?.needsWeidu &&
      h(
        "div",
        { class: "weidu-need" },
        h("span", {}, "A mod here comes without WeiDU's installer. Download WeiDU for Windows, unpack it, and give UMM its weidu.exe: UMM uses it for every mod that needs it."),
        h(
          "div",
          { class: "row" },
          h("button", { class: "link", onclick: () => attempt(() => api.openWebPage("https://github.com/WeiDUorg/weidu/releases")) }, "Get WeiDU", icon("external")),
          h("button", { class: "small", onclick: getWeidu }, "Choose weidu.exe…"),
        ),
      ),
    st?.mods?.length > 0 &&
      h(
        "ul",
        { class: "plain-list weidu-mods" },
        st.mods.map((m) =>
          h(
            "li",
            {},
            h(
              "span",
              {},
              h("strong", {}, names[m.modId] || m.modId),
              ": ",
              m.chosen ? (m.components.length ? m.components.map(([, n]) => n || "?").join(", ") : "nothing") : h("span", { class: "muted" }, "components not chosen yet"),
            ),
            h("button", { class: m.chosen ? "ghost small" : "primary small", disabled: playing || !!S.busy || st.needsWeidu, onclick: () => weiduComponentsDialog(m.modId, names[m.modId] || m.modId) }, "Components…"),
          ),
        ),
      ),
  );
}

/// Chooses a WeiDU mod's language and components, as WeiDU lists them. Components named
/// `Group -> Choice` are one group's alternatives: one of them, or none.
export async function weiduComponentsDialog(id, name) {
  const game = S.gameId;
  const profile = S.profileName;
  S.busy = `Asking ${name}'s installer for its components…`;
  renderMain();
  let list = await attempt(() => api.weiduComponents(game, profile, id, null));
  S.busy = false;
  renderMain();
  if (!list) return;
  // Chosen numbers stay across language changes: they are the same components.
  let picked = new Set(list.chosen ? list.chosen.components : []);
  const body = h("div", { class: "weidu-components" });
  const draw = () => {
    const groups = new Map();
    for (const [n, label] of list.components) {
      const i = label.indexOf(" -> ");
      if (i > 0) {
        const g = label.slice(0, i);
        if (!groups.has(g)) groups.set(g, []);
        groups.get(g).push([n, label.slice(i + 4)]);
      }
    }
    const order = [];
    const seen = new Set();
    for (const [n, label] of list.components) {
      const i = label.indexOf(" -> ");
      const g = i > 0 ? label.slice(0, i) : null;
      if (g) {
        if (!seen.has(g)) {
          seen.add(g);
          order.push(["group", g]);
        }
      } else order.push(["one", n, label]);
    }
    const lang =
      list.languages.length > 1 &&
      h(
        "label",
        { class: "field" },
        h("span", {}, "Language"),
        (() => {
          const sel = h("select", { "aria-label": "Language" }, list.languages.map(([n, l]) => h("option", { value: String(n), selected: n === list.language }, l)));
          sel.addEventListener("change", async () => {
            const next = await attempt(() => api.weiduComponents(game, profile, id, Number(sel.value)));
            if (next) {
              list = next;
              draw();
            }
          });
          return sel;
        })(),
      );
    const rows = order.map((o) => {
      if (o[0] === "one") {
        const [, n, label] = o;
        const box = h("input", { type: "checkbox", checked: picked.has(n) });
        box.addEventListener("change", () => (box.checked ? picked.add(n) : picked.delete(n)));
        return h("label", { class: "weidu-component" }, box, h("span", {}, label), h("span", { class: "muted small" }, `#${n}`));
      }
      const [, g] = o;
      const opts = groups.get(g);
      const radio = (n, label) => {
        const r = h("input", { type: "radio", name: `weidu-${g}`, checked: n === null ? !opts.some(([x]) => picked.has(x)) : picked.has(n) });
        r.addEventListener("change", () => {
          for (const [x] of opts) picked.delete(x);
          if (n !== null) picked.add(n);
        });
        return h("label", { class: "weidu-component" }, r, h("span", {}, label), n !== null && h("span", { class: "muted small" }, `#${n}`));
      };
      return h("fieldset", { class: "weidu-group" }, h("legend", {}, g), radio(null, "None"), opts.map(([n, label]) => radio(n, label)));
    });
    body.replaceChildren(
      h("p", { class: "muted small" }, "The components install in this order, without the installer's menu. A component may still ask its own questions in a console window."),
      lang || null,
      h("div", { class: "weidu-component-list" }, rows),
    );
  };
  draw();
  const v = await modal({
    title: `${name}: components`,
    body,
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Save", value: "save", class: "primary" },
    ],
  });
  if (v !== "save") return;
  const components = list.components.filter(([n]) => picked.has(n));
  const st = await attempt(() => api.setWeiduChoice(game, profile, id, list.language, components), `Components saved for ${name}`);
  if (!st) return;
  S.weidu.status = st;
  renderMain();
  // Installed right away, with the other mods chosen so far (not while mods are deployed or
  // the game runs: the next launch installs then).
  const playing = S.session.phase !== "idle" && S.session.game === game;
  const ready = !st.needsWeidu;
  if (ready && !playing && !S.details?.game?.deployed && S.gameId === game && S.profileName === profile) await runWeidu();
}

export function weiduLog(st) {
  return modal({
    title: "WeiDU install log",
    body: h("pre", { class: "patcher-output weidu-log" }, st.output),
    actions: [{ label: "Close", value: null, class: "primary", focus: true }],
  });
}

/// Installs the profile's WeiDU mods with the components chosen, without their menus.
export async function runWeidu() {
  const W = S.weidu;
  const game = S.gameId;
  S.busy = "Installing the WeiDU mods… A component may ask a question in a console window.";
  renderMain();
  const r = await attempt(() => api.runWeidu(game, S.profileName), "WeiDU mods installed");
  S.busy = false;
  W.gen = -1;
  if (r) W.status = r;
  renderMain();
  if (!r) {
    const now = await api.weiduStatus(game, S.profileName).catch(() => null);
    if (now) W.status = now;
    renderMain();
    if (now?.output) weiduLog(now);
  }
}

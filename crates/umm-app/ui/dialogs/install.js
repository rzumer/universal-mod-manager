import { api } from "../api.js";
import { launch } from "../addgame.js";
import { refreshAll } from "../data.js";
import { $, h, plural } from "../dom.js";
import { banner, modal } from "../modals.js";
import { S, attempt, toast } from "../state.js";
import { renderMain } from "../tabs/gamebanana.js";

/// Where a mod's files go: the folder inside it they install from, the game folder they go
/// into, and the files left out. `asked` when UMM could not tell and opened it after an install.
/// Why an enabled mod does nothing yet, if it does not: a WeiDU mod not installed (with the
/// profile's other WeiDU mods, in the section above the list), a patcher not run.
export function pendingInstall(m) {
  if (m?.weidu) {
    const st = S.weidu.status;
    if (!st?.installed?.includes(m.id)) return "Off until it is installed: choose its components above";
  }
  if (m?.patcher) {
    const st = S.patchers[`${S.gameId}/${m.id}`]?.s;
    if (st && (st.state === "notRun" || st.state === "failed")) return `Off until its patcher has run (${st.exe})`;
  }
  return null;
}

/// A patcher mod's state in its row, loading it the first time the row shows.
export function patcherChip(m) {
  const game = S.gameId;
  const key = `${game}/${m.id}`;
  const entry = S.patchers[key];
  const st = entry?.s;
  if (!entry || (entry.gen !== S.patchersGen && !entry.loading)) {
    const gen = S.patchersGen;
    S.patchers[key] = { s: st, gen: entry?.gen, loading: true };
    api
      .patcherStatus(game, m.id)
      .then((s) => {
        S.patchers[key] = { s, gen };
        if (S.gameId === game && S.tab === "mods") renderMain();
      })
      .catch(() => {
        S.patchers[key] = { s: st, gen };
      });
  }
  const [cls, text, title] = !st
    ? ["chip", "Patcher", "Its program patches the game's files"]
    : st.state === "current"
      ? ["chip ok", `Patches ${st.files.join(", ")}`, "While mods are deployed, its patched files replace the game's"]
      : st.state === "outdated"
        ? ["chip", "Patcher runs again at the next launch", "The game changed since it last ran (an update)"]
        : st.state === "failed"
          ? ["chip warn", "Patcher failed", st.error || ""]
          : ["chip warn", `Run ${st.exe}`, "Nothing of this mod is used until its patcher has run"];
  return h("button", { class: `${cls} chip-button`, title, onclick: () => patcherDialog(m) }, text);
}

/// A patcher mod: what running it does, its last run, and running it (the user's approval
/// to run it again by itself after a game update).
export async function patcherDialog(m) {
  const game = S.gameId;
  const key = `${game}/${m.id}`;
  const st = await attempt(() => api.patcherStatus(game, m.id));
  if (!st) return;
  S.patchers[key] = { s: st, gen: S.patchersGen };
  const when = st.ranUnix ? new Date(st.ranUnix * 1000).toLocaleString() : null;
  const body = [
    h(
      "p",
      {},
      `${m.name} is a patcher: its program, ${st.exe}, changes the game's own files. UMM runs it, keeps the files it changed, and puts the game's own files back afterwards.`,
    ),
    h(
      "p",
      { class: "muted" },
      "While mods are deployed, the patched files replace the game's, and the originals come back afterwards, so Steam still sees the game as it shipped. After a game update, UMM runs the patcher again by itself before the next launch.",
    ),
    st.files.length > 0 && h("p", {}, h("strong", {}, "Patches: "), st.files.join(", ")),
    st.error && h("p", { class: "error-text" }, st.error),
    when && h("p", { class: "muted small" }, `Last run ${when}.`),
    st.output && h("pre", { class: "patcher-output" }, st.output),
  ];
  const v = await modal({
    title: `${m.name}: patcher`,
    body,
    actions: [
      { label: "Close", value: null, class: "ghost" },
      { label: st.approved ? `Run ${st.exe} again` : `Run ${st.exe}`, value: "run", class: "primary", focus: !st.approved },
    ],
  });
  if (v !== "run") return;
  S.busy = `Running ${st.exe}…`;
  renderMain();
  const r = await attempt(() => api.runPatcher(game, m.id));
  S.busy = false;
  S.patchers[key] = { s: r || (await api.patcherStatus(game, m.id).catch(() => undefined)), gen: S.patchersGen };
  if (r) toast(`${st.exe} patched ${r.files.join(", ")}; the patched files replace the game's while mods are deployed`, "ok");
  renderMain();
  if (!r && S.patchers[key]?.s?.output) await patcherDialog(m);
}

export async function placementDialog(m, asked = false) {
  const game = S.gameId;
  let view = await attempt(() => api.placementView(game, m.id, null));
  if (!view) return;
  const p = { ...view.placement, skip: [...view.placement.skip] };
  const list = h("div", { class: "place-files" });
  const summary = h("p", { class: "muted small" });
  const under = (f) => !p.root || f.toLowerCase().startsWith(`${p.root.toLowerCase()}/`);
  const shown = (f) => (p.root ? f.slice(p.root.length + 1) : f);
  let timer = null;
  const refresh = (now = false) => {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      try {
        view = await api.placementView(game, m.id, p);
        draw();
      } catch (e) {
        summary.textContent = String(e);
        summary.className = "small error-text";
      }
    }, now ? 0 : 300);
  };
  const draw = () => {
    const inside = view.files.filter((f) => under(f.file));
    const placed = inside.filter((f) => f.target).length;
    const outside = view.files.length - inside.length;
    summary.className = "muted small";
    summary.textContent =
      `${plural(placed, "file")} go${placed === 1 ? "es" : ""} into the game` +
      (inside.length - placed ? `, ${inside.length - placed} left out` : "") +
      (outside ? `; ${plural(outside, "file")} outside this folder ${outside === 1 ? "is" : "are"} left out` : "") +
      ".";
    list.replaceChildren(
      ...inside.map((f) =>
        h(
          "label",
          { class: `place-file ${f.target ? "" : "out"}` },
          h("input", {
            type: "checkbox",
            checked: !f.skipped,
            "aria-label": `Install ${f.file}`,
            onchange: (ev) => {
              p.skip = p.skip.filter((s) => s.toLowerCase() !== f.file.toLowerCase());
              if (!ev.target.checked) p.skip.push(f.file);
              refresh(true);
            },
          }),
          h("code", { class: "place-from" }, shown(f.file)),
          h("span", { class: "place-to" }, f.target ? h("code", {}, `→ ${f.target}`) : h("span", { class: "muted" }, f.note)),
        ),
      ),
    );
  };
  const rootSel = h(
    "select",
    {
      "aria-label": "Install from",
      onchange: (ev) => {
        p.root = ev.target.value;
        p.skip = p.skip.filter(under);
        refresh(true);
      },
    },
    h("option", { value: "", selected: !p.root }, "The whole mod"),
    view.folders.map((d) => h("option", { value: d, selected: d === p.root }, d)),
  );
  const targetIn = h("input", {
    type: "text",
    list: "place-game-folders",
    value: p.target,
    placeholder: "The game folder itself",
    "aria-label": "Into this game folder",
    oninput: (ev) => {
      p.target = ev.target.value;
      refresh();
    },
  });
  draw();
  const s = view.suggested;
  // The files the suggestion leaves out, as they appear in the list below.
  const inRoot = (f) => (s.root && f.toLowerCase().startsWith(`${s.root.toLowerCase()}/`) ? f.slice(s.root.length + 1) : f);
  const names = s.skip.map(inRoot);
  const leftOut = !names.length
    ? ""
    : names.length <= 3
      ? `, leaving out ${names.slice(0, -1).join(", ")}${names.length > 1 ? " and " : ""}${names[names.length - 1]}`
      : `, leaving out ${names.slice(0, 3).join(", ")} and ${names.length - 3} more`;
  const key = (x) => [x.root.toLowerCase(), x.target.toLowerCase(), ...x.skip.map((f) => f.toLowerCase()).sort()].join("\n");
  const showSuggestion = () => {
    p.root = s.root;
    p.target = s.target;
    p.skip = [...s.skip];
    rootSel.value = s.root;
    targetIn.value = s.target;
    refresh(true);
  };
  const v = await modal({
    title: `Where ${m.name} installs`,
    body: [
      asked && h("p", { class: "banner warn" }, "UMM could not tell where this mod's files go from what is in the game folder. Check them before playing."),
      h("p", { class: "muted" }, "Pick the folder inside the mod to install from, and the game folder its contents go into. Untick files to leave them out."),
      h("label", { class: "field" }, h("span", {}, "Install from"), rootSel),
      h("label", { class: "field" }, h("span", {}, "Into this game folder"), targetIn, h("datalist", { id: "place-game-folders" }, view.gameFolders.map((d) => h("option", { value: d })))),
      h(
        "p",
        { class: "muted small" },
        `UMM suggests: ${s.root ? `“${s.root}”` : "the whole mod"} into ${s.target ? `“${s.target}”` : "the game folder"}${leftOut}${s.guessed ? " (a guess)" : ""}. `,
        h("button", { class: "link inline-link", title: "Fill in the suggestion below; nothing is saved until you press Save", onclick: showSuggestion }, "Apply"),
      ),
      summary,
      list,
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Save", value: "save", class: "primary", focus: true },
    ],
  });
  clearTimeout(timer);
  if (v !== "save") return;
  // Saved as UMM's own when it is the suggestion, so updates keep suggesting afresh.
  if (key(p) === key(s)) await attempt(() => api.setPlacement(game, m.id, null), `${m.name} uses UMM's suggestion`);
  else await attempt(() => api.setPlacement(game, m.id, p), `Saved where ${m.name} installs`);
  await refreshAll();
}

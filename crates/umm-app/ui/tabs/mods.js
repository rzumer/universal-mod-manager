import { api } from "../api.js";
import { importFrom, removeFromStore } from "../actions.js";
import { depIssuesDialog, depLine, saveProfile } from "../data.js";
import { fomodDialog } from "../dialogs/fomod.js";
import { patcherChip, pendingInstall, placementDialog } from "../dialogs/install.js";
import { $, fmtBytes, h, icon } from "../dom.js";
import { modIcon, workshopIcon, workshopId } from "../icons.js";
import { S, attempt } from "../state.js";
import { renderMain } from "./gamebanana.js";
import { tsFollowLatest } from "./thunderstore.js";
import { weiduPanel } from "./weidu.js";

// ----- Mods tab -----


export function modsTab(d) {
  const playing = S.session.phase !== "idle" && S.session.game === S.gameId;
  const byId = Object.fromEntries(d.mods.map((m) => [m.id, m]));
  const entries = S.profile?.mods || [];
  const inProfile = new Set(entries.map((e) => e.id));
  const notIn = d.mods.filter((m) => !inProfile.has(m.id));
  // Steam Workshop items: in the profile where UMM controls them, else shown as Steam has them.
  const control = d.game.workshop;
  const wsLoaded = S.workshop.game === S.gameId && Array.isArray(S.workshop.list?.items);
  const wsItems = wsLoaded ? S.workshop.list.items.filter((i) => i.subscribed) : [];
  const wsById = Object.fromEntries((wsLoaded ? S.workshop.list.items : []).map((i) => [i.id, i]));
  const wsNotIn = control === "profile" ? wsItems.filter((i) => !inProfile.has(`workshop:${i.id}`)) : [];
  const wsLocked = control && control !== "profile" ? wsItems : [];
  const wsName = (i) => i.title || `Workshop item ${i.id}`;

  const move = async (from, to) => {
    if (to < 0 || to >= entries.length || from === to) return;
    const [e] = entries.splice(from, 1);
    entries.splice(to, 0, e);
    renderMain();
    await saveProfile();
  };

  const rows = entries.map((e, i) => {
    const m = byId[e.id];
    // Reordering uses pointer events, not HTML drag and drop: the app's native file drop
    // (drop an archive on the window to import it) turns HTML drag and drop off in WebView2.
    const grip = h("span", { class: `grip ${playing ? "off" : ""}`, title: playing ? "" : "Drag to reorder" }, icon("grip"));
    if (!playing) {
      grip.addEventListener("pointerdown", (ev) => {
        if (ev.button !== 0) return;
        ev.preventDefault();
        grip.setPointerCapture(ev.pointerId);
        const list = row.parentElement;
        row.classList.add("dragging");
        let target = i;
        // The position the row would end up at: before the first row whose middle is below
        // the pointer, counted without the row being dragged.
        const over = (y) => {
          // Locked rows (Workshop items Steam loads) have no place in the order.
          const rows = [...list.children].filter((r) => !r.classList.contains("locked"));
          const before = rows.findIndex((r) => {
            const b = r.getBoundingClientRect();
            return y < b.top + b.height / 2;
          });
          if (before === -1) return rows.length - 1;
          return before > i ? before - 1 : before;
        };
        const mark = () => {
          // Moving up, the row lands above the target row; moving down, below it.
          for (const [k, r] of [...list.children].entries()) {
            r.classList.toggle("drop-target", k === target && target < i);
            r.classList.toggle("drop-after", k === target && target > i);
          }
        };
        const onMove = (mv) => {
          target = over(mv.clientY);
          mark();
        };
        const onUp = () => {
          grip.removeEventListener("pointermove", onMove);
          grip.removeEventListener("pointerup", onUp);
          grip.removeEventListener("pointercancel", onUp);
          row.classList.remove("dragging");
          for (const r of list.children) r.classList.remove("drop-target", "drop-after");
          if (target !== i) move(i, target);
        };
        grip.addEventListener("pointermove", onMove);
        grip.addEventListener("pointerup", onUp);
        grip.addEventListener("pointercancel", onUp);
      });
    }
    // A mod whose installer's options are not chosen yet installs nothing: shown off, and
    // locked off, until they are.
    const waiting = m?.installer === "waiting";
    // Shown off, and locked off, until the mod can do anything: its installer's options
    // chosen, its WeiDU install done, its patcher run. A mod switched off stays switchable.
    const wid = workshopId(e.id);
    const wi = wid ? wsById[wid] || { id: wid, title: null, folder: null, gone: wsLoaded } : null;
    const lock = wi
      ? control === "profile"
        ? null
        : "Steam and the game decide whether it loads"
      : waiting
        ? "Off until its install options are chosen"
        : e.enabled
          ? pendingInstall(m)
          : null;
    const on = e.enabled && !lock;
    const row = h(
      "li",
      { class: `mod-row ${on ? "" : "disabled"} ${m || (wi && !wi.gone) ? "" : "missing"}` },
      grip,
      h("span", { class: "order" }, i + 1),
      h(
        "label",
        { class: "toggle", title: lock || (on ? "Enabled" : "Disabled") },
        h("input", {
          type: "checkbox",
          checked: on,
          disabled: playing || !!lock,
          "aria-label": `Enable ${wi ? wsName(wi) : m?.name || e.id}`,
          onchange: async (ev) => {
            e.enabled = ev.target.checked;
            renderMain();
            await saveProfile();
          },
        }),
        h("span", { class: "track" }),
      ),
      wi ? workshopIcon(wi, wsName(wi)) : modIcon(S.gameId, e.id, m ? m.name : e.id),
      h(
        "div",
        { class: "mod-main" },
        h("div", { class: "mod-name" }, wi ? wsName(wi) : m ? m.name : e.id, !wi && (e.version || m?.version) && h("span", { class: "version" }, e.version || m.version)),
        h(
          "div",
          { class: "mod-meta" },
          wi
            ? wi.gone
              ? `Steam Workshop · ${wi.id} · no longer subscribed in Steam — remove it from the profile`
              : `Steam Workshop · ${wi.id}${wi.folder ? (wi.size ? ` · ${fmtBytes(wi.size)}` : "") : S.workshop.list?.gameDownloads ? " · the game downloads it when it starts" : " · not downloaded by Steam yet"}`
            : m
              ? `${m.thunderstore ? "Thunderstore · " : m.nexusUrl ? "Nexus Mods · " : m.gamebananaUrl ? "GameBanana · " : ""}${e.id} · ${m.files} file${m.files === 1 ? "" : "s"} · ${fmtBytes(m.size)}`
              : "No longer installed — remove it from the profile or import it again",
          m?.thunderstore && e.version
            ? h(
                "button",
                {
                  class: "chip chip-button",
                  disabled: playing,
                  title: `Pinned to ${e.version} in this profile${m.version !== e.version ? ` (latest installed: ${m.version})` : ""}. Click to follow the latest version instead.`,
                  onclick: () => tsFollowLatest(m.thunderstore),
                },
                "Pinned",
              )
            : m?.patcher
            ? patcherChip(m)
            : m?.installer === "waiting"
            ? h("button", { class: "chip warn chip-button", title: "Nothing of this mod is installed until its installer's options are chosen", onclick: () => fomodDialog(m) }, "Choose its install options")
            : m?.placement?.guessed &&
                h("button", { class: "chip warn chip-button", title: "UMM could not tell where this mod's files go", onclick: () => placementDialog(m) }, "Check deployment"),
          (() => {
            const issues = on && m ? (S.depIssues || []).filter((i) => i.modId === e.id) : [];
            if (!issues.length) return null;
            return h(
              "button",
              { class: "chip warn chip-button", disabled: playing, title: `Its stated dependencies:\n${issues.map(depLine).join("\n")}`, onclick: () => depIssuesDialog(m, issues) },
              issues.length === 1 ? "Missing a dependency" : `Missing ${issues.length} dependencies`,
            );
          })(),
        ),
      ),
      h(
        "div",
        { class: "row-actions" },
        m?.placeable &&
          (m.installer
            ? h("button", { class: "icon-btn", title: "Its installer's options, and where its files go", "aria-label": `Install options for ${m.name}`, disabled: playing, onclick: () => fomodDialog(m) }, icon("sliders"))
            : h("button", { class: "icon-btn", title: "Choose where its files go in the game folder", "aria-label": `Where ${m.name} installs`, disabled: playing, onclick: () => placementDialog(m) }, icon("sliders"))),
        m?.nexusUrl &&
          h("button", { class: "icon-btn", title: "Open its page on Nexus Mods", "aria-label": "Open on Nexus Mods", onclick: () => attempt(() => api.openNexusPage(m.nexusUrl)) }, "↗"),
        m?.gamebananaUrl &&
          h("button", { class: "icon-btn", title: "Open its page on GameBanana", "aria-label": "Open on GameBanana", onclick: () => attempt(() => api.openGbPage(m.gamebananaUrl)) }, "↗"),
        m?.thunderstore &&
          h("button", { class: "icon-btn", title: "Open its page on Thunderstore", "aria-label": "Open on Thunderstore", onclick: () => attempt(() => api.openPackagePage(S.gameId, m.thunderstore)) }, "↗"),
        wi &&
          h("button", { class: "icon-btn", title: "Open its Workshop page", "aria-label": "Open its Workshop page", onclick: () => attempt(() => api.openWorkshopWindow(S.gameId, wi.id)) }, "↗"),
        h("button", { class: "icon-btn", title: "Move up", "aria-label": "Move up", disabled: playing || i === 0, onclick: () => move(i, i - 1) }, icon("up")),
        h(
          "button",
          { class: "icon-btn", title: "Move down", "aria-label": "Move down", disabled: playing || i === entries.length - 1, onclick: () => move(i, i + 1) },
          icon("down"),
        ),
        h(
          "button",
          { class: "icon-btn", title: "Remove from this profile", "aria-label": "Remove from profile", disabled: playing, onclick: async () => {
            entries.splice(i, 1);
            renderMain();
            await saveProfile();
          } },
          "×",
        ),
      ),
    );
    return row;
  });

  // Workshop items UMM does not control, after the profile's mods: always as Steam has them,
  // so locked, with no place in the order.
  const lockedWhy = "Subscribed in Steam: the game loads it through Steam, whatever the profile. Unsubscribe in Steam to remove it.";
  const lockedRows = wsLocked.map((i) =>
    h(
      "li",
      { class: "mod-row locked", title: lockedWhy },
      h("span", { class: "grip off" }),
      h("span", { class: "order" }, ""),
      control === "steam"
        ? h(
            "label",
            { class: "toggle" },
            h("input", { type: "checkbox", checked: true, disabled: true, "aria-label": `${wsName(i)} is loaded through Steam` }),
            h("span", { class: "track" }),
          )
        : h("span", { class: "toggle" }),
      workshopIcon(i, wsName(i)),
      h(
        "div",
        { class: "mod-main" },
        h("div", { class: "mod-name" }, wsName(i)),
        h(
          "div",
          { class: "mod-meta" },
          `Steam Workshop · ${i.id}${i.folder ? (i.size ? ` · ${fmtBytes(i.size)}` : "") : " · not downloaded by Steam yet"}`,
          h("span", { class: "chip" }, "Loaded through Steam"),
        ),
      ),
      h(
        "div",
        { class: "row-actions" },
        h("button", { class: "icon-btn", title: "Open its Workshop page", "aria-label": "Open its Workshop page", onclick: () => attempt(() => api.openWorkshopWindow(S.gameId, i.id)) }, "↗"),
      ),
    ),
  );

  return h(
    "section",
    { class: "panel" },
    h(
      "div",
      { class: "toolbar" },
      h("div", {}, h("h2", {}, `Load order · ${S.profileName}`), h("p", { class: "muted" }, "Mods lower in the list win when two ship the same file.")),
      h(
        "div",
        { class: "toolbar-actions" },
        h("button", { onclick: () => importFrom("archive"), disabled: playing || S.busy }, "Import archive…"),
        h("button", { onclick: () => importFrom("folder"), disabled: playing || S.busy }, "Import folder…"),
      ),
    ),
    weiduPanel(entries, byId, playing),
    entries.length || lockedRows.length
      ? h("ol", { class: "mod-list" }, rows, lockedRows)
      : h(
          "div",
          { class: "empty drop-hint" },
          h("p", { class: "big" }, "No mods in this profile yet"),
          h("p", {}, "Import an archive (.zip, .7z or .rar) or a folder, or drop one onto this window."),
        ),
    wsNotIn.length > 0 &&
      h(
        "div",
        { class: "store-section" },
        h("h3", {}, "Subscribed in Steam, not in this profile"),
        h("p", { class: "muted small" }, "The game keeps these as they are in its own mod list. Add one to turn it on or off with this profile."),
        h(
          "ul",
          { class: "store-list" },
          wsNotIn.map((i) =>
            h(
              "li",
              {},
              workshopIcon(i, wsName(i)),
              h("div", { class: "mod-main" }, h("div", { class: "mod-name" }, wsName(i)), h("div", { class: "mod-meta" }, `Steam Workshop · ${i.id}`)),
              h(
                "div",
                { class: "row-actions" },
                h("button", { class: "small", disabled: playing, onclick: async () => {
                  entries.push({ id: `workshop:${i.id}`, enabled: true });
                  renderMain();
                  await saveProfile();
                } }, "Add"),
              ),
            ),
          ),
        ),
      ),
    notIn.length > 0 &&
      h(
        "div",
        { class: "store-section" },
        h("h3", {}, "Installed for this game, not in this profile"),
        h(
          "ul",
          { class: "store-list" },
          notIn.map((m) =>
            h(
              "li",
              {},
              modIcon(S.gameId, m.id, m.name),
              h("div", { class: "mod-main" }, h("div", { class: "mod-name" }, m.name), h("div", { class: "mod-meta" }, `${m.id} · ${m.files} files · ${fmtBytes(m.size)}`)),
              h("div", { class: "row-actions" },
                h("button", { class: "small", disabled: playing, onclick: async () => {
                  entries.push({ id: m.id, enabled: true });
                  renderMain();
                  await saveProfile();
                } }, "Add"),
                h("button", { class: "small ghost danger-text", disabled: playing, onclick: () => removeFromStore(m) }, icon("trash"), "Delete"),
              ),
            ),
          ),
        ),
      ),
    S.busy && h("div", { class: "busy-bar" }, h("span", { class: "spinner" }), S.busy),
  );
}

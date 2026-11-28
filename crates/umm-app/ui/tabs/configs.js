import { api } from "../api.js";
import { $, h } from "../dom.js";
import { banner, confirmModal } from "../modals.js";
import { S, attempt } from "../state.js";
import { renderMain } from "./gamebanana.js";

// ----- Files tab -----

// ----- Configs tab -----

/// Loads the profile's config files (again: mods create and change them while the game
/// runs). An open file stays open.
export async function loadConfigs() {
  const game = S.gameId;
  const profile = S.profileName;
  const C = S.config;
  const key = `${game}/${profile}`;
  if (C.key !== key) {
    C.key = key;
    C.list = null;
    C.open = null;
    renderMain();
  }
  const r = await attempt(() => api.configFiles(game, profile));
  if (C.key !== key) return;
  C.list = r || { dir: "", files: [], error: true };
  if (S.tab === "config") renderMain();
}

// A section and key, joined by a character neither holds (and that `CSS.escape` keeps).
export const configEditKey = (section, key) => `${section}\u001f${key}`;

export function configDirty() {
  const o = S.config.open;
  if (!o) return false;
  return o.raw ? o.text !== o.view.text : Object.keys(o.edits).length > 0;
}

/// Asks before throwing away unsaved changes. Resolves to whether to go on.
export async function configLeave() {
  if (!configDirty()) return true;
  return confirmModal("Discard your changes?", "The changes to this config file are not saved.", "Discard", true);
}

export async function openConfig(path) {
  if (!(await configLeave())) return;
  const C = S.config;
  const key = C.key;
  const view = await attempt(() => api.readConfig(S.gameId, S.profileName, path));
  if (!view || C.key !== key) return;
  C.open = { path, view, edits: {}, raw: !view.sections, text: view.text };
  renderMain();
}

/// Enables Save once something changed, without drawing the editor again (which would take
/// the focus away from the field being typed in).
export function configRefreshSave() {
  const b = $("#config-save");
  if (b) b.disabled = !configDirty();
  const r = $("#config-revert");
  if (r) r.disabled = !configDirty();
}

export async function saveConfig() {
  const C = S.config;
  const o = C.open;
  if (!o || !configDirty()) return;
  const game = S.gameId;
  const profile = S.profileName;
  const view = o.raw
    ? await attempt(() => api.setConfigText(game, profile, o.path, o.text), "Config saved")
    : await attempt(
        () =>
          api.setConfigValues(
            game,
            profile,
            o.path,
            Object.entries(o.edits).map(([k, value]) => {
              const [section, key] = k.split("\u001f");
              return { section, key, value };
            }),
          ),
        "Config saved",
      );
  if (!view || C.open !== o) return;
  C.open = { path: o.path, view, edits: {}, raw: o.raw, text: view.text };
  renderMain();
  loadConfigs();
}

/// The input for one BepInEx setting, by its type: a checkbox, a list of the values it
/// accepts, a number in its range, or text.
export function configInput(section, e) {
  const o = S.config.open;
  const k = configEditKey(section, e.key);
  const current = k in o.edits ? o.edits[k] : e.value;
  const set = (v) => {
    if (v === e.value) delete o.edits[k];
    else o.edits[k] = v;
    configRefreshSave();
    const reset = document.querySelector(`[data-reset="${CSS.escape(k)}"]`);
    if (reset) reset.hidden = e.default == null || v === e.default;
  };
  const label = `${section}: ${e.key}`;
  if (e.kind === "Boolean") {
    const box = h("input", { type: "checkbox", checked: current.toLowerCase() === "true", "aria-label": label });
    box.addEventListener("change", () => set(box.checked ? "true" : "false"));
    return h("label", { class: "toggle" }, box, h("span", { class: "track" }));
  }
  if (e.acceptable.length && e.acceptable.includes(current)) {
    const sel = h("select", { "aria-label": label }, e.acceptable.map((v) => h("option", { value: v, selected: v === current }, v)));
    sel.addEventListener("change", () => set(sel.value));
    return sel;
  }
  const numeric = /^(U?Int(16|32|64)|S?Byte|Single|Double|Decimal)$/.test(e.kind || "");
  const input = h("input", {
    type: numeric ? "number" : "text",
    value: current,
    "aria-label": label,
    ...(numeric && { step: /Int|Byte/.test(e.kind) ? "1" : "any" }),
    ...(numeric && e.range && { min: e.range[0], max: e.range[1] }),
  });
  input.addEventListener("input", () => set(input.value));
  return input;
}

export function configSettings(o) {
  return o.view.sections.map((s) =>
    h(
      "section",
      { class: "config-section" },
      h("h3", {}, s.name),
      s.entries.map((e) => {
        const k = configEditKey(s.name, e.key);
        const current = k in o.edits ? o.edits[k] : e.value;
        const notes = [
          e.kind && `Type: ${e.kind}`,
          e.range && `From ${e.range[0]} to ${e.range[1]}`,
          e.default != null && `Default: ${e.default || "(empty)"}`,
        ].filter(Boolean);
        return h(
          "div",
          { class: "config-entry" },
          h("div", { class: "config-key" }, e.key),
          h("div", { class: "config-value" }, configInput(s.name, e)),
          h(
            "div",
            { class: "config-desc" },
            e.description && h("div", {}, e.description),
            h(
              "div",
              { class: "muted small" },
              notes.join(" · "),
              e.default != null &&
                h(
                  "button",
                  {
                    class: "link",
                    hidden: current === e.default,
                    "data-reset": k,
                    onclick: () => {
                      if (e.default === e.value) delete o.edits[k];
                      else o.edits[k] = e.default;
                      renderMain();
                    },
                  },
                  " Reset",
                ),
            ),
          ),
        );
      }),
    ),
  );
}

export function configEditor(o) {
  const C = S.config;
  const playing = S.session.phase !== "idle" && S.session.game === S.gameId;
  const f = o.view.file;
  const area = h("textarea", { class: "config-text", spellcheck: "false", "aria-label": f.path }, o.text);
  area.addEventListener("input", () => {
    o.text = area.value;
    configRefreshSave();
  });
  return h(
    "section",
    { class: "panel" },
    h(
      "div",
      { class: "ts-toolbar" },
      h(
        "button",
        {
          class: "ghost small",
          onclick: async () => {
            if (!(await configLeave())) return;
            C.open = null;
            renderMain();
          },
        },
        "← Configs",
      ),
      h("strong", { class: "collection-name" }, f.name),
      f.version && h("span", { class: "muted small" }, `v${f.version}`),
      h("span", { class: "spacer" }),
      o.view.sections &&
        h(
          "button",
          {
            class: "ghost small",
            onclick: async () => {
              if (configDirty() && !(await configLeave())) return;
              C.open = { ...o, edits: {}, text: o.view.text, raw: !o.raw };
              renderMain();
            },
          },
          o.raw ? "Edit settings" : "Edit as text",
        ),
      h(
        "button",
        {
          id: "config-revert",
          class: "ghost small",
          disabled: !configDirty(),
          onclick: () => {
            C.open = { ...o, edits: {}, text: o.view.text };
            renderMain();
          },
        },
        "Revert",
      ),
      h("button", { id: "config-save", class: "primary small", disabled: !configDirty(), onclick: saveConfig }, "Save"),
    ),
    h("p", { class: "muted small path-text" }, f.path, f.guid && ` · ${f.guid}`),
    playing && h("div", { class: "ts-banner warn" }, "The game is running: a plugin may write over your changes when it saves its settings or closes."),
    o.raw ? area : configSettings(o),
  );
}

export function configTab(d) {
  const C = S.config;
  if (C.open) return configEditor(C.open);
  const list = C.list;
  const q = C.query.trim().toLowerCase();
  const files = (list?.files || []).filter((f) => !q || [f.name, f.path, f.guid || ""].some((s) => s.toLowerCase().includes(q)));
  const search = h("input", { type: "search", placeholder: "Search configs", "aria-label": "Search configs", value: C.query, "data-store-search": "" });
  search.addEventListener("input", () => {
    C.query = search.value;
    renderMain();
    const again = $("[data-store-search]");
    if (again) {
      again.focus();
      again.setSelectionRange(again.value.length, again.value.length);
    }
  });
  const row = (f) =>
    h(
      "div",
      { class: "config-row" },
      h(
        "div",
        { class: "mod-main" },
        h("div", { class: "mod-name" }, f.name, f.version && h("span", { class: "version" }, f.version)),
        h("div", { class: "mod-meta" }, h("span", { class: "path-text" }, f.path), f.modifiedUnix ? h("span", {}, `changed ${new Date(f.modifiedUnix * 1000).toLocaleString()}`) : null),
      ),
      h("button", { class: "small", onclick: () => openConfig(f.path) }, f.bepinex ? "Edit…" : "Edit as text…"),
    );
  return h(
    "section",
    { class: "panel" },
    h(
      "div",
      { class: "ts-toolbar" },
      search,
      h("button", { class: "ghost small", disabled: !list?.files?.length, onclick: () => attempt(() => api.openFolder(list.dir)), title: "Show the profile's runtime folder" }, "Show folder"),
      h("button", { class: "ghost small", onclick: loadConfigs }, "Refresh"),
    ),
    h("p", { class: "muted small" }, `The mod loader's config files in the profile “${S.profileName}”. Your edits stay: UMM never writes over a config file you changed.`),
    !list
      ? h("div", { class: "loading" }, h("span", { class: "spinner" }), "Loading configs…")
      : list.error
        ? h("p", { class: "muted" }, "The config files could not be listed.")
        : !list.files.length
          ? h("p", { class: "muted" }, "No config files yet. Mods generate default configs the first time the game runs with them in this profile.")
          : !files.length
            ? h("p", { class: "muted" }, "No configs match.")
            : h("div", { class: "config-list" }, files.map(row)),
  );
}

export function filesTab() {
  if (S.plan === "loading" || S.plan === null)
    return h("section", { class: "panel" }, h("div", { class: "loading" }, h("span", { class: "spinner" }), "Building the plan…"));
  const p = S.plan;
  const runtimeDirs = {
    bepinex: "BepInEx/",
    shimloader: "shimloader/",
    melonloader: "MelonLoader/, Mods/, Plugins/, UserData/ and UserLibs/",
    "recursive-melonloader": "MelonLoader/, Mods/, Plugins/, UserData/ and UserLibs/",
    lovely: "mods/",
    gdweave: "GDWeave/",
    godotml: "mods/ and addons/",
    "return-of-modding": "ReturnOfModding/",
    umm: "UMM/",
    northstar: "R2Northstar/",
    mio: "mods/ and modconfig/",
    rivet: "Rivet/",
    bepisloader: "BepInEx/ and Renderer/",
  }[S.details.game.loader];
  const runtimeNote =
    runtimeDirs && ["doorstop", "args"].includes(S.details.game.inject)
      ? h("p", { class: "muted" }, `Files under ${runtimeDirs} go to this profile's runtime folder, not the game folder.`)
      : null;
  return h(
    "section",
    { class: "panel" },
    h(
      "div",
      { class: "stats" },
      stat(p.files.length, "files to deploy"),
      stat(p.conflicts.length, "conflicts", p.conflicts.length ? "warn" : ""),
      stat(p.skipped.length, "skipped"),
    ),
    runtimeNote,
    h("h3", {}, "Conflicts"),
    p.conflicts.length
      ? table(["File", "Used", "Overridden"], p.conflicts.map((c) => [h("code", {}, c.target), h("strong", {}, c.winner), c.overridden.join(", ")]))
      : h("p", { class: "muted" }, "No two enabled mods ship the same file."),
    p.skipped.length > 0 && [
      h("h3", {}, "Skipped files"),
      table(["Mod", "File", "Why"], p.skipped.map((s) => [s.owner, h("code", {}, s.file), s.reason])),
    ],
    p.settings?.length > 0 && [
      h("h3", {}, "Game settings"),
      h("p", { class: "muted" }, "Set while mods are deployed, and put back afterwards. The rest of each file is left as the game keeps it."),
      table(
        ["File", "Setting", "Now"],
        p.settings.map((s) => [
          h("code", {}, s.path),
          h("code", {}, `[${s.section}] ${s.key}=${s.value}`),
          !s.exists
            ? h("span", { class: "muted" }, s.create ? "file created while deployed" : "the game has not written this file yet; left alone")
            : s.current === s.value
              ? h("span", { class: "muted" }, "already set")
              : s.current == null
                ? "not set"
                : `${s.key}=${s.current}`,
        ]),
      ),
    ],
    p.plugins && [
      h("h3", {}, p.plugins.kind === "mods" ? "Mod list" : "Plugins"),
      h(
        "p",
        { class: "muted" },
        p.plugins.kind === "mods"
          ? "This game uses a list to enable your mods and set their load order."
          : "This game uses a list to enable plugins included with your mods and set their load order.",
        " While you play, UMM adds its mods to the end of your list, if you have one.",
        p.plugins.kind === "dated" ? " Note: this game loads plugins in the order of their file dates." : "",
        " ",
        h("code", {}, p.plugins.path),
      ),
      table(
        ["", p.plugins.kind === "mods" ? "Mod" : "Plugin", "From"],
        p.plugins.entries.map((e, i) => [
          h("span", { class: "muted" }, i + 1),
          e.enabled ? h("code", {}, e.name) : h("code", { class: "muted", title: "Listed but not enabled" }, `${e.name} (off)`),
          e.owner || h("span", { class: "muted" }, "your list"),
        ]),
      ),
    ],
    h(
      "details",
      { class: "all-files" },
      h("summary", {}, `All ${p.files.length} files`),
      table(["Game path", "From", ""], p.files.map((f) => [h("code", {}, f.target), f.owner, f.writable ? h("span", { class: "chip" }, "copied") : ""])),
    ),
  );
}

export function stat(n, label, kind = "") {
  return h("div", { class: `stat ${kind}` }, h("div", { class: "n" }, n), h("div", { class: "l" }, label));
}

export function table(headers, rows) {
  return h(
    "div",
    { class: "table-wrap" },
    h("table", {}, h("thead", {}, h("tr", {}, headers.map((x) => h("th", {}, x)))), h("tbody", {}, rows.map((r) => h("tr", {}, r.map((c) => h("td", {}, c)))))),
  );
}

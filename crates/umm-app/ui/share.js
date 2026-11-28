import { api } from "./api.js";
import { refreshAll } from "./data.js";
import { $, fmtBytes, h } from "./dom.js";
import { modal } from "./modals.js";
import { S, attempt, toast } from "./state.js";
import { renderMain } from "./tabs/gamebanana.js";
import { loadTs } from "./tabs/thunderstore.js";

// ----- r2modman profiles: share and import -----

export async function shareProfile() {
  const game = S.gameId;
  const profile = S.profileName;
  const e = await attempt(() => api.exportPreview(game, profile));
  if (!e) return;
  const body = [
    h("p", {}, `${e.mods} mod${e.mods === 1 ? "" : "s"} and ${e.files.length} config file${e.files.length === 1 ? "" : "s"} (${fmtBytes(e.size)}).`),
    e.leftOut.length > 0 && h("p", { class: "muted" }, `Left out, because they are not from Thunderstore: ${e.leftOut.join(", ")}.`),
    e.files.length === 0 && h("p", { class: "muted" }, "No config files yet: the mod loader and the mods write them the first time the game runs with this profile."),
    h("p", { class: "muted" }, "Save as file writes an .r2z file on this computer. Nothing is uploaded."),
    h("p", { class: "muted" }, "Get a code uploads the profile, configs included, to Thunderstore. Anyone with the code can download it, and Thunderstore may delete it after a while."),
  ];
  const choice = await modal({
    title: `Share the profile “${profile}”`,
    body,
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Save as file…", value: "file", class: "" },
      { label: "Get a code", value: "code", class: "primary" },
    ],
  });
  if (choice === "file") {
    const path = await attempt(() => api.exportFile(game, profile));
    if (path) toast(`Saved ${path}`, "ok");
  } else if (choice === "code") {
    const code = await attempt(() => api.exportCode(game, profile));
    if (!code) return;
    const field = h("input", { type: "text", value: code, readonly: true, "aria-label": "Profile code" });
    const copy = await modal({
      title: "Profile code",
      body: [h("p", {}, "Anyone can import this profile in UMM or r2modman with this code."), field],
      actions: [
        { label: "Close", value: null, class: "ghost" },
        { label: "Copy", value: "copy", class: "primary", focus: true },
      ],
    });
    if (copy === "copy") {
      try {
        await navigator.clipboard.writeText(code);
        toast("Code copied", "ok");
      } catch {
        field.select();
        toast("Select the code and copy it", "info");
      }
    }
  }
}

export function importSummary(r) {
  const n = r.installed.installed.length;
  toast(`Imported into the new profile “${r.profile}”: ${n} package${n === 1 ? "" : "s"} downloaded, ${r.files} config file${r.files === 1 ? "" : "s"}`, "ok");
  for (const m of r.notes) toast(m, "info");
  for (const x of r.installed.missing) toast(`${x} is no longer on Thunderstore; skipped`, "error");
}

export async function importProfile() {
  const game = S.gameId;
  const field = h("input", { type: "text", placeholder: "Profile code", "aria-label": "Profile code" });
  const choice = await modal({
    title: "Import an r2modman profile",
    body: [h("p", {}, "Paste a profile code, or pick an .r2z file. The mods are installed into a new profile."), field],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "From a file…", value: "file", class: "" },
      { label: "Import code", value: "code", class: "primary" },
    ],
  });
  if (!choice) return;
  S.ts.busy = { message: "Importing…" };
  renderMain();
  const r = choice === "code" ? await attempt(() => api.importCode(game, field.value.trim())) : await attempt(() => api.importFile(game));
  S.ts.busy = null;
  if (r) {
    importSummary(r);
    S.profileName = r.profile;
  }
  await refreshAll();
  if (S.tab === "thunderstore") await loadTs(false);
}

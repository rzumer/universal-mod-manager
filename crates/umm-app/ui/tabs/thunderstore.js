import { api } from "../api.js";
import { refreshAll } from "../data.js";
import { $, fmtBytes, h } from "../dom.js";
import { banner, modal } from "../modals.js";
import { S, attempt, toast } from "../state.js";
import { renderMain } from "./gamebanana.js";
import { storePage, storeRun } from "./store.js";

// ----- Thunderstore tab -----

/// Loads the Thunderstore list from the top. `reset` also shows the loading state when the
/// game changed; `append` (Show more) adds the page at the current offset instead.
export async function loadTs(reset, append = false) {
  const game = S.gameId;
  if (!append) S.ts.query.offset = 0;
  if (reset) {
    // A new search keeps showing the last results until the new ones arrive.
    if (S.ts.game !== game) S.ts.page = null;
    S.ts.game = game;
    renderMain();
  }
  const page = await attempt(() => api.tsSearch(game, S.ts.query));
  if (S.gameId !== game) return;
  S.ts.page = storePage(S.ts.page, page, append);
  if (page && !append) S.ts.stale = false;
  if (S.tab === "thunderstore") renderMain();
}

export function fmtCount(n) {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`;
  return String(n);
}

/// Runs a Thunderstore install. `call(choice)` installs; when pinned packages are in the way
/// (older than another package of the profile needs), nothing is installed and the user
/// chooses to downgrade those packages or remove the pins, and it runs again.
export async function tsRun(title, call) {
  let r = await storeRun(S.ts, title, () => call(null));
  while (r?.conflicts?.length) {
    const choice = await pinConflictDialog(r.conflicts);
    if (!choice) {
      r = null;
      break;
    }
    r = await storeRun(S.ts, title, () => call(choice));
  }
  if (r) {
    const names = r.installed.map((i) => (i.replaced ? `${i.fullName} ${i.replaced} → ${i.version}` : `${i.fullName} ${i.version}`));
    toast(names.length ? `Installed ${names.join(", ")}` : "Everything needed was already installed", "ok");
    for (const m of r.missing) toast(`Dependency ${m} is no longer on Thunderstore; skipped`, "error");
    for (const w of r.warnings || []) toast(w, "error");
    for (const n of r.notes || []) toast(n, "info");
  }
  await refreshAll();
  await loadTs(false);
}

/// Asks what to do about pinned packages older than other packages of the profile need:
/// "downgrade" those packages, "unpin" (when every pin in the way can go), or null.
export async function pinConflictDialog(conflicts) {
  const short = (n) => n.split("-").slice(1).join("-").replaceAll("_", " ");
  const dependents = [...new Set(conflicts.map((c) => short(c.dependent)))];
  const canUnpin = conflicts.every((c) => c.canUnpin);
  return modal({
    title: "Pinned versions are in the way",
    body: [
      h("p", { class: "muted" }, `In “${S.profileName}”:`),
      h(
        "ul",
        {},
        conflicts.map((c) =>
          h("li", {}, `${short(c.dependent)} ${c.dependentVersion} needs ${short(c.pinned)} ${c.needs} or later, but ${c.canUnpin ? "this profile is pinned to" : "you are pinning it to"} ${c.pin}.`),
        ),
      ),
      h(
        "p",
        { class: "muted" },
        canUnpin
          ? `Downgrade ${dependents.join(", ")} to versions that work with the pins (pinned there), or remove the pins so those packages follow their latest version.`
          : `Downgrade ${dependents.join(", ")} to versions that work with the pins; they are pinned there. Other profiles keep theirs.`,
      ),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      ...(canUnpin ? [{ label: "Remove the pins", value: "unpin" }] : []),
      { label: `Downgrade ${dependents.length === 1 ? dependents[0] : "them"}`, value: "downgrade", class: "primary" },
    ],
  });
}

/// Makes the profile follow a package's latest version again, removing its pin.
export const tsFollowLatest = (fullName) =>
  tsRun("Following the latest version…", (c) => api.tsFollowLatest(S.gameId, fullName, S.profileName, c));

/// The version the current profile is pinned to for a package, if any.
export function pinnedVersion(fullName) {
  const m = (S.details?.mods || []).find((x) => x.thunderstore === fullName);
  return m ? S.profile?.mods.find((e) => e.id === m.id)?.version || null : null;
}

export const REVIEW_TEXT = {
  rejected: "This game's Thunderstore moderators rejected it for this game.",
  unreviewed: "It is waiting for this game's Thunderstore moderators to review it.",
};

// Installs a package by Owner-Name. One the game's list does not show (not submitted for this
// game, unreviewed, or rejected by its moderators) is installed only after saying so.
export async function installByName(prefill) {
  const game = S.gameId;
  const field = h("input", { type: "text", placeholder: "Owner-Name or Owner-Name@1.2.3", "aria-label": "Package", value: prefill || "" });
  const v = await modal({
    title: "Install an unlisted package",
    body: [
      h("p", { class: "muted" }, "A Thunderstore package this game's list does not show, by the name on its Thunderstore page (Owner-Name). Its dependencies are installed too."),
      field,
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Look up", value: "go", class: "primary", focus: false },
    ],
  });
  const text = field.value.trim();
  if (v !== "go" || !text) return;
  const [name, version] = text.split("@");
  const found = await attempt(() => api.tsLookup(game, name));
  if (!found) return;
  if (found.listed) return tsInstall([text]);
  const p = found.package;
  const why = REVIEW_TEXT[found.reviewStatus] || "It is not listed for this game.";
  const ok = await modal({
    title: `${p.name.replaceAll("_", " ")} is not listed for this game`,
    body: [
      h("p", {}, `${p.fullName} ${version || p.latest}: ${p.description}`),
      h("p", { class: "banner warn" }, `${why} UMM installs it only because you ask, and it will not offer updates for it.`),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "Install anyway", value: "go", class: "danger" },
    ],
  });
  if (ok !== "go") return;
  await tsRun("Installing…", (c) => api.tsInstallUnlisted(game, p.fullName, version, S.profileName, c));
}

/// Compares two version numbers part by part ("1.10.0" comes after "1.9.2").
export function compareVersions(a, b) {
  const pa = String(a).split(/[.\-+]/);
  const pb = String(b).split(/[.\-+]/);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const x = parseInt(pa[i] ?? "0", 10) || 0;
    const y = parseInt(pb[i] ?? "0", 10) || 0;
    if (x !== y) return x - y;
  }
  return 0;
}

/// Pins the current profile to another version of an installed package, older or newer;
/// other profiles keep theirs. A version older than another package of the profile needs
/// can be picked too: those packages are then downgraded, after asking.
export async function tsVersionsDialog(it) {
  const game = S.gameId;
  const versions = await attempt(() => api.tsVersions(game, it.fullName, S.profileName));
  if (!versions?.length) return;
  const current = versions.find((v) => v.installed)?.version || it.installed;
  const pinned = versions.find((v) => v.pinned)?.version || null;
  // Picking the version already pinned changes nothing; picking the one in use pins it.
  let chosen = pinned || current;
  const label = () => (chosen === pinned ? "Pinned" : `Pin to ${chosen}`);
  const refresh = () => {
    const go = $("#modal-root .modal-actions button.primary");
    if (go) {
      go.textContent = label();
      go.disabled = chosen === pinned;
    }
  };
  const rows = versions.map((v, i) => {
    const older = v.neededBy.length > 0;
    return h(
      "label",
      { class: `version-row ${older ? "blocked" : ""}` },
      h("input", {
        type: "radio",
        name: "ts-version",
        value: v.version,
        checked: v.version === chosen,
        onchange: () => {
          chosen = v.version;
          refresh();
        },
      }),
      h("span", { class: "version-num" }, v.version),
      i === 0 && h("span", { class: "chip" }, "Latest"),
      v.pinned ? h("span", { class: "chip ok" }, "Pinned") : v.installed && h("span", { class: "chip ok" }, "In use"),
      v.size && !v.stored ? h("span", { class: "muted small version-size" }, fmtBytes(v.size)) : null,
      older && h("span", { class: "muted small version-why" }, `${v.neededBy.map((n) => `${n.name} needs ${n.version} or later`).join(", ")}: picking this downgrades ${v.neededBy.length === 1 ? "it" : "them"}`),
    );
  });
  const pending = modal({
    title: `${it.name.replaceAll("_", " ")} versions`,
    body: [
      h(
        "p",
        { class: "muted" },
        `Pin “${S.profileName}” to a version. Other profiles keep theirs, and a pinned version is not updated with the others. ${pinned ? `This profile is pinned to ${pinned}.` : "This profile follows the latest installed version."}`,
      ),
      h("div", { class: "version-list", role: "radiogroup", "aria-label": "Versions" }, rows),
    ],
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      ...(pinned ? [{ label: "Follow latest", value: "follow" }] : []),
      { label: label(), value: "go", class: "primary" },
    ],
  });
  refresh();
  const v = await pending;
  if (v === "follow") return tsFollowLatest(it.fullName);
  if (v !== "go" || chosen === pinned) return;
  await tsRun(compareVersions(chosen, current) < 0 ? "Downgrading…" : "Pinning…", (c) =>
    api.tsInstallVersion(game, it.fullName, chosen, S.profileName, c),
  );
}

export const tsInstall = (pkgs) => tsRun("Installing…", (c) => api.tsInstall(S.gameId, pkgs, S.profileName, c));
export const tsUpdate = (names) => tsRun("Updating…", (c) => api.tsUpdate(S.gameId, names, S.profileName, c));

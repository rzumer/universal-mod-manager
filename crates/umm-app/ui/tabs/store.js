import { $, h, icon } from "../dom.js";
import { banner } from "../modals.js";
import { header, tabs } from "../panel.js";
import { S, attempt } from "../state.js";
import { action, renderMain } from "./gamebanana.js";

// ----- Store tabs (Thunderstore, Nexus Mods): the parts they share -----
//
// Each store tab is a search over a remote list, rows with an install or update button, and
// banners above. These build the common parts; each tab supplies what is its own.

export let storeSearchTimer = null;

/// A search box that calls `onSearch` once typing pauses, keeping the caret at the end of
/// the box when the tab is drawn again.
export function storeSearch(placeholder, query, onSearch, delay = 300) {
  const box = h("input", { type: "search", placeholder, "aria-label": placeholder, value: query.text, "data-store-search": "" });
  box.addEventListener("input", () => {
    query.text = box.value;
    clearTimeout(storeSearchTimer);
    storeSearchTimer = setTimeout(async () => {
      await onSearch();
      const again = $("[data-store-search]");
      if (again) {
        again.focus();
        again.setSelectionRange(again.value.length, again.value.length);
      }
    }, delay);
  });
  return box;
}

/// A drop-down of `[value, label]` options, calling `onChange` with the value picked.
export function storeSelect(label, options, value, onChange) {
  const el = h("select", { "aria-label": label }, options.map(([v, l]) => h("option", { value: v, selected: v === value }, l)));
  el.addEventListener("change", () => onChange(el.value));
  return el;
}

/// A checkbox with its label, calling `onChange` with its state.
export function storeCheck(label, checked, onChange) {
  const el = h("input", { type: "checkbox", checked });
  el.addEventListener("change", () => onChange(el.checked));
  return h("label", { class: "check inline" }, el, label);
}

/// The banners every store tab shows: what it is doing now, and that installs wait while
/// mods are deployed.
export function storeBanners(state, g, playing) {
  return [
    state.busy && h("div", { class: "ts-banner live" }, h("span", { class: "spinner" }), state.busy.message),
    (g.deployed || playing) && h("div", { class: "ts-banner warn" }, "Mods are deployed; installing and updating wait until the game folder is restored."),
  ].filter(Boolean);
}

/// A banner with a message and, when given, a button.
export function storeBanner(text, button, warn = false) {
  return h("div", { class: `ts-banner ${warn ? "warn" : ""}` }, h("span", {}, text), button || null);
}

/// The list of a store tab: loading, failed (with a retry), empty, or its rows.
export function storeList(page, { loading, noun, retry, row }) {
  if (!page) return h("div", { class: "loading" }, h("span", { class: "spinner" }), loading);
  if (page.error) return h("p", { class: "muted" }, `The ${noun} list could not be loaded.`, " ", h("button", { class: "link", onclick: retry }, "Try again"));
  if (!page.items.length) return h("p", { class: "muted" }, `No ${noun}s match.`);
  return h("div", { class: "ts-list" }, page.items.map(row));
}

/// One package or mod in a store list: its picture, its name (opening its page), who made
/// it and which version, what it is, its numbers, and what can be done with it.
export function storeRow({ image, imageClass = "", name, site, onOpen, by, version, chips = [], text, meta, action, dim = false }) {
  return h(
    "div",
    { class: `ts-row ${dim ? "deprecated" : ""}` },
    image ? h("img", { class: `ts-icon ${imageClass}`, src: image, alt: "", loading: "lazy" }) : h("div", { class: `ts-icon ${imageClass}` }),
    h(
      "div",
      { class: "ts-main" },
      h(
        "div",
        { class: "ts-title" },
        h("button", { class: "link ts-name", title: `Open its page on ${site}`, onclick: onOpen }, name, icon("external")),
        by && h("span", { class: "muted" }, ` by ${by}`),
        version && h("span", { class: "muted" }, ` · ${version}`),
        chips,
      ),
      h("div", { class: "ts-desc" }, text),
      h("div", { class: "ts-meta muted" }, meta.filter(Boolean).join(" · ")),
    ),
    action,
  );
}

/// "Show more", when the list has more than it shows.
export function storeMore(page, onMore) {
  if (!page || page.error || page.items.length >= page.total) return null;
  return h("div", { class: "ts-more" }, h("button", { class: "ghost", onclick: onMore }, "Show more"));
}

/// A store tab's panel: a header line, the toolbar, banners, a count line, the list, and
/// "Show more".
export function storeTab({ header, toolbar, banners, count, list, more }) {
  return h("section", { class: "panel" }, h("div", { class: "ts-share" }, header), h("div", { class: "ts-toolbar" }, toolbar), banners, count && h("p", { class: "muted small" }, count), list, more);
}

/// Runs an install or update with its progress at the top of the store tab.
export async function storeRun(state, title, call) {
  state.busy = { message: title };
  renderMain();
  const r = await attempt(call);
  state.busy = null;
  return r;
}

/// Shows a store's install progress in its busy banner.
export function storeProgress(state, e) {
  if (state.busy && e.game === S.gameId) {
    state.busy.message = e.message;
    const b = $(".ts-banner.live");
    if (b) b.lastChild.textContent = e.message;
  }
}

/// A page of results, after the ones shown when `append` (Show more).
export function storePage(old, page, append) {
  if (!page) return { error: true, items: [], total: 0, categories: [], updates: [] };
  if (append && old && !old.error) return { ...page, items: [...old.items, ...page.items] };
  return page;
}

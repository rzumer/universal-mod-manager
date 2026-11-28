import { api } from "../api.js";
import { workshopShown } from "../data.js";
import { $, fmtBytes, h, plural } from "../dom.js";
import { header } from "../panel.js";
import { importProfile, shareProfile } from "../share.js";
import { S, attempt, toast } from "../state.js";
import { action, renderMain } from "./gamebanana.js";
import { storeBanner, storeBanners, storeCheck, storeList, storeMore, storeRow, storeSearch, storeSelect, storeTab } from "./store.js";
import { fmtCount, installByName, loadTs, pinnedVersion, tsFollowLatest, tsInstall, tsUpdate, tsVersionsDialog } from "./thunderstore.js";

// ----- Workshop tab -----

/// Loads the game's Workshop items (`refresh` asks Steam for their details again; `quiet`
/// keeps the list shown until the new one is there).
export async function loadWorkshop(refresh, quiet = false) {
  const game = S.gameId;
  S.workshop = { ...S.workshop, game, list: quiet && S.workshop.game === game ? S.workshop.list : "loading" };
  renderMain();
  const list = await attempt(() => api.workshopItems(game, refresh));
  if (S.gameId !== game) return;
  S.workshop.list = workshopMerge(game, list || { error: true, items: [] });
  renderMain();
}

/// Steam's record of the game's items, with the subscriptions made here that it does not show
/// yet; those it shows are no longer pending.
export function workshopMerge(game, list) {
  const pend = S.workshop.pending[game];
  if (!pend || !Array.isArray(list?.items)) return list;
  const items = list.items.map((i) => {
    const p = pend[i.id];
    if (p === undefined) return i;
    if (i.subscribed === !!p) {
      delete pend[i.id];
      return i;
    }
    return { ...i, subscribed: !!p };
  });
  for (const [id, p] of Object.entries(pend)) {
    if (list.items.some((i) => i.id === id)) continue;
    if (p) items.push(p);
    else delete pend[id];
  }
  return { ...list, items };
}

/// Searches the game's Workshop (`append`: the next page, after the ones shown).
export function workshopBrowseKey(game, q) {
  return JSON.stringify([game, q.text.trim(), q.sort, q.tag]);
}

export async function loadWorkshopBrowse(append = false) {
  const game = S.gameId;
  const W = S.workshop;
  const q = W.query;
  q.page = append && W.found?.items ? W.found.page + 1 : 1;
  W.foundKey = workshopBrowseKey(game, q);
  if (!append) W.found = "loading";
  else W.more = true;
  renderMain();
  const page = await attempt(() => api.workshopBrowse(game, q));
  if (S.gameId !== game) return;
  W.more = false;
  if (!page) W.found = append && W.found?.items ? W.found : { error: true, items: [] };
  else if (append && W.found?.items) W.found = { ...page, items: [...W.found.items, ...page.items] };
  else W.found = page;
  renderMain();
}

/// Waits for the answer to a Workshop action from Steam's page: `ok`, `login`, `error`, or
/// `timeout` when none comes.
export const workshopWaiting = new Map();
/// Steam's reason for the last answer that was not `ok`, when it gave one.
export let workshopLastWhy = "";
export function workshopAnswer(e) {
  const key = `${e.game}/${e.action}/${e.item}`;
  if (e.why) console.warn(`Steam Workshop ${e.action} ${e.item}: ${e.why}`);
  workshopLastWhy = e.why || "";
  workshopWaiting.get(key)?.(e.result);
  workshopWaiting.delete(key);
}
export let workshopQueue = Promise.resolve();
export function workshopAct(game, action, item = "") {
  const run = workshopQueue.then(() => workshopActNow(game, action, item));
  workshopQueue = run.catch(() => {});
  return run;
}
export async function workshopActNow(game, action, item) {
  const key = `${game}/${action}/${item}`;
  const answer = new Promise((resolve) => {
    workshopWaiting.set(key, resolve);
    setTimeout(() => {
      if (workshopWaiting.get(key) === resolve) {
        workshopWaiting.delete(key);
        resolve("timeout");
      }
    }, 30000);
  });
  try {
    await api.workshopAction(game, action, item || null);
  } catch (e) {
    workshopWaiting.delete(key);
    toast(String(e), "error");
    return "error";
  }
  return answer;
}

/// Whether the user is signed in to Steam (in UMM's Steam windows), asked once per game.
export async function checkSteamSignIn() {
  const W = S.workshop;
  if (W.signInChecked) return;
  W.signInChecked = true;
  const r = await workshopAct(S.gameId, "status");
  if (W.signedIn === null) W.signedIn = r === "ok" ? true : r === "login" ? false : null;
  renderMain();
}

/// Opens Steam's sign-in page in UMM's Steam Workshop window. With an item, subscribes to it
/// once the user is signed in.
export async function steamSignIn(g, item = null) {
  const W = S.workshop;
  W.afterSignIn = item ? { game: g.id, item } : null;
  const opened = await attempt(() => api.openSteamSignIn(g.id));
  if (opened === undefined) {
    W.afterSignIn = null;
    return;
  }
  toast(
    item
      ? `Sign in to Steam in the window that opened, and UMM then subscribes to ${item.title}. UMM never sees your password.`
      : "Sign in to Steam in the window that opened. UMM never sees your password.",
  );
}

/// A page shown in the Steam Workshop window: if a sign-in is awaited, whether it is done.
export async function steamWindowPage() {
  const W = S.workshop;
  const wait = W.afterSignIn;
  if (!wait && W.signedIn !== false) return;
  const r = await workshopAct(wait ? wait.game : S.gameId, "status");
  if (r !== "ok") return;
  W.signedIn = true;
  if (wait && W.afterSignIn === wait) {
    W.afterSignIn = null;
    const g = S.games.find((x) => x.id === wait.game);
    if (g) await workshopSubscribe(g, wait.item, true);
  }
  renderMain();
}

/// Subscribes to an item (or unsubscribes), through Steam's page with the user's session.
export async function workshopSubscribe(g, item, on) {
  const W = S.workshop;
  W.busy = { ...(W.busy || {}), [item.id]: on ? "Subscribing…" : "Unsubscribing…" };
  renderMain();
  const r = await workshopAct(g.id, on ? "subscribe" : "unsubscribe", item.id);
  delete W.busy[item.id];
  if (r === "ok") {
    item.subscribed = on;
    W.signedIn = true;
    toast(on ? `Subscribed to ${item.title}. Steam downloads it while it runs.` : `Unsubscribed from ${item.title}.`, "ok");
    // Shown at once, as Steam's record will show it once Steam notes the change (UMM hears
    // of that and reads it again).
    const pend = (W.pending[g.id] ||= {});
    pend[item.id] = on && {
      id: item.id,
      subscribed: true,
      folder: null,
      size: item.fileSize || 0,
      updatedUnix: item.timeUpdated || 0,
      updatePending: false,
      title: item.title,
      description: item.description || "",
      previewUrl: item.previewUrl || null,
      tags: item.tags || [],
      subscriptions: item.subscriptions || 0,
      favorited: item.favorited || 0,
      createdUnix: 0,
    };
    if (W.game === g.id && Array.isArray(W.list?.items)) W.list = workshopMerge(g.id, W.list);
  } else if (r === "login") {
    W.signedIn = false;
    renderMain();
    await steamSignIn(g, on ? item : null);
  } else {
    const what = on ? "subscribe" : "unsubscribe";
    toast(
      r === "timeout"
        ? `Steam did not answer. Try again, or ${what} on the item's Workshop page.`
        : `Steam turned down the request${workshopLastWhy ? ` (${workshopLastWhy})` : ""}. You can ${what} on the item's Workshop page instead.`,
      "error",
      { label: "Open page", run: () => attempt(() => api.openWorkshopWindow(g.id, item.id)) },
    );
  }
  renderMain();
}

/// The Steam Workshop tab: the game's Workshop to browse and subscribe from, and the items
/// subscribed to on this machine.
export function workshopTab(d) {
  const g = d.game;
  const W = S.workshop;
  const q = W.query;
  const mine = W.subscribedOnly;
  // Searched again only when the search is not the one shown (ticking Subscribed off goes
  // back to it as it was).
  if (!mine && W.foundKey !== workshopBrowseKey(g.id, q) && W.found !== "loading" && workshopShown(g)) setTimeout(() => loadWorkshopBrowse(false));
  checkSteamSignIn();
  const pend = W.pending[g.id] || {};
  const open = (item) => attempt(() => api.openWorkshopWindow(g.id, item));
  // Subscribed: the items Steam has on this machine, filtered here; otherwise the Workshop.
  const search = () => (W.subscribedOnly ? renderMain() : loadWorkshopBrowse(false));
  const set = (key) => (v) => {
    q[key] = v;
    search();
  };
  const list = W.list;
  const local = list && list !== "loading" && Array.isArray(list.items) ? list.items : [];
  const p = W.found;
  const banners = [];
  if (mine && list?.stale) banners.push(storeBanner("Steam could not be reached: some titles and pictures may be missing or out of date.", null, true));
  const subscribeButton = (i, subscribed) => {
    const busy = W.busy?.[i.id];
    const it = { ...i, title: i.title || `Item ${i.id}` };
    return busy
      ? h("button", { class: "small", disabled: true }, h("span", { class: "spinner" }), busy)
      : subscribed
        ? h("button", { class: "ghost small", onclick: () => workshopSubscribe(g, it, false) }, "Unsubscribe")
        : h("button", { class: "primary small", onclick: () => workshopSubscribe(g, it, true) }, "Subscribe");
  };
  const found = (i) => {
    const subscribed = pend[i.id] !== undefined ? !!pend[i.id] : i.subscribed;
    return storeRow({
      image: i.previewUrl,
      name: i.title || `Item ${i.id}`,
      site: "Steam",
      onOpen: () => open(i.id),
      by: i.author,
      chips: subscribed && h("span", { class: "chip ok" }, "Subscribed"),
      text: i.description.replace(/\[\/?[a-z0-9=*]+\]/gi, "").slice(0, 200),
      meta: [`${fmtCount(i.subscriptions)} subscribers`, i.fileSize ? fmtBytes(i.fileSize) : null, i.timeUpdated ? `updated ${new Date(i.timeUpdated * 1000).toLocaleDateString()}` : null, i.tags.slice(0, 4).join(", ")],
      action: subscribeButton(i, subscribed),
    });
  };
  const here = (i) =>
    storeRow({
      image: i.previewUrl,
      name: i.title || `Item ${i.id}`,
      site: "Steam",
      onOpen: () => open(i.id),
      chips: [
        !i.subscribed && h("span", { class: "chip" }, "Not subscribed"),
        // The game downloads its own (Steam's older Workshop): nothing to warn of here.
        i.subscribed && list.gameDownloads && h("span", { class: "chip ok" }, "Subscribed"),
        i.subscribed && !i.folder && !list.gameDownloads && h("span", { class: "chip warn", title: "Steam has not downloaded it yet; it does while it runs" }, "Not downloaded"),
        i.updatePending && h("span", { class: "chip", title: "Steam has a newer version, which it downloads on its own" }, "Update pending"),
        !i.title && h("span", { class: "chip warn", title: "Steam does not show it: removed from the Workshop, or private" }, "Unavailable"),
      ],
      text: i.description.split("\n").find((l) => l.trim())?.replace(/\[\/?[a-z0-9=*]+\]/gi, "").slice(0, 200) || "",
      meta: [i.subscriptions ? `${fmtCount(i.subscriptions)} subscribers` : null, i.size ? fmtBytes(i.size) : null, i.updatedUnix ? `updated ${new Date(i.updatedUnix * 1000).toLocaleDateString()}` : null, i.tags.slice(0, 4).join(", ")],
      dim: !i.subscribed,
      action: h(
        "div",
        { class: "row" },
        i.folder && h("button", { class: "ghost small", title: i.folder, onclick: () => attempt(() => api.openFolder(i.folder)) }, "Open folder"),
        subscribeButton(i, i.subscribed),
      ),
    });
  const text = q.text.trim().toLowerCase();
  // Sorted as the Workshop sorts, from what Steam says of each item.
  const by = { trend: "subscriptions", subscribers: "subscriptions", rating: "favorited", recent: "createdUnix", updated: "updatedUnix" }[q.sort] || "subscriptions";
  const shown = local
    .filter((i) => (!text || (i.title || i.id).toLowerCase().includes(text) || i.tags.some((t) => t.toLowerCase().includes(text))) && (!q.tag || i.tags.includes(q.tag)))
    .sort((a, b) => (b[by] || 0) - (a[by] || 0) || (a.title || a.id).localeCompare(b.title || b.id));
  const tags = mine ? [...new Set([...local.flatMap((i) => i.tags), ...(q.tag ? [q.tag] : [])])].sort() : p?.tags || [];
  const subscribedCount = local.filter((i) => i.subscribed).length;
  const control =
    g.workshop === "profile"
      ? "Add them to a profile in the Mods tab to turn them on or off with it."
      : "The game loads them through Steam; they show in the Mods tab too.";
  return storeTab({
    header: [
      h("span", { class: "muted small" }, `${W.signedIn === true ? "Signed in to Steam. " : ""}${list?.gameDownloads ? "The game downloads what you subscribe to when it starts." : "Steam downloads what you subscribe to while it runs."}`),
      // Browsing needs no sign-in; subscribing does, and asks for it then too.
      W.signedIn !== true && h("button", { class: "link", onclick: () => steamSignIn(g) }, "Sign in to Steam"),
      h("button", { class: "link", onclick: () => attempt(() => api.openWorkshopPage(g.id, null)) }, "Open in the Steam client"),
    ],
    toolbar: [
      storeSearch("Search mods", q, search, mine ? 150 : 400),
        storeSelect(
          "Sort",
          [
            ["trend", "Most popular"],
            ["subscribers", "Most subscribed"],
            ["rating", "Top rated"],
            ["recent", "Most recent"],
            ["updated", "Recently updated"],
          ],
          q.sort,
          set("sort"),
        ),
      storeSelect("Tag", [["", "All tags"], ...tags.map((t) => [t, t])], q.tag, set("tag")),
      storeCheck("Subscribed", mine, (v) => {
        W.subscribedOnly = v;
        renderMain();
      }),
      h(
        "button",
        {
          class: "ghost small",
          disabled: list === "loading" || p === "loading",
          title: "Search the Workshop again, and read Steam's record of the items here",
          onclick: () => {
            loadWorkshop(true);
            if (!W.subscribedOnly) loadWorkshopBrowse(false);
          },
        },
        "Refresh",
      ),
    ],
    banners,
    count: mine
      ? list === "loading"
        ? "Reading Steam's Workshop items…"
        : list?.error
          ? "Could not read the game's Workshop items."
          : local.length
            ? `${plural(subscribedCount, "subscribed item")}${shown.length !== local.length ? ` · ${shown.length} shown` : ""}. ${control}`
            : "No Workshop items for this game on this machine yet. Untick Subscribed to browse the Workshop."
      : p === "loading"
        ? "Searching the Workshop…"
        : p?.error
          ? "The Steam Workshop could not be reached."
          : p
            ? `${fmtCount(p.total)} item${p.total === 1 ? "" : "s"}${q.text.trim() ? ` matching “${q.text.trim()}”` : ""}`
            : null,
    list: h("div", { class: "ts-list" }, mine ? shown.map(here) : (p && p !== "loading" ? p.items : []).map(found)),
    more:
      !mine && p && p !== "loading" && !p.error && p.page < p.totalPages
        ? h("div", { class: "ts-more" }, h("button", { class: "ghost", disabled: W.more, onclick: () => loadWorkshopBrowse(true) }, W.more ? "Loading…" : "Show more"))
        : null,
  });
}

export function tsTab(d) {
  const g = d.game;
  const playing = S.session.phase !== "idle" && S.session.game === g.id;
  const locked = playing || g.deployed || !!S.ts.busy;
  const q = S.ts.query;
  const p = S.ts.page;
  // Sets one search option and searches again.
  const set = (key) => (v) => {
    q[key] = v;
    loadTs(true);
  };
  const refresh = async () => {
    const fresh = await attempt(() => api.tsRefresh(g.id));
    if (fresh === false) toast("Thunderstore could not be reached; showing the last list downloaded", "error");
    loadTs(true);
  };

  const banners = storeBanners(S.ts, g, playing);
  if (p && !p.error && !p.loaderInstalled && p.loaderPackage)
    banners.push(
      storeBanner(
        `${p.loaderName} is not installed yet. It comes with the first mod you install, or install it now.`,
        h("button", { class: "primary small", disabled: locked, onclick: () => tsInstall([p.loaderPackage]) }, `Install ${p.loaderName}`),
      ),
    );
  if (p?.updates?.length)
    banners.push(
      storeBanner(
        `${plural(p.updates.length, "update")} available: ${p.updates.map((u) => u[0].split("-").slice(1).join("-")).join(", ")}`,
        h("button", { class: "small", disabled: locked, onclick: () => tsUpdate([]) }, "Update all"),
      ),
    );
  if (p?.stale) banners.push(storeBanner("Thunderstore could not be reached; this is the last list downloaded.", null, true));
  if (p?.loaderWarning)
    banners.push(
      storeBanner(
        p.loaderWarning,
        h("button", { class: "small", disabled: locked, onclick: () => installByName(p.loaderSuggestion || "") }, "Install unlisted package…"),
        true,
      ),
    );

  const row = (it) =>
    storeRow({
      image: it.icon,
      name: it.name.replaceAll("_", " "),
      site: "thunderstore.io",
      onOpen: () => attempt(() => api.openPackagePage(g.id, it.fullName)),
      by: it.owner,
      version: it.latest,
      chips: it.deprecated && h("span", { class: "chip warn" }, "Deprecated"),
      text: it.description,
      meta: [`${fmtCount(it.downloads)} downloads`, `${it.rating} likes`, it.size && fmtBytes(it.size), it.categories.join(", ")],
      dim: it.deprecated,
      action: it.installed
        ? h(
            "div",
            { class: "row" },
            pinnedVersion(it.fullName)
              ? h(
                  "button",
                  {
                    class: "chip chip-button",
                    disabled: locked,
                    title: `Pinned to ${pinnedVersion(it.fullName)} in this profile. Click to follow the latest version instead.`,
                    onclick: () => tsFollowLatest(it.fullName),
                  },
                  `Pinned ${pinnedVersion(it.fullName)}`,
                )
              : it.update
              ? h("button", { class: "small", disabled: locked, onclick: () => tsUpdate([it.fullName]) }, `Update to ${it.latest}`)
              : h("span", { class: "chip ok" }, `Installed ${it.installed}`),
            h("button", { class: "ghost small", disabled: locked, title: "Install another version, older or newer", onclick: () => tsVersionsDialog(it) }, "Versions…"),
          )
        : h("button", { class: "primary small", disabled: locked, onclick: () => tsInstall([it.fullName]) }, "Install"),
    });

  return storeTab({
    header: [
      h("span", { class: "muted small" }, "r2modman profiles:"),
      h("button", { class: "link", disabled: locked, onclick: shareProfile }, `Share “${S.profileName}”…`),
      h("button", { class: "link", disabled: locked, onclick: importProfile }, "Import a profile…"),
    ],
    toolbar: [
      storeSearch("Search mods", q, () => loadTs(true), 250),
      storeSelect("Category", [["", "All categories"], ...(p?.categories || []).map((c) => [c, c])], q.category, set("category")),
      storeSelect(
        "Sort",
        [
          ["rating", "Top rated"],
          ["downloads", "Most downloaded"],
          ["updated", "Recently updated"],
          ["name", "Name"],
        ],
        q.sort,
        set("sort"),
      ),
      storeCheck("Installed", q.installedOnly, set("installedOnly")),
      h("button", { class: "ghost small", onclick: refresh, title: "Download the package list again" }, "Refresh"),
      h("button", { class: "ghost small", disabled: locked, onclick: () => installByName(""), title: "Install a Thunderstore package this game's list does not show" }, "Install unlisted package…"),
    ],
    banners,
    count: p && !p.error && `${plural(p.total, "package")} · installs into the profile “${S.profileName}” with dependencies`,
    list: storeList(p, { loading: "Loading packages… the first time can take a while for big communities.", noun: "package", retry: () => loadTs(true), row }),
    more: storeMore(p, () => {
      q.offset = p.items.length;
      loadTs(false, true);
    }),
  });
}

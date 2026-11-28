import { api } from "./api.js";
import { addFromSteam, addManually, hiddenNotes, setHiddenNotes } from "./addgame.js";
import { loadGame, reloadDefinitions } from "./data.js";
import { $, h, icon } from "./dom.js";
import { linkSettings } from "./links.js";
import { gameStatus } from "./sidebar.js";
import { S, attempt, toast } from "./state.js";

// ----- Icons -----

/// Icons fetched so far, by `game/<id>` or `mod/<game>/<id>`: a `data:` URL, `null` for none,
/// or the pending fetch.
export const icons = new Map();

/// Asks again for the icons that were not there (a game installed since, a mod's picture
/// fetched when Nexus Mods was unreachable).
export function forgetMissingIcons() {
  for (const [k, v] of icons) if (v === null) icons.delete(k);
}

/// An icon fetched once by `load`: shown as soon as it is known, with a tile of `name`'s
/// first letter standing in while it loads, and when there is none.
export function iconImage(key, load, name, cls) {
  const got = icons.get(key);
  if (typeof got === "string") return h("img", { class: cls, src: got, alt: "" });
  const tile = h("span", { class: `${cls} icon-tile`, "aria-hidden": "true" }, (name || "?").trim().charAt(0).toUpperCase());
  if (got === null) return tile;
  let pending = got;
  if (!pending) {
    pending = load()
      .catch(() => null)
      .then((url) => {
        icons.set(key, url || null);
        return url || null;
      });
    icons.set(key, pending);
  }
  pending.then((url) => url && tile.isConnected && tile.replaceWith(h("img", { class: cls, src: url, alt: "" })));
  return tile;
}

export const gameIcon = (g) => iconImage(`game/${g.id}`, () => api.gameIcon(g.id), g.name, "game-icon");
export const modIcon = (game, id, name) => iconImage(`mod/${game}/${id}`, () => api.modIcon(game, id), name, "mod-icon");
export const workshopIcon = (item, name) => iconImage(`workshop/${item.id}`, () => Promise.resolve(item.previewUrl || null), name, "mod-icon");

/// A profile entry's Workshop item id (`workshop:<id>`), if it names one.
export const workshopId = (id) => (id.startsWith("workshop:") ? id.slice(9) : null);

export function renderSidebar() {
  const list = S.games.length
    ? h(
        "ul",
        { class: "game-list" },
        S.games.map((g) => {
          const [kind, text] = gameStatus(g);
          return h(
            "li",
            {},
            h(
              "button",
              {
                class: `game-item ${g.id === S.gameId ? "selected" : ""}`,
                "aria-current": g.id === S.gameId ? "true" : false,
                onclick: () => {
                  S.gameId = g.id;
                  S.profileName = "Default";
                  renderSidebar();
                  loadGame();
                },
              },
              gameIcon(g),
              h("span", { class: "game-text" }, h("span", { class: "game-name" }, g.name), h("span", { class: `status ${kind}` }, h("i", {}), text)),
            ),
          );
        }),
      )
    : h(
        "div",
        { class: "sidebar-empty" },
        h("strong", {}, "No games yet"),
        h("p", {}, "Pick the games you want to mod from your Steam libraries."),
        h("button", { class: "primary small", onclick: addFromSteam }, "Add from Steam…"),
        h(
          "p",
          { class: "fine" },
          "For a game not on Steam, ",
          h("button", { class: "link inline-link", onclick: addManually }, "add it manually"),
          " by picking its folder.",
        ),
      );

  $("#sidebar").replaceChildren(
    h("div", { class: "section-label" }, "Games"),
    list,
    S.games.length > 0
      ? h(
          "div",
          { class: "add-games" },
          h("button", { class: "ghost small", onclick: addFromSteam }, "+ Add from Steam…"),
          h("button", { class: "ghost small", onclick: addManually }, "+ Add manually…"),
        )
      : "",
    h(
      "div",
      { class: "sidebar-foot" },
      api.isMock && h("div", { class: "mock-note" }, "Preview with sample data"),
      S.startup &&
        h(
          "button",
          { class: "link", onclick: () => attempt(() => api.openFolder(S.startup.dataDir)) },
          icon("folder"),
          "Open data folder",
        ),
      h("button", { class: "link", onclick: reloadDefinitions }, "Reload game definitions"),
      h("button", { class: "link", onclick: linkSettings }, "Install buttons on mod sites…"),
      hiddenNotes().size > 0 &&
        h(
          "button",
          {
            class: "link",
            onclick: () => {
              setHiddenNotes(new Set());
              renderSidebar();
              toast("Hidden messages will show again", "ok");
            },
          },
          "Show hidden messages again",
        ),
    ),
  );
}

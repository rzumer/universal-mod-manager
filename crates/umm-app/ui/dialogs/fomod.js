import { api } from "../api.js";
import { refreshAll } from "../data.js";
import { $, h, plural } from "../dom.js";
import { banner, modal } from "../modals.js";
import { S, attempt } from "../state.js";

export const FOMOD_HINTS = {
  exactly_one: "Choose one",
  at_most_one: "Choose one, or none",
  at_least_one: "Choose one or more",
  all: "All of these are installed",
  any: "Choose any",
};
export const FOMOD_KINDS = { required: "Required", recommended: "Recommended", not_usable: "Not usable", could_be_usable: "May not work" };

/// A mod's FOMOD installer: its options step by step, then the game folder its files go
/// under. Resolves to true once saved.
export async function fomodDialog(m) {
  const game = S.gameId;
  let view = await attempt(() => api.fomodView(game, m.id, null, null));
  if (!view) return false;
  const key = (step, group) => `${step}\u0000${group}`;
  // The options in effect, by step and group: the saved ones, else the installer's defaults.
  const chosen = new Map(view.choices.map((c) => [key(c.step, c.group), c.plugins]));
  let target = view.target;
  let page = 0;
  let focus = null;
  const images = new Map();
  const choices = () => [...chosen].map(([k, plugins]) => ({ step: k.split("\u0000")[0], group: k.split("\u0000")[1], plugins }));

  return new Promise((resolve) => {
    const root = $("#modal-root");
    const close = (v) => {
      root.replaceChildren();
      document.removeEventListener("keydown", onKey);
      resolve(v);
    };
    const onKey = (e) => {
      if (e.key === "Escape") close(false);
    };
    document.addEventListener("keydown", onKey);
    const dlg = h("div", { class: "modal fomod", role: "dialog", "aria-modal": "true", "aria-label": `Install options for ${m.name}` });
    root.replaceChildren(h("div", { class: "scrim" }, dlg));

    const reload = async () => {
      const v = await attempt(() => api.fomodView(game, m.id, choices(), target));
      if (v) {
        view = v;
        for (const c of v.choices) chosen.set(key(c.step, c.group), c.plugins);
      }
      draw();
    };

    // The option described on the right: the one last pointed at, else the step's first chosen.
    const detail = (step) => {
      const all = step.groups.flatMap((g) => g.options);
      const o = all.find((x) => x.name === focus) || all.find((x) => x.selected) || all[0];
      if (!o) return h("div", { class: "fomod-detail" });
      // The same room for a picture whichever option is shown, so pointing at another
      // option never resizes the dialog and moves the options under the mouse.
      const pictures = all.some((x) => x.image);
      const img = pictures && h("div", { class: "fomod-image" });
      if (o.image && img) {
        const show = (url) => url && img.replaceChildren(h("img", { src: url, alt: "" }));
        if (images.has(o.image)) show(images.get(o.image));
        else
          api.modImage(game, m.id, o.image).then(
            (url) => {
              images.set(o.image, url);
              show(url);
            },
            () => images.set(o.image, null),
          );
      }
      return h("div", { class: "fomod-detail" }, img, h("h3", {}, o.name), h("p", { class: "fomod-desc" }, o.description || "No description."));
    };

    const group = (step, g) => {
      const radio = g.kind === "exactly_one" || g.kind === "at_most_one";
      const name = `fomod-${step.name}-${g.name}`;
      const set = (plugins) => {
        chosen.set(key(step.name, g.name), plugins);
        reload();
      };
      const row = (o) => {
        const locked = o.kind === "not_usable" || o.kind === "required" || g.kind === "all";
        return h(
          "label",
          {
            class: `fomod-option ${o.kind === "not_usable" ? "unusable" : ""}`,
            onmouseenter: () => {
              focus = o.name;
              const d = dlg.querySelector(".fomod-detail");
              d?.replaceWith(detail(step));
            },
          },
          h("input", {
            type: radio ? "radio" : "checkbox",
            name,
            checked: o.selected,
            disabled: locked,
            onchange: (ev) => {
              focus = o.name;
              const now = g.options.filter((x) => x.selected).map((x) => x.name);
              set(radio ? [o.name] : ev.target.checked ? [...now, o.name] : now.filter((n) => n !== o.name));
            },
          }),
          h("span", {}, o.name),
          FOMOD_KINDS[o.kind] && h("span", { class: `chip ${o.kind === "not_usable" ? "warn" : ""}` }, FOMOD_KINDS[o.kind]),
        );
      };
      return h(
        "fieldset",
        { class: "fomod-group" },
        h("legend", {}, g.name || step.name, h("span", { class: "muted small" }, ` · ${FOMOD_HINTS[g.kind]}`)),
        g.problem && h("p", { class: "small error-text" }, g.problem[0].toUpperCase() + g.problem.slice(1) + "."),
        g.options.map(row),
        g.kind === "at_most_one" &&
          h(
            "label",
            { class: "fomod-option" },
            h("input", { type: "radio", name, checked: !g.options.some((o) => o.selected), onchange: () => set([]) }),
            h("span", { class: "muted" }, "None"),
          ),
      );
    };

    const draw = () => {
      const steps = view.steps.filter((s) => s.visible);
      page = Math.min(page, steps.length);
      const last = page === steps.length;
      const step = steps[page];
      const stepOk = last || step.groups.every((g) => !g.problem);
      let body;
      if (!last) {
        body = h("div", { class: "fomod-step" }, h("div", { class: "fomod-groups" }, step.groups.map((g) => group(step, g))), detail(step));
      } else {
        const targetIn = h("input", {
          type: "text",
          list: "fomod-game-folders",
          value: target,
          placeholder: "The game folder itself",
          "aria-label": "Into this game folder",
          onchange: (ev) => {
            target = ev.target.value;
            reload();
          },
        });
        body = h(
          "div",
          {},
          h("label", { class: "field" }, h("span", {}, "Into this game folder"), targetIn, h("datalist", { id: "fomod-game-folders" }, view.gameFolders.map((d) => h("option", { value: d })))),
          h("p", { class: "muted small" }, `UMM suggests ${view.suggestedTarget ? `“${view.suggestedTarget}”` : "the game folder itself"}, from what the game folder already holds. `, view.suggestedTarget !== target && h("button", { class: "link inline-link", onclick: () => ((target = view.suggestedTarget), reload()) }, "Apply")),
          view.needsUnmet && h("p", { class: "banner warn" }, "The mod says it needs something this game does not have (another mod, or a newer version of the game). It may not work."),
          view.missing.length > 0 && h("p", { class: "small error-text" }, `The installer names files the archive does not have: ${view.missing.slice(0, 4).join(", ")}${view.missing.length > 4 ? "…" : ""}`),
          h("p", { class: "muted small" }, `${plural(view.files.length, "file")} to install:`),
          h(
            "div",
            { class: "place-files" },
            view.files.map((f) => h("div", { class: "place-file" }, h("span"), h("code", { class: "place-from" }, f.destination), h("span", { class: "place-to muted" }, target ? `in ${target}` : ""))),
          ),
        );
      }
      dlg.replaceChildren(
        ...[
        h("h2", {}, `${view.name}: install options`),
        h("p", { class: "muted small" }, last ? `Last step of ${steps.length + 1}: where it installs` : `Step ${page + 1} of ${steps.length + 1}: ${step.name}`),
        h("div", { class: "modal-body" }, body),
        h(
          "div",
          { class: "modal-actions" },
          h("button", { class: "ghost", onclick: () => close(false) }, "Cancel"),
          page > 0 && h("button", { onclick: () => ((page -= 1), (focus = null), draw()) }, "Back"),
          last
            ? h(
                "button",
                {
                  class: "primary",
                  disabled: !view.complete,
                  onclick: async () => {
                    const r = await attempt(() => api.fomodApply(game, m.id, choices(), target), `Saved the install options for ${m.name}`);
                    if (r === undefined) return;
                    close(true);
                    await refreshAll();
                  },
                },
                "Save",
              )
            : h("button", { class: "primary", disabled: !stepOk, onclick: () => ((page += 1), (focus = null), draw()) }, "Next"),
        ),
        ].filter(Boolean),
      );
    };
    draw();
  });
}

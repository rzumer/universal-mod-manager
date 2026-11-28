import { $, h } from "./dom.js";

// ---------- modals ----------

export function modal({ title, body, actions }) {
  return new Promise((resolve) => {
    const root = $("#modal-root");
    const close = (v) => {
      root.replaceChildren();
      document.removeEventListener("keydown", onKey);
      resolve(v);
    };
    const onKey = (e) => {
      if (e.key === "Escape") close(null);
    };
    document.addEventListener("keydown", onKey);
    const dlg = h(
      "div",
      { class: "modal", role: "dialog", "aria-modal": "true", "aria-label": title },
      h("h2", {}, title),
      h("div", { class: "modal-body" }, body),
      h(
        "div",
        { class: "modal-actions" },
        actions.map((a) =>
          h(
            "button",
            // An explicit `value: null` (Cancel, Close) must resolve to null, not the label.
            { class: a.class || "", onclick: () => close("value" in a ? a.value : a.label), autofocus: a.focus || false },
            a.label,
          ),
        ),
      ),
    );
    root.replaceChildren(h("div", { class: "scrim", onclick: (e) => e.target === e.currentTarget && close(null) }, dlg));
    dlg.querySelector("[autofocus]")?.focus();
  });
}

export const confirmModal = (title, text, okLabel, danger = false) =>
  modal({
    title,
    body: h("p", {}, text),
    actions: [
      { label: "Cancel", value: false, class: "ghost" },
      { label: okLabel, value: true, class: danger ? "danger" : "primary", focus: true },
    ],
  });

export function promptModal(title, label, placeholder = "") {
  const input = h("input", { type: "text", placeholder, "aria-label": label });
  const p = modal({
    title,
    body: h("label", { class: "field" }, h("span", {}, label), input),
    actions: [
      { label: "Cancel", value: null, class: "ghost" },
      { label: "OK", value: "__ok", class: "primary" },
    ],
  });
  setTimeout(() => {
    input.focus();
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") $("#modal-root .primary").click();
    });
  });
  return p.then((v) => (v === "__ok" ? input.value.trim() : null));
}

export function banner(kind, content, actions = []) {
  const b = h("div", { class: `banner ${kind}` }, h("div", {}, content), h("div", { class: "banner-actions" }, actions, h("button", { class: "ghost small", onclick: () => b.remove() }, "Dismiss")));
  $("#banners").append(b);
}

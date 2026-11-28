

// ---------- tiny DOM helpers ----------

export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v == null) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (k === "class") el.className = v;
    else if (k === "html") el.innerHTML = v;
    else if (v === true) el.setAttribute(k, "");
    else el.setAttribute(k, v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return el;
}

export const $ = (sel) => document.querySelector(sel);

export function fmtBytes(n) {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB"];
  let i = -1;
  do {
    n /= 1024;
    i++;
  } while (n >= 1024 && i < units.length - 1);
  return `${n.toFixed(n < 10 ? 1 : 0)} ${units[i]}`;
}

export const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

export function fmtDuration(s) {
  if (s < 60) return plural(s, "second");
  const m = Math.round(s / 60);
  if (m < 60) return plural(m, "minute");
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}

export function fmtTime(d = new Date()) {
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export const ICONS = {
  external: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M9 2h5v5h-1.5V4.6L7.6 9.5 6.5 8.4l4.9-4.9H9zM3 4h4v1.5H4.5v6h6V9H12v4H3z"/></svg>',
  grip: '<svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="5.5" cy="4" r="1.2"/><circle cx="10.5" cy="4" r="1.2"/><circle cx="5.5" cy="8" r="1.2"/><circle cx="10.5" cy="8" r="1.2"/><circle cx="5.5" cy="12" r="1.2"/><circle cx="10.5" cy="12" r="1.2"/></svg>',
  up: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 4l-4.5 5h9z"/></svg>',
  down: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 12l4.5-5h-9z"/></svg>',
  trash: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 2h4l.5 1H13v1.5H3V3h2.5zM4 5.5h8l-.6 8.5H4.6z"/></svg>',
  play: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 2.8v10.4L13 8z"/></svg>',
  folder: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M1.5 3.5h5l1.5 1.5h6.5v8.5h-13z"/></svg>',
  // Adjusting where a mod's files go.
  sliders: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 3.3h12v1.4H2zM2 7.3h12v1.4H2zM2 11.3h12v1.4H2z"/><rect x="9" y="1.5" width="2.6" height="5" rx=".8"/><rect x="3.8" y="5.5" width="2.6" height="5" rx=".8"/><rect x="8" y="9.5" width="2.6" height="5" rx=".8"/></svg>',};

export const icon = (name) => h("span", { class: "icon", html: ICONS[name] });

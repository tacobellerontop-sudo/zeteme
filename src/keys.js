// Vim-style key handling injected into every page, modeled on qutebrowser.
// Normal mode handles keys; editable fields put us in insert mode;
// ":", "o" and "/" open a one-line command bar at the bottom; "f" shows
// link hints.
(() => {
  if (window.top !== window || window.__zeteme) return;
  window.__zeteme = true;

  const STEP = 60;
  const send = (msg) => window.ipc.postMessage(msg);

  let pending = "";
  let lastSearch = "";
  let bar = null;
  let input = null;
  let barMode = null;
  let hideTimer = null;
  let hints = null;

  function ensureBar() {
    if (bar) return;
    const host = document.createElement("zeteme-bar");
    const root = host.attachShadow({ mode: "closed" });
    root.innerHTML = `
      <style>
        :host { all: initial; }
        div { position: fixed; left: 0; right: 0; bottom: 0; z-index: 2147483647;
              display: none; font: 13px/20px Consolas, monospace; color: #ddd;
              background: #1e1e1e; border-top: 1px solid #444; padding: 1px 6px; }
        input { all: unset; width: calc(100% - 2ch); color: #fff; }
        span { white-space: pre; }
      </style>
      <div><span></span><input spellcheck="false"></div>`;
    bar = root.querySelector("div");
    input = root.querySelector("input");
    input.addEventListener("keydown", onBarKey);
    input.addEventListener("blur", () => barMode && closeBar());
    document.documentElement.appendChild(host);
  }

  function openBar(prefix, text = "") {
    ensureBar();
    clearTimeout(hideTimer);
    barMode = prefix;
    bar.firstChild.textContent = prefix;
    input.style.display = "";
    input.value = text;
    bar.style.display = "block";
    input.focus();
  }

  function closeBar() {
    barMode = null;
    if (bar) bar.style.display = "none";
  }

  function flash(text) {
    ensureBar();
    closeBar();
    bar.firstChild.textContent = text;
    input.style.display = "none";
    bar.style.display = "block";
    hideTimer = setTimeout(() => { bar.style.display = "none"; input.style.display = ""; }, 1500);
  }

  function onBarKey(e) {
    e.stopPropagation();
    if (e.key === "Escape") { e.preventDefault(); closeBar(); return; }
    if (e.key !== "Enter") return;
    e.preventDefault();
    const mode = barMode;
    const value = input.value.trim();
    closeBar();
    if (mode === "/") search(value, false);
    else runCommand(value);
  }

  function search(text, backwards) {
    if (!text) return;
    lastSearch = text;
    // window.find is non-standard but supported by WebView2 (Chromium).
    if (!window.find(text, false, backwards, true)) flash(`Not found: ${text}`);
  }

  function runCommand(line) {
    const [cmd, ...rest] = line.split(/\s+/);
    const arg = rest.join(" ");
    switch (cmd) {
      case "o": case "open": if (arg) send(`open ${arg}`); break;
      case "t": case "tabopen": if (arg) send(`tabopen ${arg}`); break;
      case "close": send("close"); break;
      case "q": case "quit": send("quit"); break;
      case "back": history.back(); break;
      case "forward": history.forward(); break;
      case "r": case "reload": location.reload(); break;
      case "": break;
      default: flash(`Unknown command: ${cmd}`);
    }
  }

  function isEditable(el) {
    if (!el) return false;
    if (el.isContentEditable) return true;
    if (el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
    if (el.tagName !== "INPUT") return false;
    const nonText = ["button", "checkbox", "radio", "submit", "reset", "image", "range", "color", "file"];
    return !nonText.includes((el.type || "").toLowerCase());
  }

  // Link hints: label every visible clickable element, type its label to
  // follow it. "F" opens links in a new tab instead.
  const HINT_CHARS = "asdfghjkl";
  const CLICKABLE = "a[href], button, input, select, textarea, summary, label, " +
    "[onclick], [role=button], [role=link], [role=tab], [role=checkbox], " +
    "[role=menuitem], [contenteditable], [tabindex]:not([tabindex='-1'])";

  function hintLabels(count) {
    let len = 1;
    while (HINT_CHARS.length ** len < count) len++;
    return Array.from({ length: count }, (_, i) => {
      let label = "";
      for (let n = i, j = 0; j < len; j++, n = Math.floor(n / HINT_CHARS.length)) {
        label = HINT_CHARS[n % HINT_CHARS.length] + label;
      }
      return label;
    });
  }

  function showHints(newTab) {
    const targets = [...document.querySelectorAll(CLICKABLE)].filter((el) => {
      const r = el.getBoundingClientRect();
      if (r.width < 1 || r.height < 1) return false;
      if (r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) return false;
      const style = getComputedStyle(el);
      return style.visibility !== "hidden" && style.display !== "none";
    });
    if (!targets.length) { flash("No links"); return; }

    const host = document.createElement("zeteme-hints");
    const root = host.attachShadow({ mode: "closed" });
    root.innerHTML = `<style>
      :host { all: initial; }
      span { position: fixed; z-index: 2147483647; font: bold 11px/13px Consolas, monospace;
             color: #111; background: #fc3; border: 1px solid #a80; border-radius: 2px;
             padding: 0 2px; text-transform: uppercase; pointer-events: none; }
    </style>`;
    const labels = hintLabels(targets.length);
    const items = targets.map((el, i) => {
      const r = el.getBoundingClientRect();
      const tag = document.createElement("span");
      tag.textContent = labels[i];
      tag.style.left = `${Math.max(0, r.left)}px`;
      tag.style.top = `${Math.max(0, r.top)}px`;
      root.appendChild(tag);
      return { el, label: labels[i], tag };
    });
    document.documentElement.appendChild(host);
    hints = { host, items, typed: "", newTab };
  }

  function closeHints() {
    if (hints) hints.host.remove();
    hints = null;
  }

  function onHintKey(e) {
    e.preventDefault();
    e.stopImmediatePropagation();
    if (e.key === "Escape") { closeHints(); return; }
    if (e.key === "Backspace") hints.typed = hints.typed.slice(0, -1);
    else if (e.key.length === 1 && HINT_CHARS.includes(e.key.toLowerCase())) hints.typed += e.key.toLowerCase();
    else return;

    const matches = hints.items.filter((h) => h.label.startsWith(hints.typed));
    hints.items.forEach((h) => { h.tag.style.display = matches.includes(h) ? "" : "none"; });
    if (matches.length === 1 && matches[0].label === hints.typed) {
      const { el } = matches[0];
      const newTab = hints.newTab;
      closeHints();
      follow(el, newTab);
    } else if (!matches.length) {
      closeHints();
    }
  }

  function follow(el, newTab) {
    if (newTab && el.href) { send(`tabopen ${el.href}`); return; }
    if (isEditable(el)) { el.focus(); return; }
    el.click();
  }

  // Fullscreen video only fills the webview; ask for the window to follow.
  document.addEventListener("fullscreenchange", () => {
    send(`fullscreen ${document.fullscreenElement ? "on" : "off"}`);
  });

  const scroll = (x, y) => window.scrollBy({ left: x, top: y, behavior: "instant" });
  const half = () => window.innerHeight / 2;

  // Single keys, plus two-key sequences such as "gg" and "yy".
  const bindings = {
    j: () => scroll(0, STEP),
    k: () => scroll(0, -STEP),
    h: () => scroll(-STEP, 0),
    l: () => scroll(STEP, 0),
    d: () => send("close"),
    u: () => send("undo"),
    J: () => send("tab-next"),
    K: () => send("tab-prev"),
    gt: () => send("tab-next"),
    gT: () => send("tab-prev"),
    f: () => showHints(false),
    F: () => showHints(true),
    O: () => openBar(":", "tabopen "),
    "+": () => send("zoom-in"),
    "-": () => send("zoom-out"),
    "=": () => send("zoom-reset"),
    gg: () => window.scrollTo({ top: 0, behavior: "instant" }),
    G: () => window.scrollTo({ top: document.documentElement.scrollHeight, behavior: "instant" }),
    H: () => history.back(),
    L: () => history.forward(),
    r: () => location.reload(),
    o: () => openBar(":", "open "),
    go: () => openBar(":", `open ${location.href}`),
    ":": () => openBar(":"),
    "/": () => openBar("/"),
    n: () => search(lastSearch, false),
    N: () => search(lastSearch, true),
    yy: () => navigator.clipboard.writeText(location.href).then(() => flash("Copied URL")),
    gi: () => {
      const field = [...document.querySelectorAll("input, textarea, [contenteditable]")]
        .find((el) => isEditable(el) && el.offsetParent !== null);
      if (field) field.focus();
    },
  };
  const prefixes = new Set(Object.keys(bindings).filter((k) => k.length > 1).map((k) => k[0]));

  window.addEventListener("keydown", (e) => {
    if (barMode) return;
    if (hints) { onHintKey(e); return; }
    if (e.altKey || e.metaKey) return;
    if (e.ctrlKey) {
      // Ctrl-d / Ctrl-u scroll half a page, as in vim; other Ctrl keys go to the page.
      const ctrl = { d: () => scroll(0, half()), u: () => scroll(0, -half()) }[e.key];
      if (ctrl && !isEditable(document.activeElement)) {
        e.preventDefault();
        e.stopImmediatePropagation();
        ctrl();
      }
      return;
    }

    if (isEditable(document.activeElement)) {
      // Insert mode: only Escape is ours, to leave the field.
      if (e.key === "Escape") document.activeElement.blur();
      return;
    }

    if (e.key === "Escape") {
      pending = "";
      if (document.fullscreenElement) document.exitFullscreen();
      return;
    }
    if (e.key.length !== 1) return;

    const seq = pending + e.key;
    const action = bindings[seq];
    if (action) {
      pending = "";
    } else if (!pending && prefixes.has(e.key)) {
      pending = e.key;
    } else {
      pending = "";
      return;
    }
    e.preventDefault();
    e.stopImmediatePropagation();
    if (action) action();
  }, true);
})();

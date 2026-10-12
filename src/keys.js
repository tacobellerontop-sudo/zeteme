// Vim-style key handling injected into every page.
// Normal mode handles keys; editable fields put us in insert mode;
// ":", "o" and "/" open a one-line command bar at the bottom.
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

  const scroll = (x, y) => window.scrollBy({ left: x, top: y, behavior: "instant" });
  const half = () => window.innerHeight / 2;

  // Single keys, plus two-key sequences such as "gg" and "yy".
  const bindings = {
    j: () => scroll(0, STEP),
    k: () => scroll(0, -STEP),
    h: () => scroll(-STEP, 0),
    l: () => scroll(STEP, 0),
    d: () => scroll(0, half()),
    u: () => scroll(0, -half()),
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
    if (e.ctrlKey || e.altKey || e.metaKey) return;

    if (isEditable(document.activeElement)) {
      // Insert mode: only Escape is ours, to leave the field.
      if (e.key === "Escape") document.activeElement.blur();
      return;
    }

    if (e.key === "Escape") { pending = ""; return; }
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

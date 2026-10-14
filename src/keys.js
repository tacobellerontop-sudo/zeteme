// Vim-style key handling injected into every page, modeled on qutebrowser.
// Normal mode handles keys; editable fields put us in insert mode;
// ":", "o" and "/" open a one-line command bar at the bottom; "f" shows
// link hints. It also runs in frames, which pass bound keys up to the top
// page, so an embedded player with focus doesn't swallow them.
(() => {
  if (window.__zeteme) return;
  window.__zeteme = true;
  const isTop = window.top === window;

  const STEP = 60;
  const send = (msg) => window.ipc.postMessage(msg);

  let pending = "";
  let lastSearch = "";
  let bar = null;
  let input = null;
  let barMode = null;
  let hideTimer = null;
  let hints = null;
  // Keys whose keydown we handled; their keyup is kept from the page too.
  const swallowed = new Set();
  // Time of the last click or key the page was given. A field focused right
  // after one was chosen by the user and puts us in insert mode; one the page
  // focuses on its own (autofocus, YouTube's player) doesn't, as in qutebrowser.
  let lastIntent = 0;
  let lastPointer = 0;
  let insertEl = null;
  const intended = () => Date.now() - lastIntent < 500;

  // Built without innerHTML, which pages using Trusted Types (YouTube) block,
  // and styled with a constructed sheet, which a style-src CSP doesn't block.
  function styleRoot(root, css) {
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(css);
    root.adoptedStyleSheets = [sheet];
  }

  function ensureBar() {
    if (bar) return;
    const host = document.createElement("zeteme-bar");
    const root = host.attachShadow({ mode: "closed" });
    styleRoot(root, `
      :host { all: initial; }
      div { position: fixed; left: 0; right: 0; bottom: 0; z-index: 2147483647;
            display: none; font: 13px/20px Consolas, monospace; color: #ddd;
            background: #1e1e1e; border-top: 1px solid #444; padding: 1px 6px; }
      input { all: unset; width: calc(100% - 2ch); color: #fff; }
      span { white-space: pre; }`);
    bar = document.createElement("div");
    input = document.createElement("input");
    input.spellcheck = false;
    bar.append(document.createElement("span"), input);
    root.appendChild(bar);
    input.addEventListener("blur", () => {
      if (!barMode) return;
      // Only a click elsewhere closes the bar; pages that move focus around on
      // their own don't get to.
      if (Date.now() - lastPointer < 500) closeBar();
      else setTimeout(() => barMode && input.focus());
    });
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

  // Every key typed into the bar is kept from the page, whose shortcuts would
  // otherwise fire: it sees our closed shadow root, not a text field. Typing
  // still works, since that is the keydown's default action.
  function onBarKey(e) {
    e.stopImmediatePropagation();
    swallowed.add(e.code);
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
      case "set": case "settings": send("settings"); break;
      case "bw": case "bitwarden":
        send({ lock: "bitwarden lock", fill: "bitwarden" }[arg] || "bitwarden vault"); break;
      case "ext": case "extension": send(`ext ${arg}`); break;
      case "back": history.back(); break;
      case "forward": history.forward(); break;
      case "r": case "reload": location.reload(); break;
      case "": break;
      default: flash(`Unknown command: ${cmd}`);
    }
  }

  // The focused element, looking inside open shadow roots.
  function activeField() {
    let el = document.activeElement;
    while (el && el.shadowRoot && el.shadowRoot.activeElement) el = el.shadowRoot.activeElement;
    return el;
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
    styleRoot(root, `
      :host { all: initial; }
      span { position: fixed; z-index: 2147483647; font: bold 11px/13px Consolas, monospace;
             color: #111; background: #fc3; border: 1px solid #a80; border-radius: 2px;
             padding: 0 2px; text-transform: uppercase; pointer-events: none; }`);
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
    consume(e);
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
    if (isEditable(el)) { lastIntent = Date.now(); el.focus(); return; }
    el.click();
  }

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
    "C-d": () => scroll(0, half()),
    "C-u": () => scroll(0, -half()),
    // Ctrl-Shift-L, Bitwarden's own fill shortcut.
    "C-L": () => send("bitwarden"),
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
      if (field) { lastIntent = Date.now(); field.focus(); }
    },
    i: () => {
      const field = activeField();
      if (isEditable(field)) insertEl = field;
    },
  };
  const prefixes = new Set(Object.keys(bindings).filter((k) => k.length === 2).map((k) => k[0]));

  function consume(e) {
    e.preventDefault();
    e.stopImmediatePropagation();
    swallowed.add(e.code);
  }

  // Frames hand their keys to the top page, which runs the binding.
  function run(seq) {
    if (isTop) bindings[seq]();
    else window.top.postMessage({ zeteme: seq }, "*");
  }

  function onKey(e) {
    if (barMode) { onBarKey(e); return; }
    if (hints) { onHintKey(e); return; }
    const field = activeField();
    if (e.ctrlKey && e.shiftKey && e.key.toUpperCase() === "L") { consume(e); run("C-L"); return; }
    if (insertEl && insertEl === field && isEditable(field)) {
      // Insert mode: only Escape is ours, to leave the field.
      if (e.key === "Escape") { consume(e); insertEl = null; field.blur(); }
      else lastIntent = Date.now();
      return;
    }
    if (e.altKey || e.metaKey) { lastIntent = Date.now(); return; }
    if (e.ctrlKey) {
      // Ctrl-d / Ctrl-u scroll half a page, as in vim; other Ctrl keys go to the page.
      const seq = `C-${e.key}`;
      if (bindings[seq]) { consume(e); run(seq); } else lastIntent = Date.now();
      return;
    }
    if (e.key === "Escape") {
      pending = "";
      if (document.fullscreenElement) document.exitFullscreen();
      return;
    }
    if (e.key.length !== 1) { lastIntent = Date.now(); return; }

    const seq = pending + e.key;
    if (bindings[seq]) {
      pending = "";
      consume(e);
      run(seq);
    } else if (!pending && prefixes.has(e.key)) {
      pending = e.key;
      consume(e);
    } else {
      pending = "";
      // A field the page focused on its own isn't typed into until you click
      // it or press i.
      if (isEditable(field)) consume(e);
      else lastIntent = Date.now();
    }
  }

  // Registered at document start, before any page script, so these run first.
  window.addEventListener("keydown", onKey, true);
  window.addEventListener("keypress", (e) => {
    if (barMode || hints) e.stopImmediatePropagation();
  }, true);
  window.addEventListener("keyup", (e) => {
    if (swallowed.delete(e.code) || barMode || hints) e.stopImmediatePropagation();
  }, true);
  window.addEventListener("mousedown", () => { lastPointer = lastIntent = Date.now(); }, true);
  window.addEventListener("focusin", (e) => {
    const el = e.composedPath()[0];
    insertEl = isEditable(el) && intended() ? el : null;
  }, true);

  if (!isTop) return;

  window.addEventListener("message", (e) => {
    const seq = e.data && e.data.zeteme;
    // Only right after a real key press, so a frame can't run bindings by itself.
    if (typeof seq !== "string" || !Object.hasOwn(bindings, seq)) return;
    if (!navigator.userActivation || !navigator.userActivation.isActive) return;
    // Take focus back from the frame so hint keys and the bar reach us.
    if (document.activeElement && document.activeElement.tagName === "IFRAME") document.activeElement.blur();
    window.focus();
    bindings[seq]();
  });

  // Called by zeteme with a login from Bitwarden: fills the form's username
  // and password fields, as the page would see them typed.
  const isUserField = (el) => ["text", "email", "tel", ""].includes((el.getAttribute("type") || "").toLowerCase());
  function setValue(el, value) {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }
  window.zetemeFill = (user, pass) => {
    const fields = [...document.querySelectorAll("input")].filter((el) =>
      el.getClientRects().length && !el.disabled && !el.readOnly && el.type !== "hidden");
    const focused = activeField();
    const inForm = focused && focused.form;
    const passEl = (focused && focused.type === "password" && focused) ||
      fields.find((el) => el.type === "password" && inForm && el.form === inForm) ||
      fields.find((el) => el.type === "password");
    let userEl = null;
    if (passEl) {
      userEl = fields.slice(0, fields.indexOf(passEl)).filter(isUserField).pop();
    } else if (focused && fields.includes(focused) && isUserField(focused)) {
      userEl = focused;
    } else {
      // A sign-in that asks for the username first.
      userEl = fields.find((el) => isUserField(el) && /user|mail|login|account|name/i.test(
        `${el.name} ${el.id} ${el.autocomplete}`)) || fields.find(isUserField);
    }
    if (!userEl && !passEl) { flash("No login form here"); return; }
    if (userEl && user) setValue(userEl, user);
    if (passEl) setValue(passEl, pass);
    (passEl || userEl).focus();
  };
  window.zetemeFlash = flash;

  // Fullscreen video only fills the webview; ask for the window to follow.
  document.addEventListener("fullscreenchange", () => {
    send(`fullscreen ${document.fullscreenElement ? "on" : "off"}`);
  });
})();

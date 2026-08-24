// Agency preview bridge. Injected by the preview proxy into every HTML page
// the Run tab preview loads (see preview/proxy.rs), same-origin with the page,
// so it can read the console, walk the DOM and synthesize input. It talks only
// to the loopback server that injected it, under /__agency__/. Nothing here is
// part of the app under development, and nothing survives into a real browser:
// pages opened outside Agency load the dev server directly, unproxied.
(function () {
  "use strict";
  if (window.__agencyPreviewBridge) return;
  window.__agencyPreviewBridge = true;

  var B = "/__agency__";
  var instance = Math.random().toString(36).slice(2) + Date.now().toString(36);
  var retired = false;

  // ── console capture ───────────────────────────────────────────────────────
  // Entries batch for 250ms so a chatty render loop costs a few requests per
  // second at worst, and flush via sendBeacon on pagehide so the last lines
  // before a navigation are not lost.

  var pending = [];
  var flushTimer = null;

  function push(level, text) {
    if (retired) return;
    pending.push({ level: level, text: String(text).slice(0, 4000) });
    if (pending.length > 200) pending.splice(0, pending.length - 200);
    if (!flushTimer) flushTimer = setTimeout(flush, 250);
  }

  function flush() {
    flushTimer = null;
    if (!pending.length) return;
    var body = JSON.stringify({ instance: instance, entries: pending.splice(0) });
    try {
      fetch(B + "/log", { method: "POST", body: body, keepalive: true }).catch(function () {});
    } catch (e) { /* fetch itself unavailable: nothing to do */ }
  }

  function fmt(args) {
    var parts = [];
    for (var i = 0; i < args.length; i++) parts.push(one(args[i]));
    return parts.join(" ");
  }

  function one(v) {
    if (typeof v === "string") return v;
    if (v instanceof Error) return v.stack || String(v);
    try {
      var seen = [];
      return JSON.stringify(v, function (_k, val) {
        if (typeof val === "object" && val !== null) {
          if (seen.indexOf(val) !== -1) return "[circular]";
          seen.push(val);
        }
        if (typeof val === "function") return "[function]";
        return val;
      });
    } catch (e) {
      return String(v);
    }
  }

  ["log", "info", "warn", "error", "debug"].forEach(function (level) {
    var orig = console[level] && console[level].bind(console);
    console[level] = function () {
      push(level, fmt(arguments));
      if (orig) orig.apply(null, arguments);
    };
  });

  window.addEventListener("error", function (e) {
    var where = e.filename ? " (" + e.filename + ":" + e.lineno + ")" : "";
    push("error", "Uncaught " + (e.message || "error") + where);
  });
  window.addEventListener("unhandledrejection", function (e) {
    push("error", "Unhandled promise rejection: " + one(e.reason));
  });
  window.addEventListener("pagehide", flush);

  // ── presence ──────────────────────────────────────────────────────────────

  function hello() {
    if (retired) return;
    try {
      fetch(B + "/hello", {
        method: "POST",
        body: JSON.stringify({ instance: instance, url: location.href, title: document.title }),
        keepalive: true,
      }).catch(function () {});
    } catch (e) { /* ignore */ }
  }

  ["DOMContentLoaded", "hashchange", "popstate"].forEach(function (ev) {
    window.addEventListener(ev, function () { setTimeout(hello, 50); });
  });
  ["pushState", "replaceState"].forEach(function (name) {
    var orig = history[name];
    if (!orig) return;
    history[name] = function () {
      var r = orig.apply(this, arguments);
      setTimeout(hello, 50);
      return r;
    };
  });

  // ── command loop ──────────────────────────────────────────────────────────
  // Long poll: the server parks the request until a tool call arrives (or ~20s
  // pass), so commands land within a network round trip of being issued.

  function poll() {
    if (retired) return;
    var ctl = typeof AbortController !== "undefined" ? new AbortController() : null;
    var timer = ctl && setTimeout(function () { ctl.abort(); }, 30000);
    fetch(B + "/poll", {
      method: "POST",
      body: JSON.stringify({ instance: instance }),
      signal: ctl && ctl.signal,
    })
      .then(function (r) { return r.ok ? r.json() : {}; })
      .then(function (cmd) {
        if (timer) clearTimeout(timer);
        if (cmd && cmd.retire) { retired = true; return; }
        if (cmd && cmd.id !== undefined) {
          var result;
          try { result = run(cmd); } catch (e) { result = { error: "bridge error: " + (e && e.message) }; }
          var body = JSON.stringify({ instance: instance, id: cmd.id, result: result });
          // keepalive survives a same-tick navigation (a clicked link), but
          // caps the body size, so big results go without it.
          return fetch(B + "/result", { method: "POST", body: body, keepalive: body.length < 50000 })
            .catch(function () {})
            .then(poll);
        }
        poll();
      })
      .catch(function () {
        if (timer) clearTimeout(timer);
        if (!retired) setTimeout(poll, 1000);
      });
  }

  function run(cmd) {
    var c = cmd.cmd || {};
    switch (c.kind) {
      case "navigate": return navigate(c);
      case "click": return click(c);
      case "type": return type(c);
      case "snapshot": return snapshot();
      default: return { error: "unknown command: " + c.kind };
    }
  }

  function pageState() {
    return { url: location.href, title: document.title };
  }

  // ── navigate ──────────────────────────────────────────────────────────────

  function navigate(c) {
    var to;
    try { to = new URL(c.path, location.href); } catch (e) { return { error: "not a valid path: " + c.path }; }
    if (to.origin !== location.origin) {
      return { error: "the preview stays on the app's own origin; " + to.origin + " is outside it" };
    }
    // Answer first: assigning location unloads this page, bridge included.
    setTimeout(function () { location.assign(to.href); }, 30);
    return { ok: true, navigating: to.href };
  }

  // ── click ─────────────────────────────────────────────────────────────────

  function find(selector) {
    var el;
    try { el = document.querySelector(selector); } catch (e) { return { error: "not a valid CSS selector: " + selector }; }
    if (!el) return { error: "nothing matches " + selector + " on " + location.pathname + "; take a preview_snapshot for current selectors" };
    return { el: el };
  }

  function click(c) {
    var f = find(c.selector);
    if (!f.el) return f;
    var el = f.el;
    try { el.scrollIntoView({ block: "center", inline: "center" }); } catch (e) { /* ignore */ }
    var r = el.getBoundingClientRect();
    var x = r.left + r.width / 2, y = r.top + r.height / 2;
    var opts = { bubbles: true, cancelable: true, view: window, clientX: x, clientY: y, button: 0, detail: 1 };
    ["pointerdown", "mousedown", "pointerup", "mouseup", "click"].forEach(function (t) {
      var ev = t.indexOf("pointer") === 0 && typeof PointerEvent !== "undefined"
        ? new PointerEvent(t, opts)
        : new MouseEvent(t, opts);
      el.dispatchEvent(ev);
    });
    if (typeof el.focus === "function") { try { el.focus(); } catch (e) { /* ignore */ } }
    var s = pageState();
    s.ok = true;
    s.clicked = describe(el);
    return s;
  }

  // ── type ──────────────────────────────────────────────────────────────────

  function type(c) {
    var f = find(c.selector);
    if (!f.el) return f;
    var el = f.el;
    var clear = c.clear !== false;
    try { el.focus(); } catch (e) { /* ignore */ }
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
      var proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      var desc = Object.getOwnPropertyDescriptor(proto, "value");
      var next = clear ? c.text : el.value + c.text;
      // The native setter, so frameworks that patch `value` (React's controlled
      // inputs) see the change when the input event fires.
      if (desc && desc.set) desc.set.call(el, next); else el.value = next;
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    } else if (el.isContentEditable) {
      if (clear) el.textContent = c.text; else el.textContent += c.text;
      el.dispatchEvent(new Event("input", { bubbles: true }));
    } else {
      return { error: describe(el) + " is not an input, textarea or contenteditable element" };
    }
    if (c.enter) {
      var key = { key: "Enter", code: "Enter", keyCode: 13, which: 13, bubbles: true, cancelable: true };
      var down = new KeyboardEvent("keydown", key);
      el.dispatchEvent(down);
      el.dispatchEvent(new KeyboardEvent("keyup", key));
      var form = el.form;
      if (!down.defaultPrevented && form && typeof form.requestSubmit === "function") {
        try { form.requestSubmit(); } catch (e) { /* invalid form: browser blocked it */ }
      }
    }
    var s = pageState();
    s.ok = true;
    s.typed_into = describe(el);
    return s;
  }

  // ── snapshot ──────────────────────────────────────────────────────────────
  // A structured-text outline of the visible page: what an agent needs to
  // decide where to click, with a working selector per interactive element.
  // Text, not pixels, because WebKit taints any canvas a foreignObject SVG is
  // drawn into, so a same-page pixel screenshot cannot be exported at all —
  // pixels come from the app's native snapshot path instead (preview_shot.rs).

  function visible(el) {
    if (!el.getClientRects().length) return false;
    var st = getComputedStyle(el);
    return st.visibility !== "hidden" && st.display !== "none";
  }

  function describe(el) {
    var d = el.tagName.toLowerCase();
    if (el.id) d += "#" + el.id;
    return d;
  }

  function selectorFor(el) {
    if (el.id) return "#" + cssEscape(el.id);
    var testid = el.getAttribute && el.getAttribute("data-testid");
    if (testid) return "[data-testid=\"" + testid.replace(/"/g, '\\"') + "\"]";
    var parts = [];
    var cur = el;
    while (cur && cur.nodeType === 1 && parts.length < 5) {
      var tag = cur.tagName.toLowerCase();
      if (cur.id) { parts.unshift("#" + cssEscape(cur.id)); break; }
      var parent = cur.parentElement;
      if (parent) {
        var same = Array.prototype.filter.call(parent.children, function (ch) {
          return ch.tagName === cur.tagName;
        });
        if (same.length > 1) tag += ":nth-of-type(" + (same.indexOf(cur) + 1) + ")";
      }
      parts.unshift(tag);
      cur = parent;
    }
    return parts.join(" > ");
  }

  function cssEscape(s) {
    return typeof CSS !== "undefined" && CSS.escape ? CSS.escape(s) : s.replace(/([^\w-])/g, "\\$1");
  }

  function textOf(el, cap) {
    var t = (el.innerText || el.textContent || "").replace(/\s+/g, " ").trim();
    return t.length > cap ? t.slice(0, cap) + "…" : t;
  }

  function snapshot() {
    var lines = [];
    var seen = 0;
    var walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_ELEMENT);
    var interactive = { A: 1, BUTTON: 1, INPUT: 1, TEXTAREA: 1, SELECT: 1, SUMMARY: 1 };
    while (walker.nextNode() && lines.length < 300 && seen < 5000) {
      seen++;
      var el = walker.currentNode;
      var tag = el.tagName;
      if (tag === "SCRIPT" || tag === "STYLE" || tag === "NOSCRIPT") continue;
      if (!visible(el)) continue;
      if (/^H[1-6]$/.test(tag)) {
        lines.push("# " + tag.toLowerCase() + " \"" + textOf(el, 120) + "\"");
      } else if (tag === "A" && el.getAttribute("href")) {
        lines.push("link \"" + textOf(el, 80) + "\" href=" + el.getAttribute("href") + "  [" + selectorFor(el) + "]");
      } else if (tag === "BUTTON" || (el.getAttribute && el.getAttribute("role") === "button")) {
        lines.push("button \"" + textOf(el, 80) + "\"" + (el.disabled ? " (disabled)" : "") + "  [" + selectorFor(el) + "]");
      } else if (tag === "INPUT") {
        var kind = el.type || "text";
        if (kind === "hidden") continue;
        var val = kind === "password" ? "••" : String(el.value).slice(0, 60);
        var extra = kind === "checkbox" || kind === "radio" ? (el.checked ? " checked" : " unchecked") : " value=\"" + val + "\"";
        lines.push("input(" + kind + ")" + (el.placeholder ? " placeholder=\"" + el.placeholder + "\"" : "") + extra + "  [" + selectorFor(el) + "]");
      } else if (tag === "TEXTAREA") {
        lines.push("textarea value=\"" + String(el.value).slice(0, 80) + "\"  [" + selectorFor(el) + "]");
      } else if (tag === "SELECT") {
        var opt = el.selectedOptions && el.selectedOptions[0];
        lines.push("select \"" + (opt ? textOf(opt, 60) : "") + "\"  [" + selectorFor(el) + "]");
      } else if ((tag === "P" || tag === "LI" || tag === "LABEL" || tag === "TD") && el.children.length === 0) {
        var t = textOf(el, 140);
        if (t && !(el.parentElement && interactive[el.parentElement.tagName])) lines.push("text \"" + t + "\"");
      }
    }
    var out = lines.join("\n");
    if (out.length > 14000) out = out.slice(0, 14000) + "\n… truncated";
    if (lines.length >= 300) out += "\n… more elements not listed";
    var s = pageState();
    s.outline = out || "(no visible elements yet)";
    return s;
  }

  // ── start ─────────────────────────────────────────────────────────────────

  hello();
  poll();
})();

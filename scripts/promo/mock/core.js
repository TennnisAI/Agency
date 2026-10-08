// Tauri IPC emulation for capturing the Agency UI in a plain browser.
// Loaded via addInitScript before any app code. Data lives in mock-data.js,
// which defines window.__MOCK_HANDLERS__ (cmd -> fn(args)).
(function () {
  const callbacks = new Map();
  let nextId = 1;
  const listeners = new Map(); // event -> [{id, handler}]
  const unhandled = new Set();
  window.__mockLog = [];
  window.__mockUnhandled = unhandled;

  function transformCallback(cb, once) {
    const id = nextId++;
    callbacks.set(id, (data) => {
      if (once) callbacks.delete(id);
      return cb && cb(data);
    });
    return id;
  }
  function unregisterCallback(id) { callbacks.delete(id); }
  function runCallback(id, data) {
    const cb = callbacks.get(id);
    if (cb) cb(data);
  }

  // Channel helper: a Channel arg serialises as an object with `.id`.
  const channelIdx = new Map();
  function sendChannel(channel, message) {
    if (!channel) return;
    const id = typeof channel === "object" ? channel.id : Number(String(channel).replace("__CHANNEL__:", ""));
    const index = channelIdx.get(id) ?? 0;
    channelIdx.set(id, index + 1);
    runCallback(id, { index, message });
  }

  function emit(event, payload) {
    for (const l of listeners.get(event) || []) {
      runCallback(l.handler, { event, id: l.id, payload });
    }
  }

  async function invoke(cmd, args = {}, _opts) {
    window.__mockLog.push(cmd);
    if (cmd === "plugin:event|listen") {
      const arr = listeners.get(args.event) || [];
      const id = nextId++;
      arr.push({ id, handler: args.handler });
      listeners.set(args.event, arr);
      return id;
    }
    if (cmd === "plugin:event|unlisten") {
      const arr = listeners.get(args.event) || [];
      listeners.set(args.event, arr.filter((l) => l.id !== args.eventId));
      return null;
    }
    if (cmd === "plugin:event|emit" || cmd === "plugin:event|emit_to") {
      emit(args.event, args.payload);
      return null;
    }
    if (cmd.startsWith("plugin:window|") || cmd.startsWith("plugin:webview|") || cmd.startsWith("plugin:app|")) {
      if (cmd === "plugin:app|version") return "0.2.3";
      if (cmd.endsWith("is_maximized") || cmd.endsWith("is_fullscreen")) return false;
      if (cmd.endsWith("is_focused")) return true;
      if (cmd.endsWith("scale_factor")) return 2;
      return null;
    }
    if (cmd.startsWith("plugin:opener|") || cmd.startsWith("plugin:dialog|")) return null;
    if (cmd.startsWith("plugin:path|")) return "/Users/demo/Library/Logs/build.agency.app";
    const h = window.__MOCK_HANDLERS__ && window.__MOCK_HANDLERS__[cmd];
    if (h) {
      try {
        const r = await h(args);
        return r === undefined ? null : r;
      } catch (e) {
        console.warn("[mock] handler threw", cmd, e);
        throw e;
      }
    }
    if (!unhandled.has(cmd)) {
      unhandled.add(cmd);
      console.warn("[mock] unhandled", cmd, JSON.stringify(args).slice(0, 200));
    }
    return window.__MOCK_FALLBACK__ ? window.__MOCK_FALLBACK__(cmd, args) : null;
  }

  window.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
    invoke,
    transformCallback,
    unregisterCallback,
    runCallback,
    callbacks,
    convertFileSrc: (p, protocol = "asset") => `${protocol}://localhost/${encodeURIComponent(p)}`,
    plugins: { path: { sep: "/", delimiter: ":" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event, id) {
      const arr = listeners.get(event) || [];
      listeners.set(event, arr.filter((l) => l.id !== id));
    },
  };
  window.__mock = { emit, sendChannel, runCallback };
})();

// Simulated agent CLI sessions. Each session keeps a transcript (lines of
// ANSI text) plus a footer (spinner + input box) and is driven by a timed
// script of steps. Output is streamed to attached xterm channels the way an
// Ink-style TUI redraws: cursor up over the footer, clear, append, redraw.
(function () {
  const E = "\x1b[";
  const rgb = (r, g, b) => `${E}38;2;${r};${g};${b}m`;
  const bgc = (r, g, b) => `${E}48;2;${r};${g};${b}m`;
  const R = `${E}0m`, B = `${E}1m`, D = `${E}2m`, I = `${E}3m`;
  const C = {
    claude: rgb(215, 119, 87), white: rgb(230, 230, 230), dim: rgb(140, 140, 150), green: rgb(78, 186, 101),
    red: rgb(255, 107, 128), gray: rgb(153, 153, 153), blue: rgb(110, 160, 255), cyan: rgb(80, 200, 220),
    yellow: rgb(229, 192, 123), purple: rgb(190, 140, 255), addBg: bgc(30, 62, 40), delBg: bgc(80, 34, 44),
    teal: rgb(100, 210, 190), pink: rgb(245, 150, 200), orange: rgb(250, 170, 90),
  };
  const strip = (s) => s.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "");
  const vis = (s) => [...strip(s)].length;
  const pad = (s, w) => s + " ".repeat(Math.max(0, w - vis(s)));

  const sessions = {};
  window.__TERM__ = { sessions, C, strip };

  // ---------- Styles: how each CLI draws a step ----------
  const SPIN = ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"];
  const BRAILLE = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

  function boxLines(w, inner, color) {
    const top = `${color}╭${"─".repeat(w - 2)}╮${R}`;
    const bot = `${color}╰${"─".repeat(w - 2)}╯${R}`;
    return [top, ...inner.map((l) => `${color}│${R} ${pad(l, w - 4)} ${color}│${R}`), bot];
  }

  const styles = {
    claude: {
      header(s) {
        const w = Math.min(58, s.cols - 2);
        return [
          ...boxLines(w, [`${C.claude}✻${R} Welcome to ${B}Claude Code${R}!`, "", `  ${I}${C.dim}/help for help, /status for your current setup${R}`, "", `  ${C.dim}cwd: ${s.cwd}${R}`], C.claude),
          "",
        ];
      },
      prompt: (t) => ["", `${C.dim}>${R} ${C.dim}${t}${R}`, ""],
      say: (t) => [`${C.white}●${R} ${t}`, ""],
      read: (f, n) => [`${C.green}●${R} ${B}Read${R}(${f})`, `  ${C.dim}⎿${R}  Read ${B}${n}${R} lines`, ""],
      search: (q, n) => [`${C.green}●${R} ${B}Search${R}(pattern: "${q}")`, `  ${C.dim}⎿${R}  Found ${B}${n}${R} files`, ""],
      edit(f, lines, add, del) {
        const out = [`${C.green}●${R} ${B}Update${R}(${f})`, `  ${C.dim}⎿${R}  Updated ${B}${f}${R} with ${B}${add}${R} additions${del ? ` and ${B}${del}${R} removals` : ""}`];
        let n = 40;
        for (const l of lines) {
          const k = l[0], body = l.slice(2).replace(/\t/g, "    ");
          const num = String(n++).padStart(6);
          if (k === "+") out.push(`     ${C.addBg}${num} +  ${body}${" ".repeat(Math.max(0, 66 - body.length))}${R}`);
          else if (k === "-") out.push(`     ${C.delBg}${num} -  ${body}${" ".repeat(Math.max(0, 66 - body.length))}${R}`);
          else out.push(`     ${C.dim}${num}${R}    ${body}`);
        }
        out.push("");
        return out;
      },
      bash(cmd, outLines) {
        return [`${C.green}●${R} ${B}Bash${R}(${cmd})`, ...outLines.map((l, i) => `  ${C.dim}${i === 0 ? "⎿" : " "}${R}  ${l}`), ""];
      },
      todo(items) {
        return [`${C.green}●${R} ${B}Update Todos${R}`, ...items.map((it, i) => `  ${C.dim}${i === 0 ? "⎿" : " "}${R}  ${it[0] ? `${C.green}☒${R} ${D}${it[1]}${R}` : `☐ ${it[1]}`}`), ""];
      },
      footer(s) {
        const w = s.cols - 1;
        const out = [];
        if (s.busy) {
          const g = SPIN[s.tick % SPIN.length];
          const secs = Math.max(1, Math.floor((s.now - s.busySince) / 1000));
          const tok = (s.tokens / 1000).toFixed(1);
          out.push(`${C.claude}${g} ${s.busy}…${R} ${C.dim}(${secs}s · ↓ ${tok}k tokens · esc to interrupt)${R}`, "");
        }
        if (s.blocked) {
          out.push(...boxLines(Math.min(70, w), [`${B}Bash command${R}`, "", `  ${s.blocked}`, "", "Do you want to proceed?", `${C.claude}❯${R} 1. Yes`, "  2. Yes, and don't ask again this session", "  3. No, and tell Claude what to do differently"], C.claude));
          return out;
        }
        out.push(...boxLines(w, [`${C.dim}>${R} ${s.draft || ""}${s.cursorOn ? "\x1b[7m \x1b[27m" : ""}`], C.gray));
        out.push(`  ${C.purple}⏵⏵ accept edits on${R} ${C.dim}(shift+tab to cycle)${R}`);
        return out;
      },
    },
  };

  // Codex-like
  styles.codex = {
    header: (s) => [`${B}>_ ${R}${B}OpenAI Codex${R}`, "", `${C.dim}directory:${R} ${s.cwd}`, `${C.dim}approval:${R}  on-request`, ""],
    prompt: (t) => [`${C.cyan}user${R}`, t, ""],
    say: (t) => [`${C.purple}codex${R}`, t, ""],
    read: (f) => [`${B}•${R} ${B}Explored${R}`, `  ${C.dim}└${R} Read ${f}`, ""],
    search: (q, n) => [`${B}•${R} ${B}Explored${R}`, `  ${C.dim}└${R} Search ${q} ${C.dim}(${n} matches)${R}`, ""],
    edit(f, lines, add, del) {
      const out = [`${C.green}•${R} ${B}Edited${R} ${f} ${C.dim}(${C.green}+${add}${C.dim} ${C.red}-${del}${C.dim})${R}`];
      let n = 60;
      for (const l of lines) {
        const k = l[0], body = l.slice(2).replace(/\t/g, "    ");
        const num = String(n++).padStart(4);
        if (k === "+") out.push(`    ${C.dim}${num}${R} ${C.green}+${body}${R}`);
        else if (k === "-") out.push(`    ${C.dim}${num}${R} ${C.red}-${body}${R}`);
        else out.push(`    ${C.dim}${num}  ${body}${R}`);
      }
      out.push("");
      return out;
    },
    bash: (cmd, o) => [`${C.green}•${R} ${B}Ran${R} ${cmd}`, ...o.map((l, i) => `  ${C.dim}${i === 0 ? "└" : " "}${R} ${C.dim}${l}${R}`), ""],
    todo: (items) => [`${B}•${R} ${B}Updated Plan${R}`, ...items.map((it, i) => `  ${C.dim}${i === 0 ? "└" : " "}${R} ${it[0] ? `${C.green}✔${R} ${D}${it[1]}${R}` : `□ ${it[1]}`}`), ""],
    footer(s) {
      const out = [];
      if (s.busy) {
        const secs = Math.max(1, Math.floor((s.now - s.busySince) / 1000));
        out.push(`${C.dim}${BRAILLE[s.tick % BRAILLE.length]}${R} ${B}${s.busy}${R} ${C.dim}(${secs}s • esc to interrupt)${R}`, "");
      }
      out.push(`${C.cyan}▌${R} ${s.draft || `${C.dim}Ask Codex to do anything${R}`}`);
      out.push(`  ${C.dim}⏎ send   ⇧⏎ newline   ⌃T transcript   ⌃C quit${R}`);
      return out;
    },
  };

  // Gemini-like
  const GEM = ["███  ██████  ██████ ███    ███ ██ ███    ██ ██", " ███ ██      ██      ████  ████ ██ ████   ██ ██", "  ███ ██   ███ █████   ██ ████ ██ ██ ██ ██  ██ ██", " ███  ██    ██ ██      ██  ██  ██ ██ ██  ██ ██ ██", "███    ██████  ██████  ██      ██ ██ ██   ████ ██"];
  const grad = (line) => [...line].map((ch, i) => { const t = i / line.length; return `${rgb(Math.round(70 + 120 * t), Math.round(140 - 20 * t), Math.round(250 - 30 * t))}${ch}`; }).join("") + R;
  styles.gemini = {
    header: (s) => ["", ...GEM.map(grad), "", "Tips for getting started:", `1. Ask questions, edit files, or run commands.`, `2. Be specific for the best results.`, ""],
    prompt: (t) => [`${C.dim}>${R} ${t}`, ""],
    say: (t) => [`${C.purple}✦${R} ${t}`, ""],
    read: (f) => [...boxLines(64, [`${C.green}✔${R}  ${B}ReadFile${R} ${f}`], C.dim), ""],
    search: (q) => [...boxLines(64, [`${C.green}✔${R}  ${B}SearchText${R} '${q}'`], C.dim), ""],
    edit(f, lines, add, del) {
      const inner = [`${C.green}✔${R}  ${B}Edit${R} ${f}`, ""];
      let n = 20;
      for (const l of lines.slice(0, 8)) {
        const k = l[0], body = l.slice(2).replace(/\t/g, "    ");
        if (k === "+") inner.push(`${C.dim}${String(n++).padStart(3)}${R} ${C.green}+ ${body}${R}`);
        else if (k === "-") inner.push(`${C.dim}${String(n++).padStart(3)}${R} ${C.red}- ${body}${R}`);
        else inner.push(`${C.dim}${String(n++).padStart(3)}   ${body}${R}`);
      }
      return [...boxLines(76, inner, C.dim), ""];
    },
    bash: (cmd, o) => [...boxLines(76, [`${C.green}✔${R}  ${B}Shell${R} ${cmd}`, "", ...o.map((l) => `${C.dim}${l}${R}`)], C.dim), ""],
    todo: (items) => items.map((it) => `  ${it[0] ? `${C.green}✓${R}` : "○"} ${it[1]}`).concat([""]),
    footer(s) {
      const w = s.cols - 1;
      const out = [];
      if (s.busy) out.push(`${C.blue}${BRAILLE[s.tick % BRAILLE.length]}${R} ${s.busy} ${C.dim}(esc to cancel, ${Math.max(1, Math.floor((s.now - s.busySince) / 1000))}s)${R}`, "");
      if (s.blocked) {
        out.push(...boxLines(Math.min(72, w), [`${C.yellow}?${R}  ${B}Shell${R} ${s.blocked}`, "", "Allow execution?", "", `${C.green}●${R} Yes, allow once`, "  Yes, allow always", "  No (esc)"], C.yellow));
      } else {
        out.push(...boxLines(w, [`${C.purple}>${R}   ${s.draft || `${C.dim}Type your message or @path/to/file${R}`}`], C.blue));
      }
      out.push(`${C.blue}${s.cwdShort}${R} ${C.purple}(${s.branch}*)${R}${" ".repeat(Math.max(1, w - 70))}${C.dim}sandbox: off${R}`);
      return out;
    },
  };

  // Generic plain style for the rest, coloured per agent.
  function plainStyle(name, accent, glyph) {
    return {
      header: (s) => [`${accent}${B}${glyph} ${name}${R}  ${C.dim}${s.cwdShort} · ${s.branch}${R}`, ""],
      prompt: (t) => [`${accent}❯${R} ${t}`, ""],
      say: (t) => [`${accent}${glyph}${R} ${t}`, ""],
      read: (f, n) => [`  ${C.dim}read${R}   ${f} ${C.dim}(${n} lines)${R}`],
      search: (q, n) => [`  ${C.dim}grep${R}   ${q} ${C.dim}(${n} files)${R}`],
      edit(f, lines, add, del) {
        const out = [`  ${C.yellow}edit${R}   ${f} ${C.green}+${add}${R} ${C.red}-${del}${R}`];
        for (const l of lines.slice(0, 6)) {
          const k = l[0], body = l.slice(2).replace(/\t/g, "    ");
          if (k === "+") out.push(`         ${C.green}+ ${body}${R}`);
          else if (k === "-") out.push(`         ${C.red}- ${body}${R}`);
          else out.push(`         ${C.dim}  ${body}${R}`);
        }
        return out;
      },
      bash: (cmd, o) => [`  ${C.cyan}run${R}    ${cmd}`, ...o.map((l) => `         ${C.dim}${l}${R}`)],
      todo: (items) => items.map((it) => `  ${it[0] ? `${C.green}[x]${R}` : "[ ]"} ${it[1]}`).concat([""]),
      footer(s) {
        const w = s.cols - 1;
        const out = [""];
        if (s.busy) out.push(`${accent}${BRAILLE[s.tick % BRAILLE.length]}${R} ${s.busy} ${C.dim}${Math.max(1, Math.floor((s.now - s.busySince) / 1000))}s${R}`);
        out.push(`${C.dim}${"─".repeat(w)}${R}`);
        out.push(`${accent}>${R} ${s.draft || `${C.dim}Message ${name}${R}`}`);
        out.push(`${C.dim}${"─".repeat(w)}${R}`);
        return out;
      },
    };
  }
  styles.opencode = plainStyle("opencode", rgb(250, 178, 131), "◆");
  styles.cursor = plainStyle("Cursor Agent", rgb(220, 220, 220), "⬢");
  styles.pi = plainStyle("pi", rgb(148, 226, 213), "π");
  styles.copilot = plainStyle("GitHub Copilot", rgb(180, 160, 255), "◉");
  styles.kimi = plainStyle("Kimi Code", rgb(245, 194, 231), "◐");
  styles.crush = plainStyle("Crush", rgb(166, 227, 161), "▲");
  styles.hermes = plainStyle("Hermes", rgb(203, 166, 247), "☿");
  styles.dsh = plainStyle("DeepSeek Harness", rgb(137, 220, 235), "◇");
  styles.shell = {
    header: () => [], prompt: (t) => [t], say: (t) => [t], read: () => [], search: () => [], edit: () => [],
    bash: (cmd, o) => [`${C.green}demo@orbit${R}:${C.blue}~/code/orbit-api${R}$ ${cmd}`, ...o],
    todo: () => [], footer: (s) => [`${C.green}demo@orbit${R}:${C.blue}~/code/orbit-api${R}$ ${s.draft || ""}`],
  };

  // ---------- Session engine ----------
  function mk(id, agent, opts) {
    const s = {
      id, agent, cols: 110, rows: 40, transcript: [], tick: 0, busy: null, busySince: 0, tokens: 0,
      blocked: null, draft: "", cursorOn: false, channels: [], queue: [], nextAt: 0, now: Date.now(),
      cwd: opts.cwd || "/Users/demo/code/orbit-api", cwdShort: (opts.cwd || "~/code/orbit-api").replace("/Users/demo", "~"),
      branch: opts.branch || "main", footerShown: 0, finished: false, onDone: opts.onDone,
    };
    s.style = styles[agent] || styles.opencode;
    s.transcript.push(...s.style.header(s));
    sessions[id] = s;
    return s;
  }

  function render(s) { return [...s.transcript, ...s.style.footer(s)]; }

  function writeAll(s, str) {
    if (!str) return;
    const b64 = btoa(unescape(encodeURIComponent(str)));
    for (const ch of s.channels) window.__mock.sendChannel(ch, { b64 });
  }
  function snapshot(s) {
    const lines = render(s);
    s.footerShown = s.style.footer(s).length;
    return `${E}?25l${E}2J${E}H` + lines.join("\r\n");
  }
  // Append transcript lines and redraw the footer in place.
  function redraw(s, newLines) {
    if (newLines && newLines.length) s.transcript.push(...newLines);
    if (!s.channels.length) return;
    const foot = s.style.footer(s);
    let out = "";
    if (s.footerShown > 0) out += `\r${s.footerShown - 1 > 0 ? `${E}${s.footerShown - 1}A` : ""}${E}J`;
    if (newLines && newLines.length) out += newLines.join("\r\n") + "\r\n";
    out += foot.join("\r\n");
    s.footerShown = foot.length;
    writeAll(s, out);
  }

  // Step kinds: [kind, ...args, {wait}]
  function runStep(s, st) {
    const [k, ...a] = st;
    const sty = s.style;
    switch (k) {
      case "prompt": s.draft = ""; redraw(s, sty.prompt(a[0])); break;
      case "type": s.draft = a[0]; redraw(s); break;
      case "busy": s.busy = a[0]; s.busySince = s.now; redraw(s); break;
      case "idle": s.busy = null; redraw(s); break;
      case "say": redraw(s, sty.say(a[0])); break;
      case "read": redraw(s, sty.read(a[0], a[1] || 120)); s.tokens += 1800; break;
      case "search": redraw(s, sty.search(a[0], a[1] || 4)); s.tokens += 600; break;
      case "edit": redraw(s, sty.edit(a[0], a[1], a[2], a[3] || 0)); s.tokens += 2400; break;
      case "bash": redraw(s, sty.bash(a[0], a[1])); s.tokens += 900; break;
      case "todo": redraw(s, sty.todo(a[0])); break;
      case "raw": redraw(s, a[0]); break;
      case "block": s.blocked = a[0]; s.busy = null; redraw(s); break;
      case "unblock": s.blocked = null; redraw(s); break;
      case "call": a[0](s); break;
    }
  }

  // Queue a script. Each step waits `gap` ms, or {w: ms} as its last element.
  function play(id, steps, { gap = 1400 } = {}) {
    const s = sessions[id];
    let t = Math.max(s.nextAt, Date.now());
    for (const raw of steps) {
      const last = raw[raw.length - 1];
      const hasW = last && typeof last === "object" && !Array.isArray(last) && "w" in last;
      const st = hasW ? raw.slice(0, -1) : raw;
      s.queue.push({ at: t, st });
      t += hasW ? last.w : gap;
    }
    s.nextAt = t;
  }
  // Apply queued steps immediately (history before the capture starts).
  function fastForward(id, ms) {
    const s = sessions[id];
    const until = Date.now() + ms;
    for (const q of s.queue) if (q.at <= until) q.at = 0;
  }

  function tickAll() {
    const now = Date.now();
    for (const s of Object.values(sessions)) {
      s.now = now;
      while (s.queue.length && s.queue[0].at <= now) runStep(s, s.queue.shift().st);
      if ((s.busy || s.cursorOn !== undefined) && s.channels.length) {
        const nt = Math.floor(now / 110);
        if (nt !== s.tick) {
          s.tick = nt;
          const blink = Math.floor(now / 530) % 2 === 0;
          const changed = s.busy || blink !== s.cursorOn;
          s.cursorOn = blink;
          if (s.busy) s.tokens += 37;
          if (changed) redraw(s);
        }
      }
    }
  }
  setInterval(tickAll, 50);

  function attach(id, channel, cols, rows) {
    const s = sessions[String(id)];
    if (!s) return;
    s.cols = Math.max(40, cols); s.rows = rows;
    tickAll();
    s.channels = [channel];
    setTimeout(() => writeAll(s, snapshot(s)), 0);
  }
  function preview(id, n) {
    const s = sessions[String(id)];
    if (!s) return "";
    s.now = Date.now();
    const lines = render(s).map(strip);
    return lines.slice(-n).join("\n");
  }
  function input(id, data) {
    const s = sessions[String(id)];
    if (!s) return;
    if (s.onInput) s.onInput(s, data);
  }

  // Build history synchronously (before anything attaches).
  function history(id, steps) {
    const s = sessions[id];
    for (const raw of steps) {
      const last = raw[raw.length - 1];
      const st = last && typeof last === "object" && !Array.isArray(last) && "w" in last ? raw.slice(0, -1) : raw;
      runStep(s, st);
    }
  }

  Object.assign(window.__TERM__, { history, mk, play, fastForward, attach, preview, input, redraw, styles, C });

  const H = window.__MOCK_HANDLERS__;
  H.attach_run = ({ id, cols, rows, onChunk }) => { attach(id, onChunk, cols, rows); return null; };
  H.run_preview = ({ id, lines }) => preview(id, lines);
  H.detach_run = ({ id }) => { const s = sessions[id]; if (s) s.channels = []; return null; };
  H.resize_run = ({ id, cols, rows }) => {
    const s = sessions[id];
    if (s && (s.cols !== cols)) { s.cols = Math.max(40, cols); s.rows = rows; if (s.channels.length) writeAll(s, snapshot(s)); }
    return null;
  };
})();

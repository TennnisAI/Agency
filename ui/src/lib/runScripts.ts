// Naming for run scripts. A script's name is only a label for the Run tab and
// the key of its session, so nobody should have to stop and invent one: the
// editor derives it from the command and only asks when the guess is wrong.

// Command runners that say nothing about what the script does. "pnpm dev" is a
// dev script, not a pnpm script, so the interesting word is the one after.
const RUNNERS = new Set([
  "npm", "pnpm", "yarn", "bun", "npx", "pnpx", "deno",
  "make", "cargo", "go", "docker", "python", "python3", "poetry", "uv",
  "rake", "dotnet", "mvn", "gradle", "sh", "bash", "zsh",
]);

// Words a runner takes before the real subcommand: `npm run build`, `pnpm exec vite`.
const FILLER = new Set(["run", "exec", "x", "task"]);

/// A short name for a command, e.g. "dev" for "pnpm dev --port $AGENCY_PORT" or
/// "./dev.sh". Empty for a command with nothing nameable in it, which is the
/// editor's cue to leave the field blank rather than invent something.
export function deriveScriptName(command: string): string {
  const words = command
    .trim()
    .split(/\s+/)
    // Flags, their values and leading env assignments describe how the command
    // runs, never what it is.
    .filter((w) => w && !w.startsWith("-") && !w.includes("=") && !w.includes("$"));

  let fallback = "";
  for (const word of words) {
    const bare = basename(word);
    if (!bare) continue;
    if (RUNNERS.has(word)) continue;
    if (FILLER.has(word)) {
      // "cargo run" and "go run ." have nothing after the filler worth naming,
      // so the filler itself is the best label left.
      fallback ||= word;
      continue;
    }
    return clean(bare);
  }
  return clean(fallback);
}

/// `base` if it is free, else the first "base2", "base3", … that is. Keeps the
/// derived name usable on a project that already runs a "dev".
export function uniqueScriptName(base: string, taken: string[]): string {
  if (!base || !taken.includes(base)) return base;
  for (let n = 2; ; n++) {
    const next = `${base}${n}`;
    if (!taken.includes(next)) return next;
  }
}

// The last path segment without its extension: "./scripts/start-web.sh" is a
// "start-web" script. "." and ".." name nothing.
function basename(word: string): string {
  const last = word.split("/").filter(Boolean).pop() ?? "";
  if (/^\.+$/.test(last)) return "";
  const dot = last.lastIndexOf(".");
  return dot > 0 ? last.slice(0, dot) : last;
}

// A name keys a session and is shown in a tab, so keep it to plain characters
// and short enough to read at a glance.
function clean(word: string): string {
  return word.replace(/[^A-Za-z0-9._-]/g, "").slice(0, 24);
}

// The ">" command registry (one-stop Phase 4). Ids are the `onMenu` action
// names in App.tsx — for native-menu items they equal the `menu:<id>` names in
// menu.rs, so the palette and the menu bar can't drift (a test scans both
// sources). Excluded on purpose: "palette" (it's already open) and "quit"
// (routed backend-side via lifecycle::request_quit, never through onMenu).

export type CommandWhen = "always" | "project" | "focusedAgent" | "workspaceVisible";

export interface PaletteCommand {
  /** onMenu action id. */
  id: string;
  label: string;
  sublabel: string;
  when: CommandWhen;
}

export const PALETTE_COMMANDS: PaletteCommand[] = [
  { id: "new-agent", label: "New Agent", sublabel: "Spawn the default agent in this project", when: "project" },
  { id: "new-terminal", label: "New Terminal", sublabel: "Open a terminal in this project", when: "project" },
  { id: "new-issue", label: "New Issue", sublabel: "Capture an issue in this project", when: "project" },
  { id: "daily-note", label: "Today's Note", sublabel: "Open today's journal entry in the workspace", when: "workspaceVisible" },
  { id: "weekly-note", label: "Generate Weekly Note", sublabel: "Assemble this week's merges, closed issues, and runs", when: "workspaceVisible" },
  { id: "workspace-guide", label: "Workspace Guide", sublabel: "Open the Welcome note explaining links, properties, and tasks", when: "workspaceVisible" },
  { id: "go-agents", label: "Go to Agents", sublabel: "Show this project's agents", when: "project" },
  { id: "go-issues", label: "Go to Issues", sublabel: "Show this project's issue board", when: "project" },
  { id: "go-docs", label: "Go to Docs", sublabel: "Show this project's docs", when: "project" },
  { id: "go-files", label: "Go to Files", sublabel: "Show this project's files", when: "project" },
  { id: "source", label: "Source & Diff", sublabel: "Show source control", when: "project" },
  // Always offered: find works on whatever is on screen, project or not.
  { id: "find", label: "Find…", sublabel: "Search the note, file, or board in front of you", when: "always" },
  { id: "replace", label: "Find and Replace…", sublabel: "Search and replace in the text you're editing", when: "always" },
  { id: "find-next", label: "Find Next", sublabel: "Jump to the next match", when: "always" },
  { id: "find-prev", label: "Find Previous", sublabel: "Jump to the previous match", when: "always" },
  { id: "approve", label: "Approve & Merge", sublabel: "Merge the focused agent's branch", when: "focusedAgent" },
  { id: "archive", label: "Archive Agent", sublabel: "Move the focused agent to the archive", when: "focusedAgent" },
  { id: "discard", label: "Discard Agent", sublabel: "Stop the focused agent and delete its run", when: "focusedAgent" },
  { id: "add-project", label: "Add Project…", sublabel: "Pick a folder to work in", when: "always" },
  { id: "clone-project", label: "Clone Repository…", sublabel: "Clone a repo and add it as a project", when: "always" },
  { id: "toggle-sidebar", label: "Toggle Sidebar", sublabel: "Show or hide the project list", when: "always" },
  { id: "home", label: "All Projects", sublabel: "Back to the overview", when: "always" },
  { id: "settings", label: "Settings…", sublabel: "Open settings", when: "always" },
  { id: "report-issue", label: "Report an Issue…", sublabel: "Open the GitHub issue form", when: "always" },
  { id: "github", label: "Agency on GitHub", sublabel: "Open the repository", when: "always" },
];

export interface CommandContext {
  hasProject: boolean;
  hasFocusedAgent: boolean;
  workspaceVisible: boolean;
}

/** The commands that would actually do something right now (mirrors the
 * native menu's set_context gating). */
export function availableCommands(ctx: CommandContext): PaletteCommand[] {
  return PALETTE_COMMANDS.filter((c) =>
    c.when === "always"
    || (c.when === "project" && ctx.hasProject)
    || (c.when === "focusedAgent" && ctx.hasFocusedAgent)
    || (c.when === "workspaceVisible" && ctx.workspaceVisible));
}

// Which tab a project opens on when it is selected, and the tab set itself.

export type Tab = "agents" | "source" | "files" | "issues" | "docs" | "run";

// Every project opened on the agents grid, workspace included — and the
// workspace is a notes vault whose grid is empty for most of its life, so the
// pinned notes home landed on nothing and put its Docs tab, the entire point of
// the place, one click away on every visit (AGE-198). Only the first selection
// takes this: after that the project's own remembered tab wins.
export function openingTab(kind: string | null | undefined): Tab {
  return kind === "workspace" ? "docs" : "agents";
}

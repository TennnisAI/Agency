import { describe, expect, it } from "vitest";
import { PALETTE_COMMANDS, availableCommands } from "./paletteCommands";

// Raw sources for the drift scans below (vite resolves ?raw at collect time).
import menuRs from "../../../crates/agency-app/src/menu.rs?raw";
import appTsx from "../App.tsx?raw";

const ids = PALETTE_COMMANDS.map((c) => c.id);

describe("availableCommands", () => {
  const all = { hasProject: true, hasFocusedAgent: true, workspaceVisible: true };

  it("returns everything when every context bit is on", () => {
    expect(availableCommands(all)).toHaveLength(PALETTE_COMMANDS.length);
  });

  it("hides project commands without a project", () => {
    const cmds = availableCommands({ ...all, hasProject: false }).map((c) => c.id);
    expect(cmds).not.toContain("new-agent");
    expect(cmds).not.toContain("go-issues");
    expect(cmds).toContain("settings");
  });

  it("hides agent commands without a focused agent", () => {
    const cmds = availableCommands({ ...all, hasFocusedAgent: false }).map((c) => c.id);
    expect(cmds).not.toContain("approve");
    expect(cmds).not.toContain("archive");
    expect(cmds).not.toContain("discard");
  });

  it("hides Today's Note when the workspace is hidden", () => {
    const cmds = availableCommands({ ...all, workspaceVisible: false }).map((c) => c.id);
    expect(cmds).not.toContain("daily-note");
  });
});

describe("registry drift", () => {
  it("covers every native-menu action (except palette/quit)", () => {
    const menuIds = [...new Set([...menuRs.matchAll(/"menu:([a-z-]+)"/g)].map((m) => m[1]))];
    expect(menuIds.length).toBeGreaterThan(10); // the scan found the real ids
    for (const id of menuIds) {
      if (id === "palette" || id === "quit") continue;
      expect(ids, `menu.rs action "${id}" missing from PALETTE_COMMANDS`).toContain(id);
    }
  });

  it("every registry id is routed by App.tsx onMenu", () => {
    for (const id of ids) {
      expect(appTsx, `App.tsx onMenu has no case for "${id}"`).toContain(`case "${id}"`);
    }
  });
});

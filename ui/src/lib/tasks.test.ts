import { describe, expect, it } from "vitest";
import { Project, TaskHit } from "../api";
import { groupTasks, isTaskExcluded, openTaskCount, promoteBody } from "./tasks";

const project = { id: "p1", name: "P" } as Project;
const hit = (path: string, line: number, checked: boolean, text: string): TaskHit => ({
  path, line, checked, text,
});

describe("groupTasks", () => {
  it("groups per note by path, tasks by line", () => {
    const groups = groupTasks(project, "docs", [
      hit("b.md", 4, false, "later"),
      hit("a/todo.md", 9, true, "done"),
      hit("b.md", 1, false, "first"),
    ]);
    expect(groups.map((g) => g.path)).toEqual(["a/todo.md", "b.md"]);
    expect(groups[0].title).toBe("todo");
    expect(groups[1].tasks.map((t) => t.line)).toEqual([1, 4]);
    expect(groups[0].docsDir).toBe("docs");
  });

  it("openTaskCount counts unchecked only", () => {
    const groups = groupTasks(project, "", [
      hit("a.md", 0, false, "x"),
      hit("a.md", 1, true, "y"),
      hit("b.md", 2, false, "z"),
    ]);
    expect(openTaskCount(groups)).toBe(2);
  });
});

describe("promoteBody", () => {
  it("links the source note by basename without extension", () => {
    expect(promoteBody("journal/2026-07-31.md")).toBe("From [[2026-07-31]]");
    expect(promoteBody("Plans.md")).toBe("From [[Plans]]");
  });
});

describe("isTaskExcluded", () => {
  it("matches whole projects, folders, and exact notes", () => {
    const rules = [
      { projectId: "p1", prefix: "" },
      { projectId: "p2", prefix: "docs/superpowers/plans" },
      { projectId: "p3", prefix: "todo.md" },
    ];
    expect(isTaskExcluded("p1", "anything.md", rules)).toBe(true);
    expect(isTaskExcluded("p2", "docs/superpowers/plans/phase8.md", rules)).toBe(true);
    expect(isTaskExcluded("p2", "docs/superpowers/plans.md", rules)).toBe(false);
    expect(isTaskExcluded("p2", "docs/other.md", rules)).toBe(false);
    expect(isTaskExcluded("p3", "todo.md", rules)).toBe(true);
    expect(isTaskExcluded("p3", "sub/todo.md", rules)).toBe(false);
    expect(isTaskExcluded("p4", "anything.md", rules)).toBe(false);
  });
});

import { describe, expect, it } from "vitest";
import { initialCapture, feed } from "./firstPrompt";

function run(chunks: string[]) {
  let state = initialCapture();
  const lines: string[] = [];
  for (const c of chunks) {
    const r = feed(state, c);
    state = r.state;
    if (r.line !== null) lines.push(r.line);
  }
  return { state, lines };
}

describe("firstPrompt capture", () => {
  it("captures a line submitted with carriage return", () => {
    const { lines, state } = run(["hello world", "\r"]);
    expect(lines).toEqual(["hello world"]);
    expect(state.done).toBe(true);
  });

  it("captures across multiple chunks", () => {
    const { lines } = run(["fix ", "the ", "bug\r"]);
    expect(lines).toEqual(["fix the bug"]);
  });

  it("applies backspace (\\x7f)", () => {
    const { lines } = run(["hellp\x7fo\r"]);
    expect(lines).toEqual(["hello"]);
  });

  it("ignores empty submissions and keeps waiting", () => {
    const { lines, state } = run(["\r", "   \r", "real\r"]);
    expect(lines).toEqual(["real"]);
    expect(state.done).toBe(true);
  });

  it("stops capturing after the first line", () => {
    const { lines } = run(["first\r", "second\r"]);
    expect(lines).toEqual(["first"]);
  });

  it("ignores bare control characters", () => {
    const { lines } = run(["a\x00\x01b\r"]); // NUL/SOH dropped, letters kept
    expect(lines).toEqual(["ab"]);
  });
});

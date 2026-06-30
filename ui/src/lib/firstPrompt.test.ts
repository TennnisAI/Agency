import { describe, expect, it, test } from "vitest";
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

test("skips an OSC color-report reply, captures real typing", () => {
  let s = initialCapture();
  // xterm.js reply to an OSC 11 query, terminated by ST (ESC \)
  s = feed(s, "\x1b]11;rgb:b3b3/bcbc/b2b2\x1b\\").state;
  const r = feed(s, "hi\r");
  expect(r.line).toBe("hi");
});

test("skips a CSI DECRQM reply", () => {
  let s = initialCapture();
  s = feed(s, "\x1b[?1016;2$y").state;
  const r = feed(s, "ok\r");
  expect(r.line).toBe("ok");
});

test("skips an escape sequence split across chunks", () => {
  let s = initialCapture();
  s = feed(s, "\x1b]11;rgb:b3b3").state; // first half of OSC
  s = feed(s, "/bcbc/b2b2\x07").state;   // rest + BEL terminator
  const r = feed(s, "go\r");
  expect(r.line).toBe("go");
});

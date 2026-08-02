import { describe, it, expect } from "vitest";
import { Terminal } from "@xterm/xterm";
import { follow, GESTURE_MS } from "./termFollow";

describe("follow", () => {
  it("keeps following while the viewport sits on the newest output", () => {
    expect(follow({ viewportY: 400, baseY: 400, gesture: false, following: true }))
      .toEqual({ following: true, repin: false });
  });

  it("hands the pane over when the user scrolls up", () => {
    expect(follow({ viewportY: 380, baseY: 400, gesture: true, following: true }))
      .toEqual({ following: false, repin: false });
  });

  it("leaves a reader where they are as output keeps arriving", () => {
    // Same gesture-less event a stuck pane produces, but this reader chose it.
    expect(follow({ viewportY: 380, baseY: 460, gesture: false, following: false }))
      .toEqual({ following: false, repin: false });
  });

  it("follows again once the user scrolls back to the bottom", () => {
    expect(follow({ viewportY: 460, baseY: 460, gesture: false, following: false }))
      .toEqual({ following: true, repin: false });
  });

  it("puts the pane back when it jumps up on its own (AGE-19)", () => {
    // The viewport is thrown ~30 lines back with nothing driving it.
    expect(follow({ viewportY: 12, baseY: 42, gesture: false, following: true }))
      .toEqual({ following: true, repin: true });
  });

  it("puts the pane back when output runs on past a viewport that stopped", () => {
    // The state that leaves behind: the latch is stuck, so the base advances
    // line by line while the viewport never moves.
    expect(follow({ viewportY: 0, baseY: 1, gesture: false, following: true }))
      .toEqual({ following: true, repin: true });
  });

  it("settles after the pin lands", () => {
    const jumped = follow({ viewportY: 12, baseY: 42, gesture: false, following: true });
    expect(jumped.repin).toBe(true);
    expect(follow({ viewportY: 42, baseY: 42, gesture: false, following: jumped.following }))
      .toEqual({ following: true, repin: false });
  });
});

// Against a real xterm, wired the way FocusTerminal wires it. Headless: with no
// DOM there is no viewport, so the scrolls it would drive — the user's, the
// phantom ones, and the pin's own `term.scrollToBottom()` — are stood in for by
// the buffer-level scrolls each performs, `suppressScrollEvent` included.
describe("follow, on a live xterm buffer", () => {
  function pane(policy = true) {
    const term = new Terminal({ rows: 10, cols: 40, scrollback: 1000 });
    const buf = (term as any)._core._bufferService;
    let following = true;
    let repinning = false;
    let gesturing = false;
    const atBottom = () => buf.buffer.ydisp >= buf.buffer.ybase;
    if (policy) term.onScroll(() => {
      if (repinning) return;
      const next = follow({
        viewportY: buf.buffer.ydisp,
        baseY: buf.buffer.ybase,
        gesture: gesturing,
        following,
      });
      following = next.following;
      if (!next.repin) return;
      repinning = true;
      buf.scrollLines(buf.buffer.ybase - buf.buffer.ydisp);
      repinning = false;
    });
    // Everything the viewport drives is suppressed: the policy is never told.
    const viewportScroll = (lines: number) => buf.scrollLines(lines, true);
    // A scroll with a gesture behind it, and the sample that lands after it.
    const userScroll = (lines: number) => {
      gesturing = true;
      viewportScroll(lines);
      if (policy) following = atBottom();
      gesturing = false;
    };
    const write = (s: string) => new Promise<void>((r) => term.write(s, r));
    const print = async (tag: string, n: number) => {
      for (let i = 0; i < n; i++) await write(`${tag} ${i}\r\n`);
    };
    const behind = () => buf.buffer.ybase - buf.buffer.ydisp;
    return { term, buf, print, behind, viewportScroll, userScroll, isFollowing: () => following };
  }

  it("is what stops a pane drifting away from the agent (AGE-19)", async () => {
    // One scroll nobody made — xterm's viewport turning a stale scrollTop into
    // a buffer row — and an unpinned pane is parked for good: the latch is set,
    // there is no second scroll coming to clear it, and every line the agent
    // prints from here advances the base alone.
    const bare = pane(false);
    await bare.print("line", 60);
    bare.viewportScroll(-5);
    await bare.print("more", 40);
    expect(bare.buf.isUserScrolling).toBe(true);
    expect(bare.behind()).toBe(45); // the 5 it was scrolled, plus every new line

    const pinned = pane();
    await pinned.print("line", 60);
    pinned.viewportScroll(-5);
    await pinned.print("more", 40);
    expect(pinned.buf.isUserScrolling).toBe(false);
    expect(pinned.behind()).toBe(0);
  });

  it("recovers a pane thrown all the way to the opening prompt", async () => {
    // xterm's own warning about an unreliable scrollTop: it "causes the
    // terminal to scroll the buffer to the top". The scrollback is untouched,
    // so line 0 is still the first thing the agent ever printed.
    const p = pane();
    await p.print("line", 60);
    p.viewportScroll(-p.buf.buffer.ydisp);
    expect(p.buf.buffer.ydisp).toBe(0);

    await p.print("more", 1); // the next line of output is all it takes
    expect(p.behind()).toBe(0);
    expect(p.buf.buffer.lines.get(0).translateToString(true)).toContain("line 0");
  });

  it("does not let repeated phantom scrolls accumulate", async () => {
    const p = pane();
    await p.print("line", 60);
    for (let i = 0; i < 5; i++) {
      p.viewportScroll(-7);
      await p.print("more", 3);
      expect(p.behind()).toBe(0);
    }
  });

  it("leaves a reader who scrolled up alone while the agent keeps writing", async () => {
    const p = pane();
    await p.print("line", 60);
    p.userScroll(-8);

    const parked = p.buf.buffer.ydisp;
    await p.print("more", 30);
    expect(p.buf.buffer.ydisp).toBe(parked);
    expect(p.isFollowing()).toBe(false);
  });

  it("still leaves them alone when output only resumes much later", async () => {
    // The gesture is long over by the time the next line arrives, so the pane
    // has to have taken the reader's position from the sample, not the clock.
    const p = pane();
    await p.print("line", 60);
    p.userScroll(-8);
    const parked = p.buf.buffer.ydisp;

    await new Promise((r) => setTimeout(r, GESTURE_MS + 50));
    await p.print("more", 10);
    expect(p.buf.buffer.ydisp).toBe(parked);
  });

  it("follows again once the reader scrolls back down", async () => {
    const p = pane();
    await p.print("line", 60);
    p.userScroll(-8);
    p.userScroll(8);

    await p.print("more", 30);
    expect(p.behind()).toBe(0);
    expect(p.isFollowing()).toBe(true);
  });
});

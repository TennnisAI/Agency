/**
 * Keeping a terminal pane pinned to the newest output.
 *
 * A pane can only show older output than the agent is writing while xterm's
 * `isUserScrolling` is set: with it clear, every line the buffer scrolls also
 * drags the viewport to the bottom. It is a latch — the first scroll that moves
 * the viewport up sets it, and only a later scroll that lands back at the
 * bottom clears it — so one scroll the user never made parks the pane for good,
 * and each further one drags it further back until it is sitting on the opening
 * prompt with the agent still working. That is AGE-19.
 *
 * Those scrolls come from `Viewport`, which turns the viewport element's
 * scrollTop into a buffer row and scrolls by the difference. Whenever that
 * scrollTop is stale against a buffer the agent is still growing — the scroll
 * area is resized a frame behind the buffer, and xterm's own source warns the
 * value goes outright bad when the element isn't visible, "which causes the
 * terminal to scroll the buffer to the top" — the difference comes out negative
 * and the pane is scrolled up by it. Nothing distinguishes that from the user
 * reaching for the scrollbar, so nothing puts it back.
 *
 * So the pane keeps its own answer to "should this follow the tail?" and
 * corrects xterm when the two disagree. The rule reads the viewport rather than
 * trying to model what moved it, so it holds however the pane got scrolled:
 *
 *   - viewport at the bottom            → following; nothing to do
 *   - viewport away from it, on the back of a real gesture
 *                                       → the user is reading; leave it alone
 *   - viewport away from it, on its own  → put it back
 *
 * Note that xterm reports none of the viewport's own scrolling through
 * `onScroll` (it suppresses the event to avoid feeding the viewport its own
 * scrolls), so a pane hears about a user's wheel only by sampling the position
 * after the gesture. `onScroll` on its own would also mean a pane hears about a
 * phantom only on the next line of output, which for a pane sitting at a shell
 * prompt never comes: the companion terminal drifted up the moment focus went
 * to the agent pane and stayed there, with the latch set and nothing coming to
 * clear it (AGE-88). So the rule is fed from the viewport element's own DOM
 * scroll event as well, which xterm does not suppress, and from anything else
 * that can move the buffer under the viewport without one — a reflow after a
 * resize, in particular.
 */

/**
 * How long a scroll still counts as user-driven after the gesture that caused
 * it. The DOM scroll event trails the wheel/pointer event that produced it by a
 * frame or two, so this only has to outlive that hop; a trackpad glide keeps
 * refreshing it with every notch.
 */
export const GESTURE_MS = 500;

export interface FollowInput {
  /** `IBuffer.viewportY`: buffer line shown at the top of the pane. */
  viewportY: number;
  /** `IBuffer.baseY`: buffer line the bottom-most screen starts at. */
  baseY: number;
  /** A real scroll gesture (wheel, drag, page key) is in flight. */
  gesture: boolean;
  /** Whether the pane was following the newest output until now. */
  following: boolean;
}

export interface FollowResult {
  following: boolean;
  /** Scroll the pane back to the bottom: it fell behind on its own. */
  repin: boolean;
}

/** Decide what a pane should do now that its viewport position has changed. */
export function follow({ viewportY, baseY, gesture, following }: FollowInput): FollowResult {
  if (viewportY >= baseY) return { following: true, repin: false };
  if (gesture) return { following: false, repin: false };
  // Off the bottom with nobody driving: either the viewport was scrolled up
  // behind our back, or output ran on underneath a viewport xterm stopped
  // advancing. Both are the same fix, and both are only ours to make if the
  // pane was following — a reader who scrolled away stays where they put it.
  return { following, repin: following };
}

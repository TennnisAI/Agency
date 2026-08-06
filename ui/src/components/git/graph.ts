export type Lane = { target: string; color: string };

/**
 * One drawable rail within a row, in column coordinates.
 * - `pass`: a lane flowing straight through the row (top edge → bottom edge).
 * - `enter`: an incoming lane ending at this row's commit dot (top edge → dot).
 * - `exit`: a lane born at this row's commit dot (dot → bottom edge), i.e. a parent.
 * `from` is the column at the segment's start, `to` at its end; when they differ
 * the renderer draws a curved connector, which is what keeps merges/forks joined.
 */
export type Segment = {
  kind: "pass" | "enter" | "exit";
  from: number;
  to: number;
  color: string;
};

export type GraphRow = {
  hash: string;
  input: Lane[];
  output: Lane[];
  circleIndex: number;
  color: string;
  /** Merge commit (2+ parents) — rendered as a hollow dot. */
  isMerge: boolean;
  segments: Segment[];
  /** Columns this row actually occupies (the width its graph gutter needs). */
  lanes: number;
};

export const PALETTE = ["--blue", "--peach", "--pink", "--yellow", "--teal", "--mauve", "--green"];

export function computeGraph(commits: { hash: string; parents: string[] }[]): GraphRow[] {
  const rows: GraphRow[] = [];
  let colorIndex = 0;
  const nextColor = () => {
    const c = PALETTE[colorIndex];
    colorIndex = (colorIndex + 1) % PALETTE.length;
    return c;
  };

  let input: Lane[] = [];
  for (const commit of commits) {
    const output: Lane[] = [];
    const segments: Segment[] = [];
    let circleIndex = -1;
    let firstParentPlaced = false;

    // Lanes at a column left of the dot never collapse before it, so the first
    // matching lane (the dot) keeps its column in `output` for its first parent.
    input.forEach((lane, i) => {
      if (lane.target === commit.hash) {
        if (circleIndex === -1) {
          circleIndex = i;
          if (commit.parents[0]) {
            output.push({ target: commit.parents[0], color: lane.color });
            firstParentPlaced = true;
          }
        }
        // Every lane targeting this commit bends into the dot (first one is vertical).
        segments.push({ kind: "enter", from: i, to: circleIndex, color: lane.color });
      } else {
        segments.push({ kind: "pass", from: i, to: output.length, color: lane.color });
        output.push({ ...lane });
      }
    });

    const commitColor = circleIndex === -1 ? nextColor() : input[circleIndex].color;
    if (circleIndex === -1) circleIndex = input.length;

    // Tip (no incoming lane): place first parent now.
    if (!firstParentPlaced && commit.parents[0]) {
      output.push({ target: commit.parents[0], color: commitColor });
      segments.push({ kind: "exit", from: circleIndex, to: output.length - 1, color: commitColor });
    } else if (firstParentPlaced) {
      segments.push({ kind: "exit", from: circleIndex, to: circleIndex, color: commitColor });
    }
    // Extra parents (merge) each add a new lane leaving the dot.
    for (let p = 1; p < commit.parents.length; p++) {
      const color = nextColor();
      output.push({ target: commit.parents[p], color });
      segments.push({ kind: "exit", from: circleIndex, to: output.length - 1, color });
    }

    const lanes = Math.max(input.length, output.length, circleIndex + 1);
    rows.push({
      hash: commit.hash,
      input,
      output,
      circleIndex,
      color: commitColor,
      isMerge: commit.parents.length > 1,
      segments,
      lanes,
    });
    input = output.map((l) => ({ ...l }));
  }
  return rows;
}

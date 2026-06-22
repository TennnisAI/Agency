export type Lane = { target: string; color: string };
export type GraphRow = {
  hash: string;
  input: Lane[];
  output: Lane[];
  circleIndex: number;
  color: string;
};

export const PALETTE = ["--peach", "--pink", "--yellow", "--teal", "--mauve"];

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
    let circleIndex = -1;
    let firstParentPlaced = false;

    input.forEach((lane, i) => {
      if (lane.target === commit.hash) {
        if (circleIndex === -1) {
          circleIndex = i;
          // first lane targeting commit continues as first parent
          if (commit.parents[0]) {
            output.push({ target: commit.parents[0], color: lane.color });
            firstParentPlaced = true;
          }
        }
        // additional lanes targeting commit collapse (dropped)
      } else {
        output.push({ ...lane });
      }
    });

    const commitColor = circleIndex === -1 ? nextColor() : input[circleIndex].color;
    if (circleIndex === -1) circleIndex = input.length;

    // tip (no incoming lane): place first parent now
    if (!firstParentPlaced && commit.parents[0]) {
      output.push({ target: commit.parents[0], color: commitColor });
    }
    // extra parents (merge) each add a new lane
    for (let p = 1; p < commit.parents.length; p++) {
      output.push({ target: commit.parents[p], color: nextColor() });
    }

    rows.push({ hash: commit.hash, input, output, circleIndex, color: commitColor });
    input = output.map((l) => ({ ...l }));
  }
  return rows;
}

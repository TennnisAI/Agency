import { describe, expect, it } from "vitest";
import { quitBody } from "./quitPrompt";

describe("quitBody", () => {
  it("promises nothing is interrupted only when nothing is running", () => {
    expect(quitBody(0)).toBe("All agents are idle; nothing will be interrupted.");
  });

  it("offers both ways out, in the singular", () => {
    expect(quitBody(1)).toBe(
      "1 session is running. Leave it running and it carries on while Agency is closed, "
        + "without notifications, until you reopen it. Stop it and agents resume where they "
        + "left off next time.",
    );
  });

  it("offers both ways out, in the plural", () => {
    const body = quitBody(12);
    expect(body).toMatch(/^12 sessions are running\. Leave them running and they carry on/);
    expect(body).toContain("Stop them and agents resume");
  });

  it("says the sessions go unwatched while the app is closed", () => {
    // Nothing notifies once the app has exited: saying "carry on" alone would
    // promise the turn-finished pings too.
    expect(quitBody(3)).toContain("without notifications");
  });

  it("has no em dash", () => {
    for (const n of [0, 1, 2]) expect(quitBody(n)).not.toContain("—");
  });
});

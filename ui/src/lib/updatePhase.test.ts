import { describe, expect, it } from "vitest";
import { InstallKind, UpdateCheck } from "../api";
import { EMPTY_CHECK, Phase, afterUpdateChecked, headline, installerNote, phaseFor, subhead } from "./updatePhase";

const found = (over: Partial<UpdateCheck> = {}): UpdateCheck => ({
  ...EMPTY_CHECK,
  current: "0.2.0",
  latest: "0.3.0",
  updateAvailable: true,
  installKind: "mac-app",
  canInstall: true,
  ...over,
});

describe("phaseFor", () => {
  it("opens on the report when there is something to install", () => {
    expect(phaseFor(found()).kind).toBe("report");
  });

  it("opens on the restart prompt for a release already staged", () => {
    expect(phaseFor(found({ staged: "0.3.0", updateAvailable: false }))).toEqual({ kind: "installed", version: "0.3.0" });
  });

  it("still reports a release newer than the staged one", () => {
    expect(phaseFor(found({ staged: "0.3.0", latest: "0.4.0" })).kind).toBe("report");
  });

  it("opens on the download when one is already running", () => {
    expect(phaseFor(found({ installing: true, canInstall: false })).kind).toBe("installing");
  });
});

describe("afterUpdateChecked", () => {
  const installing: Phase = { kind: "installing", check: found(), progress: { downloaded: 10, total: 100 } };

  it("follows an install another dialog started", () => {
    expect(afterUpdateChecked({ kind: "report", check: found() }, found({ installing: true })).kind).toBe("installing");
  });

  it("leaves a download's progress alone while it runs", () => {
    expect(afterUpdateChecked(installing, found({ installing: true }))).toBe(installing);
  });

  it("moves to the restart prompt when the install lands", () => {
    expect(afterUpdateChecked(installing, found({ staged: "0.3.0", updateAvailable: false }))).toEqual({ kind: "installed", version: "0.3.0" });
  });

  it("moves to the failure when the install fails", () => {
    const next = afterUpdateChecked(installing, found({ installError: "signature did not verify" }));
    expect(next).toMatchObject({ kind: "failed", message: "signature did not verify" });
  });

  it("does not disturb a report for an ordinary check", () => {
    const report: Phase = { kind: "report", check: found() };
    expect(afterUpdateChecked(report, found({ latest: "0.4.0" }))).toBe(report);
  });
});

describe("copy", () => {
  it("counts running sessions in the restart prompt", () => {
    const installed: Phase = { kind: "installed", version: "0.3.0" };
    expect(subhead(installed, null)).toBe("Restart when it suits you.");
    expect(subhead(installed, 0)).toContain("Nothing is running");
    expect(subhead(installed, 1)).toContain("1 session is running");
    expect(subhead(installed, 3)).toContain("3 sessions are running");
  });

  it("tells a copy on a read-only mount to move into Applications", () => {
    expect(installerNote(found({ installKind: "mac-read-only", canInstall: false }))).toContain("Move Agency into Applications");
  });

  it("says the download carries on when the dialog closes", () => {
    const phase: Phase = { kind: "installing", check: found(), progress: null };
    expect(headline(phase)).toBe("Downloading Agency 0.3.0");
    expect(subhead(phase, null)).toContain("the download carries on");
  });

  it("uses no em dashes in anything the user reads", () => {
    const kinds: InstallKind[] = ["mac-app", "mac-read-only", "app-image", "deb", "rpm", "pacman", "unknown"];
    const phases: Phase[] = [
      { kind: "checking" },
      { kind: "report", check: found() },
      { kind: "report", check: found({ error: "offline" }) },
      { kind: "report", check: found({ updateAvailable: false }) },
      { kind: "installing", check: found(), progress: null },
      { kind: "installed", version: "0.3.0" },
      { kind: "failed", check: found(), message: "x" },
    ];
    const copy = [
      ...phases.flatMap((p) => [headline(p), subhead(p, 2)]),
      ...kinds.map((k) => installerNote(found({ installKind: k }))),
    ];
    for (const s of copy) expect(s).not.toContain("—");
  });
});

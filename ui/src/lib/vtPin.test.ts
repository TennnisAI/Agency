import { describe, it, expect } from "vitest";
import ui from "../../package.json";
import installed from "@xterm/xterm/package.json";

/**
 * The pane's VT parser is pinned, and this is the half of the check that reads
 * the artifact rather than a manifest: node_modules is what vite bundles, so a
 * stale install runs a parser other than the recorded one with nothing to say
 * so. It is the frontend's version of a stale `agency-termd` sidecar.
 *
 * The recorded version lives in `crates/agency-core/src/term/vt_pin.rs`, which
 * explains why either parser is pinned at all. A test there ties it to
 * `ui/package.json`; this ties `ui/package.json` to what is installed.
 */
describe("the @xterm/xterm pin", () => {
  const pinned = ui.dependencies["@xterm/xterm"];

  it("is an exact version, not a range", () => {
    expect(pinned).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it("is the version actually installed", () => {
    expect(installed.version).toBe(pinned);
  });
});

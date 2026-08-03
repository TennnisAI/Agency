import { describe, it, expect } from "vitest";
import { deriveScriptName, uniqueScriptName } from "./runScripts";

describe("deriveScriptName", () => {
  it("names a package-manager script after the script, not the manager", () => {
    expect(deriveScriptName("pnpm dev --port $AGENCY_PORT")).toBe("dev");
    expect(deriveScriptName("npm run build")).toBe("build");
    expect(deriveScriptName("yarn serve")).toBe("serve");
    expect(deriveScriptName("bun run start --port $AGENCY_PORT")).toBe("start");
  });

  it("ignores leading env assignments", () => {
    expect(deriveScriptName("PORT=$AGENCY_PORT npm run start")).toBe("start");
  });

  it("uses a script file's stem", () => {
    expect(deriveScriptName("./dev.sh")).toBe("dev");
    expect(deriveScriptName("./scripts/start-web.sh")).toBe("start-web");
  });

  it("falls back to the subcommand when nothing follows it", () => {
    expect(deriveScriptName("cargo run")).toBe("run");
    expect(deriveScriptName("go run .")).toBe("run");
  });

  it("reads the interesting word out of other toolchains", () => {
    expect(deriveScriptName("docker compose up")).toBe("compose");
    expect(deriveScriptName("make dev")).toBe("dev");
  });

  it("is empty for a command with nothing nameable in it", () => {
    expect(deriveScriptName("")).toBe("");
    expect(deriveScriptName("   ")).toBe("");
    expect(deriveScriptName("--help")).toBe("");
  });

  it("keeps the name to plain, short characters", () => {
    expect(deriveScriptName("./dev(2).sh")).toBe("dev2");
    expect(deriveScriptName(`./${"a".repeat(40)}.sh`)).toHaveLength(24);
  });
});

describe("uniqueScriptName", () => {
  it("leaves a free name alone", () => {
    expect(uniqueScriptName("dev", ["build"])).toBe("dev");
    expect(uniqueScriptName("", ["dev"])).toBe("");
  });

  it("suffixes past every taken name", () => {
    expect(uniqueScriptName("dev", ["dev"])).toBe("dev2");
    expect(uniqueScriptName("dev", ["dev", "dev2", "dev3"])).toBe("dev4");
  });
});

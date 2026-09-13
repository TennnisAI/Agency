import { describe, expect, it, vi } from "vitest";
import { launchModel } from "./launchModel";

describe("launchModel", () => {
  it("repeats the remembered model when the caller had no picker", async () => {
    const remembered = vi.fn(async () => "opus");
    expect(await launchModel(undefined, remembered)).toBe("opus");
    expect(remembered).toHaveBeenCalledOnce();
  });

  it("launches on a picked model without asking what was remembered", async () => {
    const remembered = vi.fn(async () => "opus");
    expect(await launchModel("sonnet", remembered)).toBe("sonnet");
    expect(remembered).not.toHaveBeenCalled();
  });

  // The agent's own default is a choice, not the absence of one: it must not
  // be swapped for the remembered model.
  it("keeps an explicit pick of the agent's default", async () => {
    const remembered = vi.fn(async () => "opus");
    expect(await launchModel(null, remembered)).toBeNull();
    expect(remembered).not.toHaveBeenCalled();
  });

  it("falls to the agent's default when nothing is remembered", async () => {
    expect(await launchModel(undefined, async () => null)).toBeNull();
  });
});

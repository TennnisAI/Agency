import { describe, expect, it } from "vitest";
import { backdropDismisses } from "./useBackdropDismiss";

const sheet = { name: "sheet" } as unknown as EventTarget;
const input = { name: "input" } as unknown as EventTarget;
const button = { name: "button" } as unknown as EventTarget;

describe("backdropDismisses", () => {
  it("dismisses a click that starts and ends on the sheet", () => {
    expect(backdropDismisses(sheet, sheet, sheet)).toBe(true);
  });

  it("keeps the dialog when a text selection drags out onto the sheet", () => {
    expect(backdropDismisses(input, sheet, sheet)).toBe(false);
  });

  it("keeps the dialog when a press on the sheet is released inside it", () => {
    expect(backdropDismisses(sheet, button, sheet)).toBe(false);
  });

  it("keeps the dialog when no press was seen", () => {
    expect(backdropDismisses(null, sheet, sheet)).toBe(false);
  });
});

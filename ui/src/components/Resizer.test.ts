import { describe, expect, it } from "vitest";
import { resizerClass, axisCoord } from "./Resizer";

describe("resizerClass", () => {
  it("returns 'resizer' for vertical", () => {
    expect(resizerClass("vertical")).toBe("resizer");
  });
  it("returns 'resizer horizontal' for horizontal", () => {
    expect(resizerClass("horizontal")).toBe("resizer horizontal");
  });
});

describe("axisCoord", () => {
  it("vertical orientation reads clientX", () => {
    expect(axisCoord("vertical", 10, 99)).toBe(10);
  });
  it("horizontal orientation reads clientY", () => {
    expect(axisCoord("horizontal", 10, 99)).toBe(99);
  });
});

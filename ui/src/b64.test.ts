import { describe, expect, it } from "vitest";
import { b64ToBytes } from "./b64";

describe("b64ToBytes", () => {
  it("decodes ascii", () => {
    const bytes = b64ToBytes(btoa("hi\n"));
    expect(Array.from(bytes)).toEqual([104, 105, 10]);
  });

  it("decodes a high byte", () => {
    // base64 of [0xff, 0x00]
    const bytes = b64ToBytes("/wA=");
    expect(Array.from(bytes)).toEqual([255, 0]);
  });
});

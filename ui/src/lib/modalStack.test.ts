import { describe, expect, it } from "vitest";
import { pushModal, topModal } from "./modalStack";

describe("modalStack", () => {
  it("only the last-registered modal is on top", () => {
    const a = () => {};
    const b = () => {};
    const popA = pushModal(a);
    expect(topModal()).toBe(a);
    const popB = pushModal(b);
    expect(topModal()).toBe(b);
    popB();
    expect(topModal()).toBe(a);
    popA();
    expect(topModal()).toBeNull();
  });

  it("removing a buried modal keeps the top intact", () => {
    const a = () => {};
    const b = () => {};
    const popA = pushModal(a);
    const popB = pushModal(b);
    popA(); // e.g. host modal unmounts while its child confirm is open
    expect(topModal()).toBe(b);
    popB();
    expect(topModal()).toBeNull();
  });

  it("unregister is idempotent", () => {
    const a = () => {};
    const popA = pushModal(a);
    popA();
    popA();
    expect(topModal()).toBeNull();
  });
});

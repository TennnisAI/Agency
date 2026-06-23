import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import Resizer from "./Resizer";

describe("Resizer", () => {
  it("defaults to a vertical separator", () => {
    const { container } = render(<Resizer width={100} min={0} max={200} onChange={() => {}} />);
    const el = container.querySelector(".resizer")!;
    expect(el.className).not.toContain("horizontal");
    expect(el.getAttribute("aria-orientation")).toBe("vertical");
  });

  it("renders a horizontal separator when orientation is horizontal", () => {
    const { container } = render(
      <Resizer width={100} min={0} max={200} onChange={() => {}} orientation="horizontal" />,
    );
    const el = container.querySelector(".resizer")!;
    expect(el.className).toContain("horizontal");
    expect(el.getAttribute("aria-orientation")).toBe("horizontal");
  });
});

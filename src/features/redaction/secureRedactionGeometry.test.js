import { describe, expect, it } from "vitest";
import {
  logicalRectToViewportRect,
  viewportDragToLogicalRect,
} from "./secureRedactionGeometry.js";

describe("secure redaction review geometry", () => {
  const logical = { x: 0.1, y: 0.2, width: 0.3, height: 0.25 };

  it.each([0, 90, 180, 270])("round-trips logical regions at viewer rotation %i", (rotation) => {
    const viewport = logicalRectToViewportRect(logical, rotation);
    const recovered = viewportDragToLogicalRect(
      { x: viewport.x, y: viewport.y },
      { x: viewport.x + viewport.width, y: viewport.y + viewport.height },
      rotation,
    );
    expect(recovered.x).toBeCloseTo(logical.x, 10);
    expect(recovered.y).toBeCloseTo(logical.y, 10);
    expect(recovered.width).toBeCloseTo(logical.width, 10);
    expect(recovered.height).toBeCloseTo(logical.height, 10);
  });

  it("keeps the stored logical rectangle unchanged when presentation rotation changes", () => {
    const original = structuredClone(logical);
    expect(logicalRectToViewportRect(logical, 90)).not.toEqual(logicalRectToViewportRect(logical, 0));
    expect(logical).toEqual(original);
  });

  it("rejects zero-area and non-finite drags while clamping pointer bounds", () => {
    expect(viewportDragToLogicalRect({ x: 0.5, y: 0.5 }, { x: 0.5, y: 0.5 }, 0)).toBeNull();
    expect(viewportDragToLogicalRect({ x: NaN, y: 0 }, { x: 1, y: 1 }, 0)).toBeNull();
    expect(viewportDragToLogicalRect({ x: -1, y: -1 }, { x: 2, y: 2 }, 0)).toEqual({ x: 0, y: 0, width: 1, height: 1 });
  });
});

import { describe, expect, it } from "vitest";
import {
  rotateNormalizedRectForViewport,
  composeRenderedPageRotation,
} from "./pdfGeometry.js";
import { rotateSearchBox } from "./searchResultUtils.js";

describe("Viewer Rotation Characterization", () => {
  it("characterizes intrinsic vs viewer rotation composition", () => {
    // Rust worker renders main canvas using page.rotation() (intrinsic rotation).
    // Intrinsic 90° + Viewer 90° = 180° total visual rotation.
    const intrinsicRotation = 90;
    const viewerRotation = 90;
    const combinedRotation = composeRenderedPageRotation(
      intrinsicRotation,
      viewerRotation,
    );
    expect(combinedRotation).toBe(180);
  });

  it("characterizes normalized box rotation for viewport rendering", () => {
    const box = { x: 0.1, y: 0.2, width: 0.3, height: 0.4 };
    const rotatedViewportBox = rotateNormalizedRectForViewport(box, 90);

    expect(rotatedViewportBox).toEqual({
      space: "normalized_page",
      x: 0.4,
      y: 0.1,
      width: 0.4,
      height: 0.3,
    });
  });

  it("characterizes search box rotation parity with pdfGeometry helper", () => {
    const box = { x: 0.2, y: 0.3, width: 0.4, height: 0.1 };
    const legacySearchRotated = rotateSearchBox(box, 90);
    const canonicalViewportRotated = rotateNormalizedRectForViewport(box, 90);

    expect(legacySearchRotated.x).toBe(canonicalViewportRotated.x);
    expect(legacySearchRotated.y).toBe(canonicalViewportRotated.y);
    expect(legacySearchRotated.width).toBe(canonicalViewportRotated.width);
    expect(legacySearchRotated.height).toBe(canonicalViewportRotated.height);
  });
});

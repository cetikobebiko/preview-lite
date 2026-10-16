import { describe, expect, it } from "vitest";
import { MAX_SCALE, MIN_SCALE, normalizeScale } from "./zoomLimits.js";

describe("zoomLimits", () => {
  it("keeps the committed scale inside 0.25–4", () => {
    expect([MIN_SCALE, MAX_SCALE]).toEqual([0.25, 4]);
    expect(normalizeScale(0)).toBe(MIN_SCALE);
    expect(normalizeScale(-1)).toBe(MIN_SCALE);
    expect(normalizeScale(5)).toBe(MAX_SCALE);
  });

  it("maps non-finite and non-number input to 1", () => {
    for (const value of [Number.NaN, Infinity, -Infinity, "abc", "2", null, undefined, {}]) {
      expect(normalizeScale(value)).toBe(1);
    }
  });

  it("rounds to two decimals", () => {
    expect(normalizeScale(1 + 0.1 + 0.1)).toBe(1.2);
    expect(normalizeScale(1.1025)).toBe(1.1);
    expect(normalizeScale(2.346)).toBe(2.35);
  });
});

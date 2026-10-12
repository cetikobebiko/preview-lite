import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createVisibilityDwell } from "./visibilityDwell.js";

describe("createVisibilityDwell", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("starts work only after the element stays visible for the dwell time", () => {
    const onDwell = vi.fn();
    const dwell = createVisibilityDwell({ delayMs: 120, onDwell });

    dwell.enter();
    vi.advanceTimersByTime(119);
    expect(onDwell).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onDwell).toHaveBeenCalledOnce();
    expect(dwell.isPending()).toBe(false);
  });

  it("starts nothing for items that only pass through the viewport", () => {
    const onDwells = Array.from({ length: 20 }, () => vi.fn());
    const dwells = onDwells.map((onDwell) =>
      createVisibilityDwell({ delayMs: 120, onDwell }),
    );

    // Each item is visible for 40 ms while the list scrolls past.
    dwells.forEach((dwell) => {
      dwell.enter();
      vi.advanceTimersByTime(40);
      dwell.leave();
    });
    vi.advanceTimersByTime(1_000);
    onDwells.forEach((onDwell) => expect(onDwell).not.toHaveBeenCalled());
  });

  it("does not restart the dwell on repeated visibility notifications", () => {
    const onDwell = vi.fn();
    const dwell = createVisibilityDwell({ delayMs: 120, onDwell });

    dwell.enter();
    vi.advanceTimersByTime(100);
    dwell.enter();
    vi.advanceTimersByTime(20);
    expect(onDwell).toHaveBeenCalledOnce();
  });

  it("cancels a pending dwell on destroy", () => {
    const onDwell = vi.fn();
    const dwell = createVisibilityDwell({ delayMs: 120, onDwell });

    dwell.enter();
    dwell.destroy();
    vi.advanceTimersByTime(1_000);
    expect(onDwell).not.toHaveBeenCalled();
  });

  it("requires a callback", () => {
    expect(() => createVisibilityDwell()).toThrow(TypeError);
  });
});

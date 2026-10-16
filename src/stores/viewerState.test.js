import { describe, expect, it } from "vitest";
import { createViewerState } from "./viewerState.svelte.js";

describe("viewer state", () => {
  it("keeps independent image identity out of document session state", () => {
    const state = createViewerState();
    state.activeImageSourcePath = "/tmp/photo.png";
    expect(state.activeImageSourcePath).toBe("/tmp/photo.png");
    expect(state).not.toHaveProperty("activeSessionId");
    expect(state).not.toHaveProperty("contentGeneration");
  });

  it("derives physical page and book spreads from page presentation state", () => {
    const state = createViewerState();
    state.pageOrder = [2, 0, 1];
    state.currentPageIndex = 1;
    state.isBookMode = true;
    expect(state.physicalPageIndex).toBe(0);
    expect(state.spreads).toEqual([[2], [0, 1]]);
  });

  it("keeps the spreads array until page order or book layout changes", () => {
    const state = createViewerState();
    state.pageOrder = [0, 1, 2];
    const continuous = state.spreads;
    expect(continuous).toEqual([[0], [1], [2]]);
    expect(state.spreads).toBe(continuous);
    state.scale = 2;
    expect(state.spreads).toBe(continuous);
    state.isBookMode = true;
    expect(state.spreads).toEqual([[0], [1, 2]]);
    state.coverMode = false;
    expect(state.spreads).toEqual([[0, 1], [2]]);
    state.pageOrder = [2, 1, 0];
    expect(state.spreads).toEqual([[2, 1], [0]]);
  });
});

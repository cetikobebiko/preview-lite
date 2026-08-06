import { describe, expect, it } from "vitest";
import { createSearchState } from "./searchState.svelte.js";

describe("search state", () => {
  it("resets results without owning document identity", () => {
    const state = createSearchState();
    state.query = "needle";
    state.begin(2);
    expect(state.query).toBe("needle");
    expect(state.results).toEqual([]);
    expect(state).not.toHaveProperty("activeSessionId");
    expect(state).not.toHaveProperty("contentGeneration");
  });

  it("updates active match grouping through one owner", () => {
    const state = createSearchState();
    state.applyResult({ response: {}, normalized: { results: [
      { index: 0, pageIndex: 1, boxes: [] }, { index: 1, pageIndex: 2, boxes: [] },
    ], totalMatches: 2, stats: {} } });
    expect(state.selectMatch(1)?.pageIndex).toBe(2);
    expect(state.currentMatchIndex).toBe(1);
  });
});

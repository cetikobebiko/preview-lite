import { describe, expect, it, vi } from "vitest";
import { createSearchController } from "./searchController.js";
import { createSearchState } from "../../stores/searchState.svelte.js";

describe("search controller", () => {
  it("rejects a result after the document context changes", async () => {
    const state = createSearchState();
    let sessionId = "A";
    const controller = createSearchController({ state, search: async () => ({ results: [], totalMatches: 0 }), getContext: () => ({ mounted: true, isPdf: true, sessionId, contentGeneration: 1 }), getPageOrder: () => [], setPageIndex: vi.fn(), tick: async () => {}, scrollToPage: vi.fn() });
    controller.start(); controller.setQuery("a"); sessionId = "B";
    await Promise.resolve();
    expect(state.results).toEqual([]);
    controller.destroy();
  });

  it("clears the search request in semantic mode", () => {
    const state = createSearchState();
    const controller = createSearchController({ state, search: vi.fn(), getContext: () => ({}), getPageOrder: () => [], setPageIndex: vi.fn(), tick: async () => {}, scrollToPage: vi.fn() });

    controller.handleQueryEvent({ detail: { query: "neural", isSemanticMode: true } });

    expect(state.query).toBe("");
  });
});

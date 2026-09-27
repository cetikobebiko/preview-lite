import { describe, expect, it } from "vitest";
import { createPrepressState } from "./prepressState.svelte.js";

const context = { isLoadedPdfDocument: true, sessionId: "A", contentGeneration: 1, physicalPageIndex: 0 };
const event = { sessionId: "A", contentGeneration: 1, physicalPageIndex: 0, requestId: "request-1", result: { heatmapPngBase64: "heat" }, channels: { c: false, m: true, y: true, k: true } };

describe("prepress state", () => {
  it("resets when the same display path would be reopened under another session", () => {
    const state = createPrepressState();
    state.updateContext(context);
    state.applyCoveragePreview(event, context);
    state.updateContext({ ...context, sessionId: "B" });
    expect(state.coveragePreview).toBeNull();
    expect(state.channels).toEqual({ c: true, m: true, y: true, k: true });
  });

  it("rejects stale session, generation, and physical-page events", () => {
    const state = createPrepressState();
    expect(state.applyCoveragePreview({ ...event, contentGeneration: 2 }, context)).toBe(false);
    expect(state.applyCoveragePreview({ ...event, physicalPageIndex: 2 }, context)).toBe(false);
    expect(state.applyCoveragePreview(event, context)).toBe(true);
  });

  it("rejects a stale clear from an older request in the same page context", () => {
    const state = createPrepressState();
    state.updateContext(context);
    state.applyCoveragePreview(event, context);
    state.applyCoveragePreview({ ...event, requestId: "request-2" }, context);
    expect(state.applyCoveragePreview({ ...event, result: null }, context)).toBe(false);
    expect(state.coveragePreview.requestId).toBe("request-2");
    expect(state.applyCoveragePreview({ ...event, requestId: "request-2", result: null }, context)).toBe(true);
    expect(state.coveragePreview).toBeNull();
  });
});

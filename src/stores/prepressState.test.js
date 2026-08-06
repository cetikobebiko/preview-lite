import { describe, expect, it } from "vitest";
import { createPrepressState } from "./prepressState.svelte.js";

const context = { isLoadedPdfDocument: true, sessionId: "A", contentGeneration: 1, physicalPageIndex: 0 };
const event = { sessionId: "A", contentGeneration: 1, pageIndex: 0, heatmap: "heat", channels: { c: false, m: true, y: true, k: true } };

describe("prepress state", () => {
  it("resets when the same display path would be reopened under another session", () => {
    const state = createPrepressState();
    state.updateContext(context);
    state.applyHeatmap(event, context);
    state.updateContext({ ...context, sessionId: "B" });
    expect(state.heatmap).toBeNull();
    expect(state.channels).toEqual({ c: true, m: true, y: true, k: true });
  });

  it("rejects stale session, generation, and physical-page events", () => {
    const state = createPrepressState();
    expect(state.applyHeatmap({ ...event, contentGeneration: 2 }, context)).toBe(false);
    expect(state.applyHeatmap({ ...event, pageIndex: 2 }, context)).toBe(false);
    expect(state.applyHeatmap(event, context)).toBe(true);
  });
});

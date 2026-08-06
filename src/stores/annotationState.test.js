import { describe, expect, it } from "vitest";
import { createAnnotationState } from "./annotationState.svelte.js";

describe("annotation state", () => {
  it("owns local tool and gesture state without inferring session dirty state", () => {
    const state = createAnnotationState();
    state.activeTool = "highlight";
    state.isDrawing = true;
    expect(state.activeTool).toBe("highlight");
    expect(state.isDrawing).toBe(true);
    expect(state).not.toHaveProperty("dirty");
    expect(state).not.toHaveProperty("activeSessionId");
    expect(state).not.toHaveProperty("redactionMode");
    expect(state).not.toHaveProperty("redactionRects");
  });
});

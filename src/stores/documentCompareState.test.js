import { describe, expect, it, vi } from "vitest";
import { createDocumentCompareState } from "./documentCompareState.svelte.js";

const comparison = {
  currentIsOld: false,
  totalChanges: 1,
  oldWords: 2,
  newWords: 2,
  changes: [{
    kind: "replaced",
    old: { text: "a", textTruncated: false, pageIndex: 0, intrinsicRotation: 0, rects: [], near: null },
    new: { text: "b", textTruncated: false, pageIndex: 0, intrinsicRotation: 0, rects: [{ x: 0.1, y: 0.1, width: 0.1, height: 0.1 }], near: null },
  }],
};

const job = (promise) => ({ jobId: "job", promise, cancel: vi.fn() });

describe("document compare state", () => {
  it("keeps a result only for the document version it was computed for", async () => {
    const state = createDocumentCompareState();
    state.otherPath = "/v1.pdf";
    state.currentIsOld = false;
    const start = vi.fn(() => job(Promise.resolve(comparison)));
    await state.run(start, "session-1", 3);
    expect(start).toHaveBeenCalledWith("session-1", 3, "/v1.pdf", false);
    expect(state.resultFor("session-1", 3)).toEqual(comparison);
    expect(Object.keys(state.highlightsFor("session-1", 3))).toEqual(["0"]);
    expect(state.resultFor("session-1", 4)).toBeNull();
    expect(state.highlightsFor("session-2", 3)).toBeNull();
    expect(state.isComparing).toBe(false);
  });

  it("does not start without a file and reports failures but not cancellations", async () => {
    const state = createDocumentCompareState();
    const start = vi.fn();
    await state.run(start, "session-1", 1);
    expect(start).not.toHaveBeenCalled();

    state.otherPath = "/v1.pdf";
    const failure = Object.assign(new Error("Seçilen PDF açılamadı"), { code: "SESSION_JOB_COMPARE_INPUT_OPEN_FAILED" });
    await state.run(() => job(Promise.reject(failure)), "session-1", 1);
    expect(state.error).toBe("Seçilen PDF açılamadı");

    const cancelled = Object.assign(new Error("iptal"), { code: "SESSION_JOB_CANCELLED" });
    await state.run(() => job(Promise.reject(cancelled)), "session-1", 1);
    expect(state.error).toBe("");
    expect(state.resultFor("session-1", 1)).toBeNull();
  });

  it("ignores a result that arrives after a reset and cancels the running job", async () => {
    const state = createDocumentCompareState();
    state.otherPath = "/v1.pdf";
    let resolve;
    const running = job(new Promise((r) => { resolve = r; }));
    const done = state.run(() => running, "session-1", 1);
    expect(state.isComparing).toBe(true);
    state.reset();
    expect(running.cancel).toHaveBeenCalled();
    resolve(comparison);
    await done;
    expect(state.resultFor("session-1", 1)).toBeNull();
    expect(state.isComparing).toBe(false);
  });
});

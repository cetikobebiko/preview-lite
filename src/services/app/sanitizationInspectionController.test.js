import { describe, expect, it, vi } from "vitest";
import { createSanitizationState } from "../../stores/sanitizationState.svelte.js";
import { createSanitizationInspectionController } from "./sanitizationInspectionController.js";

const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

describe("sanitizationInspectionController", () => {
  it("applies only the current session and generation result", async () => {
    const state = createSanitizationState();
    const context = { isPdf: true, sessionId: "A", contentGeneration: 1 };
    const pending = deferred();
    const startInspection = vi.fn(() => ({ jobId: "job-A", promise: pending.promise, cancel: vi.fn() }));
    const controller = createSanitizationInspectionController({ state, getContext: () => context, startInspection });
    controller.start();
    const result = controller.inspect();
    context.sessionId = "B";
    pending.resolve({ schemaVersion: "report-A" });
    expect(await result).toBeNull();
    expect(state.inspection).toBeNull();
  });

  it("discards generation-stale and superseded results and cancels the prior job", async () => {
    const state = createSanitizationState();
    const context = { isPdf: true, sessionId: "A", contentGeneration: 1 };
    const first = deferred();
    const second = deferred();
    const cancelFirst = vi.fn();
    let calls = 0;
    const startInspection = () => ++calls === 1
      ? { jobId: "job-1", promise: first.promise, cancel: cancelFirst }
      : { jobId: "job-2", promise: second.promise, cancel: vi.fn() };
    const controller = createSanitizationInspectionController({ state, getContext: () => context, startInspection });
    controller.start();
    const result1 = controller.inspect();
    const result2 = controller.inspect();
    expect(cancelFirst).toHaveBeenCalledTimes(1);
    first.resolve({ id: 1 });
    second.resolve({ id: 2 });
    expect(await result1).toBeNull();
    expect(await result2).toEqual({ id: 2 });
    expect(state.inspection).toEqual({ id: 2 });

    const third = deferred();
    const controller2 = createSanitizationInspectionController({
      state: createSanitizationState(), getContext: () => context,
      startInspection: () => ({ jobId: "job-3", promise: third.promise, cancel: vi.fn() }),
    });
    controller2.start();
    const result3 = controller2.inspect();
    context.contentGeneration = 2;
    third.resolve({ id: 3 });
    expect(await result3).toBeNull();
  });

  it("cancels and clears on context reconciliation and destroy", async () => {
    const state = createSanitizationState();
    const context = { isPdf: true, sessionId: "A", contentGeneration: 1 };
    const pending = deferred();
    const cancel = vi.fn();
    const controller = createSanitizationInspectionController({
      state, getContext: () => context,
      startInspection: () => ({ jobId: "job-1", promise: pending.promise, cancel }),
    });
    controller.start();
    void controller.inspect();
    context.isPdf = false;
    controller.reconcileContext();
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(state.inspectionStatus).toBe("idle");
    controller.destroy();
    pending.resolve({ ignored: true });
  });
});

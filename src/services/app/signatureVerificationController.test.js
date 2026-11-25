import { describe, expect, it, vi } from "vitest";
import { createSignatureVerificationController } from "./signatureVerificationController.js";
import { createSignatureVerificationState } from "../../stores/signatureVerificationState.svelte.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function setup() {
  const context = { sessionId: "s1", contentGeneration: 1, isPdf: true };
  const state = createSignatureVerificationState();
  const jobs = [];
  const startInspection = vi.fn((sessionId, generation) => {
    const job = deferred();
    const handle = { jobId: `job-${jobs.length}`, promise: job.promise, cancel: vi.fn() };
    jobs.push({ ...job, handle, sessionId, generation });
    return handle;
  });
  const controller = createSignatureVerificationController({
    state,
    startInspection,
    getContext: () => context,
  });
  return { context, state, jobs, startInspection, controller };
}

const report = (sessionId, contentGeneration) => ({ sessionId, contentGeneration, signatures: [] });

describe("signature verification controller", () => {
  it("stores the report of the current session and generation", async () => {
    const { state, jobs, controller } = setup();
    controller.run();
    expect(state.isRunning).toBe(true);
    jobs[0].resolve(report("s1", 1));
    await Promise.resolve();
    await Promise.resolve();
    expect(state.isRunning).toBe(false);
    expect(state.report).toEqual(report("s1", 1));
    expect(state.isStaleFor("s1", 1)).toBe(false);
    expect(state.isStaleFor("s1", 2)).toBe(true);
  });

  it("drops a result that arrives after the document changed", async () => {
    const { context, state, jobs, controller } = setup();
    controller.run();
    context.contentGeneration = 2;
    jobs[0].resolve(report("s1", 1));
    await Promise.resolve();
    await Promise.resolve();
    expect(state.report).toBeNull();
  });

  it("cancels a running job when the context changes", () => {
    const { context, state, jobs, controller } = setup();
    controller.run();
    context.sessionId = "s2";
    controller.reconcileContext();
    expect(jobs[0].handle.cancel).toHaveBeenCalled();
    expect(state.isRunning).toBe(false);
  });

  it("ignores a superseded request and reports failures", async () => {
    const { state, jobs, controller } = setup();
    controller.run();
    controller.run();
    expect(jobs[0].handle.cancel).toHaveBeenCalled();
    jobs[0].resolve(report("s1", 1));
    jobs[1].reject(Object.assign(new Error("İmza raporu geçersiz"), { code: "SIGNATURE_REPORT_INVALID" }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(state.report).toBeNull();
    expect(state.error).toBe("İmza raporu geçersiz");
  });

  it("treats a cancelled job as no result and refuses without a PDF", async () => {
    const { context, state, jobs, controller } = setup();
    controller.run();
    jobs[0].reject(Object.assign(new Error("iptal"), { code: "SESSION_JOB_CANCELLED" }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(state.error).toBeNull();
    context.isPdf = false;
    controller.run();
    expect(state.error).toMatch(/PDF/);
  });
});

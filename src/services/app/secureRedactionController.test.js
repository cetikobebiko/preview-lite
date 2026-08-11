import { beforeEach, describe, expect, it, vi } from "vitest";
import { createSecureRedactionState } from "../../stores/secureRedactionState.svelte.js";
import {
  createSecureRedactionController,
  mapSecureRedactionExecutionFailure,
} from "./secureRedactionController.js";

const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
};
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const region = () => ({
  id: "region-1",
  pageIndex: 2,
  normalizedTopLeftRect: { x: 0.1, y: 0.2, width: 0.3, height: 0.2 },
  source: "manual_selection",
});
const readyPlan = () => ({
  contentGeneration: 5,
  planDigest: "a".repeat(64),
  digestVerified: true,
  executable: true,
  readyForExecution: true,
  blockers: [],
});

describe("secureRedactionController", () => {
  let context;
  let state;
  let planJobs;
  let executionJobs;
  let startPlan;
  let startExecution;
  let saveFile;
  let controller;

  beforeEach(() => {
    context = { panelOpen: true, isPdf: true, sessionId: "session-1", contentGeneration: 5, fileName: "source.pdf" };
    state = createSecureRedactionState();
    state.addRegion(region());
    planJobs = [];
    executionJobs = [];
    startPlan = vi.fn(() => {
      const job = deferred();
      const handle = { jobId: `plan-${planJobs.length + 1}`, promise: job.promise, cancel: vi.fn() };
      planJobs.push({ ...job, handle });
      return handle;
    });
    startExecution = vi.fn(() => {
      const job = deferred();
      const handle = { jobId: `exec-${executionJobs.length + 1}`, promise: job.promise, cancel: vi.fn() };
      executionJobs.push({ ...job, handle });
      return handle;
    });
    saveFile = vi.fn().mockResolvedValue("/tmp/output.pdf");
    controller = createSecureRedactionController({ state, getContext: () => context, startPlan, startExecution, saveFile });
    controller.start();
  });

  it("plans with validated exact inputs and accepts only the same session/generation result", async () => {
    const handle = controller.plan();
    expect(handle.jobId).toBe("plan-1");
    expect(startPlan).toHaveBeenCalledWith("session-1", 5, state.policy, state.acknowledgements, state.pendingRegions);
    planJobs[0].resolve(readyPlan());
    await flush();
    expect(state.plan.planDigest).toBe("a".repeat(64));
    expect(state.status).toBe("ready");
  });

  it("rejects missing regions and invalid PDF context before creating a job", () => {
    state.clearRegions();
    expect(() => controller.plan()).toThrow(/En az bir/);
    context.isPdf = false;
    expect(() => controller.plan()).toThrow(/PDF oturumu/);
    expect(startPlan).not.toHaveBeenCalled();
  });

  it("cancels a previous planning job and discards stale session, generation, and input results", async () => {
    controller.plan();
    controller.plan();
    expect(planJobs[0].handle.cancel).toHaveBeenCalledTimes(1);
    planJobs[0].resolve(readyPlan());
    await flush();
    expect(state.plan).toBeNull();

    context.sessionId = "session-2";
    controller.reconcileContext();
    expect(planJobs[1].handle.cancel).toHaveBeenCalledTimes(1);
    expect(state.pendingRegions).toEqual([]);

    context = { ...context, sessionId: "session-2", contentGeneration: 6, panelOpen: true, isPdf: true };
    state.addRegion(region());
    controller.plan();
    controller.updatePolicy({ rasterDpi: 400 });
    expect(planJobs[2].handle.cancel).toHaveBeenCalledTimes(1);
    planJobs[2].resolve({ ...readyPlan(), contentGeneration: 6 });
    await flush();
    expect(state.plan).toBeNull();
  });

  it("discards planning result after generation changes and destroy cancels planning", async () => {
    controller.plan();
    context.contentGeneration = 6;
    controller.reconcileContext();
    expect(planJobs[0].handle.cancel).toHaveBeenCalledTimes(1);
    expect(state.status).toBe("idle");

    state.addRegion(region());
    controller.plan();
    controller.destroy();
    expect(planJobs[1].handle.cancel).toHaveBeenCalledTimes(1);
    planJobs[1].resolve(readyPlan());
    await flush();
    expect(state.plan).toBeNull();
  });

  it("opens Save only for a ready plan and calls execution with the immutable exact plan snapshot", async () => {
    state.beginPlanning("prepared", "session-1", 5);
    state.completePlanning("prepared", readyPlan());
    const policy = JSON.parse(JSON.stringify(state.policy));
    const acknowledgements = JSON.parse(JSON.stringify(state.acknowledgements));
    const regions = JSON.parse(JSON.stringify(state.pendingRegions));
    const handle = await controller.execute();
    expect(handle.jobId).toBe("exec-1");
    expect(saveFile).toHaveBeenCalledWith(expect.objectContaining({
      defaultPath: "source-secure-redacted.pdf",
      filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
    }));
    expect(startExecution).toHaveBeenCalledWith(
      "session-1", 5, policy, acknowledgements, regions, "a".repeat(64), "/tmp/output.pdf",
    );
    expect(() => state.updatePolicy({ rasterDpi: 500 })).toThrow(/Execution sürerken/);
  });

  it("does not open Save for a blocked/stale plan and Save cancel starts no execution", async () => {
    state.beginPlanning("blocked", "session-1", 5);
    state.completePlanning("blocked", { ...readyPlan(), blockers: [{ code: "SIGNED" }] });
    await expect(controller.execute()).rejects.toThrow(/hazır değil/);
    expect(saveFile).not.toHaveBeenCalled();

    state.beginPlanning("ready", "session-1", 5);
    state.completePlanning("ready", readyPlan());
    saveFile.mockResolvedValueOnce(null);
    await expect(controller.execute()).resolves.toBeNull();
    expect(startExecution).not.toHaveBeenCalled();
    expect(state.status).toBe("ready");
  });

  it("protects against execution double-click and routes cancellation to the current handle", async () => {
    state.beginPlanning("ready", "session-1", 5);
    state.completePlanning("ready", readyPlan());
    const first = controller.execute();
    const second = controller.execute();
    await Promise.all([first, second]);
    expect(startExecution).toHaveBeenCalledTimes(1);
    expect(controller.cancelExecution()).toBe(true);
    expect(executionJobs[0].handle.cancel).toHaveBeenCalledTimes(1);
  });

  it("preserves post-commit disk truth when cancellation resolves with a published report", async () => {
    state.beginPlanning("ready", "session-1", 5);
    state.completePlanning("ready", readyPlan());
    await controller.execute();
    controller.cancelExecution();
    executionJobs[0].resolve({ published: true, verificationReport: { isVerified: true }, publicationReport: { commitState: "new_committed" } });
    await flush();
    expect(state.status).toBe("completed");
    expect(state.result.cancelArrivedLate).toBe(true);
    expect(state.result.report.published).toBe(true);
  });

  it("shows cancelled only when the authoritative job outcome is cancelled", async () => {
    state.beginPlanning("ready", "session-1", 5);
    state.completePlanning("ready", readyPlan());
    await controller.execute();
    controller.cancelExecution();
    const cancelled = new Error("Session job iptal edildi");
    cancelled.code = "SESSION_JOB_CANCELLED";
    executionJobs[0].reject(cancelled);
    await flush();
    expect(state.status).toBe("cancelled");
    expect(state.result).toBeNull();
  });

  it("cancels/discards active execution on session or generation reconciliation", async () => {
    state.beginPlanning("ready", "session-1", 5);
    state.completePlanning("ready", readyPlan());
    await controller.execute();
    context.contentGeneration = 6;
    controller.reconcileContext();
    expect(executionJobs[0].handle.cancel).toHaveBeenCalledTimes(1);
    executionJobs[0].resolve({ published: true });
    await flush();
    expect(state.result).toBeNull();
    expect(state.status).toBe("idle");
  });

  it("promotes only resolved scan geometry and preserves physical page indices", () => {
    expect(controller.promoteSensitiveScanItem({ geometryStatus: "unresolved" })).toEqual([]);
    const promoted = controller.promoteSensitiveScanItem({
      geometryStatus: "resolved",
      pageIndex: 2,
      pageGeometry: {
        coordinateSpace: "pdf_user_space", unit: "pdf_point", origin: "bottom_left",
        mediaBox: { left: 0, bottom: 0, width: 400, height: 600 }, cropBox: null,
        effectiveBox: { left: 0, bottom: 0, width: 400, height: 600 }, intrinsicRotation: 0,
      },
      sourceRectsPdfPoints: [{ x: 40, y: 480, width: 80, height: 60 }, { x: 200, y: 300, width: 40, height: 30 }],
    });
    expect(promoted).toHaveLength(2);
    expect(promoted.every((item) => item.pageIndex === 2)).toBe(true);
    expect(new Set(promoted.map((item) => item.id)).size).toBe(2);
    expect(promoted[0].normalizedTopLeftRect).toEqual({ x: 0.1, y: 0.1, width: 0.2, height: 0.1 });
  });
});

describe("secure redaction typed publication failure mapping", () => {
  it.each([
    ["recovery_required", "recovery_required"],
    ["indeterminate", "indeterminate"],
  ])("preserves %s commit metadata", (commitState, kind) => {
    const error = new Error("backend failed");
    error.jobOutcome = { error: { code: "PUBLISH_FAILED", commitState, recoveryPath: "/tmp/recovery.pdf", replacementDigest: "b".repeat(64) } };
    expect(mapSecureRedactionExecutionFailure(error)).toEqual(expect.objectContaining({ kind, commitState, recoveryPath: "/tmp/recovery.pdf" }));
  });

  it("distinguishes publication conflict and verification failure", () => {
    const conflict = new Error("conflict");
    conflict.code = "REDACTION_TARGET_CONFLICT";
    expect(mapSecureRedactionExecutionFailure(conflict).kind).toBe("publication_conflict");
    const verify = new Error("verify");
    verify.code = "REDACTION_VERIFICATION_FAILED";
    expect(mapSecureRedactionExecutionFailure(verify).kind).toBe("verification_failed");
  });
});

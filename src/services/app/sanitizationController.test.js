import { describe, expect, it, vi } from "vitest";
import { SANITIZATION_MINIMUM_PROFILE_CONTROL_IDS } from "../../features/sanitization/sanitizationSchemas.js";
import { createSanitizationState } from "../../stores/sanitizationState.svelte.js";
import { createSanitizationController } from "./sanitizationController.js";

const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((ok, fail) => { resolve = ok; reject = fail; });
  return { promise, resolve, reject };
};

const inspection = (overrides = {}) => ({
  inspectionVersion: "sanitization-inspection-v1",
  policySchemaVersion: "sanitization-policy-v1",
  controlRegistryVersion: "sanitization-controls/1",
  documentFacts: { signed: false, ...overrides.documentFacts },
  blockingFindings: overrides.blockingFindings ?? [],
});

function applyInspection(state, value = inspection(), sessionId = "session-a", generation = 1) {
  const revision = state.beginInspection({ jobId: "inspection-setup", sessionId, contentGeneration: generation });
  state.applyInspection({
    jobId: "inspection-setup", sessionId, contentGeneration: generation, revision, inspection: value,
  });
}

function setup(options = {}) {
  const state = createSanitizationState();
  const context = options.context ?? {
    isPdf: true, sessionId: "session-a", contentGeneration: 1, fileName: "source.pdf",
  };
  const startInspection = options.startInspection ?? vi.fn(() => ({
    jobId: "inspect-1", promise: Promise.resolve(inspection()), cancel: vi.fn(),
  }));
  const startExecution = options.startExecution ?? vi.fn(() => ({
    jobId: "execute-1", promise: Promise.resolve({ assurance: "verified_limited" }), cancel: vi.fn(),
  }));
  const saveFile = options.saveFile ?? vi.fn(async () => "/exports/source.sanitized-limited.pdf");
  const controller = createSanitizationController({
    state,
    getContext: () => context,
    startInspection,
    startExecution,
    saveFile,
  });
  controller.start();
  return { state, context, controller, startInspection, startExecution, saveFile };
}

const flush = async () => { await Promise.resolve(); await Promise.resolve(); };

describe("sanitizationController", () => {
  it("uses the canonical inspection wrapper and applies only the owned result", async () => {
    const { controller, state, startInspection } = setup();
    await controller.inspect();
    await flush();
    expect(startInspection).toHaveBeenCalledWith("session-a", 1);
    expect(state.inspectionStatus).toBe("review");
  });

  it("captures an immutable versioned request and calls the canonical execution wrapper after Save", async () => {
    const execution = deferred();
    const startExecution = vi.fn(() => ({ jobId: "execute-1", promise: execution.promise, cancel: vi.fn() }));
    const { controller, state, saveFile } = setup({ startExecution });
    applyInspection(state);
    const promise = controller.execute();
    await flush();
    const handle = await promise;
    expect(saveFile).toHaveBeenCalledWith(expect.objectContaining({
      defaultPath: "source.sanitized-limited.pdf",
      filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
    }));
    const [sessionId, generation, policy, targetPath] = startExecution.mock.calls[0];
    expect([sessionId, generation, targetPath]).toEqual(["session-a", 1, "/exports/source.sanitized-limited.pdf"]);
    expect(policy).toEqual({
      schemaVersion: "sanitization-policy-v1",
      profile: "verified_limited_v1",
      controls: SANITIZATION_MINIMUM_PROFILE_CONTROL_IDS,
      acknowledgements: { acknowledgeSignatureInvalidation: false },
      expectedInspectionVersion: "sanitization-inspection-v1",
    });
    expect(Object.isFrozen(state.executionRequest)).toBe(true);
    expect(state.executionRequest.workflowRevision).toBe(state.workflowRevision);
    expect(handle.jobId).toBe("execute-1");
  });

  it("blocks stale inspection, missing acknowledgement, and unsupported blockers before Save", async () => {
    const stale = setup();
    applyInspection(stale.state, inspection(), "session-a", 0);
    await expect(stale.controller.execute()).rejects.toThrow(/eşleşmiyor/);
    expect(stale.saveFile).not.toHaveBeenCalled();

    const signed = setup();
    applyInspection(signed.state, inspection({ documentFacts: { signed: true } }));
    await expect(signed.controller.execute()).rejects.toThrow(/acknowledgement/);
    expect(signed.saveFile).not.toHaveBeenCalled();

    const blocked = setup();
    applyInspection(blocked.state, inspection({
      blockingFindings: [{ controlId: "encryption", code: "BLOCK", safeMessage: "Encrypted input unsupported" }],
    }));
    await expect(blocked.controller.execute()).rejects.toThrow(/blocking unsupported/);
    expect(blocked.saveFile).not.toHaveBeenCalled();
  });

  it("does not start execution when Save is cancelled and collapses double execute to one job", async () => {
    const cancelled = setup({ saveFile: vi.fn(async () => null) });
    applyInspection(cancelled.state);
    expect(await cancelled.controller.execute()).toBeNull();
    expect(cancelled.startExecution).not.toHaveBeenCalled();

    const save = deferred();
    const double = setup({ saveFile: vi.fn(() => save.promise) });
    applyInspection(double.state);
    const first = double.controller.execute();
    const second = double.controller.execute();
    expect(await second).toBeNull();
    save.resolve("/exports/once.pdf");
    await first;
    expect(double.startExecution).toHaveBeenCalledTimes(1);
  });

  it("cancels inspection and execution on session/generation reconciliation", async () => {
    const inspectionJob = deferred();
    const inspectionCancel = vi.fn();
    const first = setup({
      startInspection: vi.fn(() => ({ jobId: "inspect-live", promise: inspectionJob.promise, cancel: inspectionCancel })),
    });
    await first.controller.inspect();
    first.context.sessionId = "session-b";
    first.controller.reconcileContext();
    expect(inspectionCancel).toHaveBeenCalledTimes(1);
    expect(first.state.workflowStatus).toBe("idle");

    const executionJob = deferred();
    const executionCancel = vi.fn();
    const second = setup({
      startExecution: vi.fn(() => ({ jobId: "execute-live", promise: executionJob.promise, cancel: executionCancel })),
    });
    applyInspection(second.state);
    await second.controller.execute();
    second.context.contentGeneration = 2;
    second.controller.reconcileContext();
    expect(executionCancel).toHaveBeenCalledTimes(1);
    expect(second.state.workflowStatus).toBe("idle");
  });

  it("discards stale session/generation results and preserves post-commit success after cancel", async () => {
    const staleJob = deferred();
    const stale = setup({
      startExecution: vi.fn(() => ({ jobId: "execute-stale", promise: staleJob.promise, cancel: vi.fn() })),
    });
    applyInspection(stale.state);
    await stale.controller.execute();
    stale.context.sessionId = "session-b";
    stale.controller.reconcileContext();
    staleJob.resolve({ assurance: "verified_limited" });
    await flush();
    expect(stale.state.executionResult).toBeNull();

    const committedJob = deferred();
    const committedReport = {
      assurance: "verified_limited",
      publication: { published: true, commitState: "new_committed" },
    };
    const committed = setup({
      startExecution: vi.fn(() => ({ jobId: "execute-committed", promise: committedJob.promise, cancel: vi.fn() })),
    });
    applyInspection(committed.state);
    await committed.controller.execute();
    expect(committed.controller.cancelExecution()).toBe(true);
    committedJob.resolve(committedReport);
    await flush();
    expect(committed.state.executionStatus).toBe("completed");
    expect(committed.state.executionResult).toEqual(committedReport);
  });
});

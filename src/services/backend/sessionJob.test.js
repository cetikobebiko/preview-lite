import { beforeEach, describe, expect, it, vi } from "vitest";
import { runSessionJob } from "./sessionJob.js";

const JOB_ID = "00000000-0000-4000-8000-000000000001";
const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

beforeEach(() => {
  invokeMock.mockReset();
  vi.stubGlobal("crypto", {
    ...globalThis.crypto,
    randomUUID: vi.fn(() => JOB_ID),
  });
});

describe("runSessionJob", () => {
  it("returns the exact handle and unwraps a completed outcome", async () => {
    const invokeOperation = vi.fn(async () => ({
      status: "completed",
      jobId: JOB_ID,
      sessionId: "session-1",
      contentGeneration: 4,
      value: ["ok"],
      cancelRequested: false,
      cancellationReason: null,
      cleanup: { status: "cleaned" },
    }));

    const handle = runSessionJob("session-1", invokeOperation);

    expect(handle.jobId).toBe(JOB_ID);
    expect(handle.cancel).toEqual(expect.any(Function));
    await expect(handle.promise).resolves.toEqual(["ok"]);
    expect(invokeOperation).toHaveBeenCalledWith(JOB_ID);
  });

  it("can deliver cancel before the deferred operation invoke", async () => {
    const order = [];
    invokeMock.mockImplementation(async (command) => {
      order.push(command);
      return { status: "cancel_pending" };
    });
    const handle = runSessionJob("session-1", async () => {
      order.push("operation");
      return {
        status: "cancelled",
        jobId: JOB_ID,
        sessionId: "session-1",
        contentGeneration: 4,
        cancellation: "user_requested",
        cleanup: { status: "cleaned" },
      };
    });

    await expect(handle.cancel()).resolves.toEqual({ status: "cancel_pending" });
    await expect(handle.promise).rejects.toMatchObject({
      code: "SESSION_JOB_CANCELLED",
    });
    expect(order).toEqual(["cancel_session_job", "operation"]);
    expect(invokeMock).toHaveBeenCalledWith("cancel_session_job", {
      sessionId: "session-1",
      jobId: JOB_ID,
    });
  });

  it("preserves a failed outcome and its cleanup warning", async () => {
    const handle = runSessionJob("session-1", async () => ({
      status: "failed",
      jobId: JOB_ID,
      sessionId: "session-1",
      contentGeneration: 4,
      error: { code: "OCR_FAILED", message: "OCR failed" },
      cleanup: {
        status: "retry_scheduled",
        attempt: 1,
        retryAtMs: 10,
        warning: {
          code: "SESSION_JOB_CLEANUP_RETRY_SCHEDULED",
          message: "Cleanup retry",
        },
      },
    }));

    await expect(handle.promise).rejects.toMatchObject({
      code: "OCR_FAILED",
      jobOutcome: {
        status: "failed",
        cleanup: { status: "retry_scheduled" },
      },
    });
  });
});

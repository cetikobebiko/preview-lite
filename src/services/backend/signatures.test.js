import { describe, expect, it, vi } from "vitest";
import { rawReport } from "../../features/signature/signatureReportTestData.js";

const callBackend = vi.hoisted(() => vi.fn());
const cancel = vi.hoisted(() => vi.fn());
const runSessionJob = vi.hoisted(() =>
  vi.fn((_sessionId, invokeOperation) => ({ jobId: "job-1", promise: invokeOperation("job-1"), cancel })),
);

vi.mock("./client.js", () => ({ callBackend }));
vi.mock("./sessionJob.js", () => ({ runSessionJob }));

import { startSignatureInspection } from "./signatures.js";
import { CONTRACTS, PROTECTED_CONTRACT_IDS } from "./contracts.js";

describe("startSignatureInspection", () => {
  it("runs the read-only session job and normalizes the report", async () => {
    callBackend.mockResolvedValueOnce(rawReport());
    const handle = startSignatureInspection("session-1", 3);
    expect(handle.jobId).toBe("job-1");
    expect(handle.cancel).toBe(cancel);
    const report = await handle.promise;
    expect(callBackend).toHaveBeenCalledWith("inspectSessionSignatures", {
      jobId: "job-1",
      sessionId: "session-1",
      expectedContentGeneration: 3,
    });
    expect(report.signatures[0].integrity).toBe("valid");
  });

  it("rejects a malformed report instead of showing it", async () => {
    callBackend.mockResolvedValueOnce({ schema_version: 1 });
    await expect(startSignatureInspection("session-1", 3).promise).rejects.toThrow(/geçersiz/);
  });

  it("is a protected contract with only job identity in its payload", () => {
    expect(CONTRACTS.inspectSessionSignatures.command).toBe("inspect_session_signatures");
    expect(PROTECTED_CONTRACT_IDS).toContain("inspectSessionSignatures");
  });
});

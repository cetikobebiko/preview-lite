import { beforeEach, describe, expect, it, vi } from "vitest";
import { exportSessionPagesAsImages } from "../backend/session.js";
import { splitSessionPdf } from "../backend/manipulation.js";
import {
  OUTPUT_CONFLICT_CODE,
  outputConflictMessage,
  outputConflictNames,
  publishWithOverwriteConfirmation,
} from "./outputConflict.js";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const outcomeBase = {
  jobId: "job",
  sessionId: "session-1",
  contentGeneration: 7,
  cleanup: { status: "cleaned" },
};
const conflictOutcome = (names) => ({
  ...outcomeBase,
  status: "failed",
  error: {
    code: OUTPUT_CONFLICT_CODE,
    message: `Hedef klasörde aynı adlı ${names.length} dosya zaten var`,
    commitState: null,
    recoveryPath: null,
    replacementDigest: null,
    conflictingNames: names,
  },
});
const completedOutcome = (value) => ({
  ...outcomeBase,
  status: "completed",
  value,
  cancelRequested: false,
  cancellationReason: null,
});

const payloadOf = (call) => call[1];

describe("output conflict handling", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("publishes directly into an empty folder without asking", async () => {
    invokeMock.mockResolvedValueOnce(completedOutcome(["/out/sayfa_1.png"]));
    const confirmOverwrite = vi.fn();

    const result = await publishWithOverwriteConfirmation(
      (overwrite) => exportSessionPagesAsImages("session-1", 7, [0], "/out", "png", 300, overwrite),
      confirmOverwrite,
    );

    expect(result).toEqual({ status: "completed", value: ["/out/sayfa_1.png"] });
    expect(confirmOverwrite).not.toHaveBeenCalled();
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(payloadOf(invokeMock.mock.calls[0])).toMatchObject({ outputDir: "/out", overwrite: false });
  });

  it("asks with the conflicting names and retries with overwrite after confirmation", async () => {
    invokeMock
      .mockResolvedValueOnce(conflictOutcome(["split_0_0.pdf"]))
      .mockResolvedValueOnce(completedOutcome(["/out/split_0_0.pdf"]));
    const confirmOverwrite = vi.fn().mockResolvedValue(true);

    const result = await publishWithOverwriteConfirmation(
      (overwrite) => splitSessionPdf("session-1", 7, [[0, 1]], "/out", overwrite),
      confirmOverwrite,
    );

    expect(confirmOverwrite).toHaveBeenCalledWith(["split_0_0.pdf"]);
    expect(result).toEqual({ status: "completed", value: ["/out/split_0_0.pdf"] });
    expect(invokeMock.mock.calls.map(([command, payload]) => [command, payload.overwrite])).toEqual([
      ["split_session_pdf", false],
      ["split_session_pdf", true],
    ]);
  });

  it("does not retry when the user declines the overwrite", async () => {
    invokeMock.mockResolvedValueOnce(conflictOutcome(["sayfa_1.png", "sayfa_2.png"]));

    const result = await publishWithOverwriteConfirmation(
      (overwrite) => exportSessionPagesAsImages("session-1", 7, [0, 1], "/out", "png", 300, overwrite),
      async () => false,
    );

    expect(result).toEqual({ status: "declined", names: ["sayfa_1.png", "sayfa_2.png"] });
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it("rethrows failures that are not output conflicts without asking", async () => {
    invokeMock.mockResolvedValueOnce({
      ...outcomeBase,
      status: "failed",
      error: { code: "SESSION_JOB_OUTPUT_WRITE_FAILED", message: "disk dolu" },
    });
    const confirmOverwrite = vi.fn();

    await expect(
      publishWithOverwriteConfirmation(
        (overwrite) => splitSessionPdf("session-1", 7, [[0, 1]], "/out", overwrite),
        confirmOverwrite,
      ),
    ).rejects.toMatchObject({ code: "SESSION_JOB_OUTPUT_WRITE_FAILED" });
    expect(confirmOverwrite).not.toHaveBeenCalled();
  });

  it("extracts names only from conflict errors", () => {
    expect(outputConflictNames(null)).toBeNull();
    expect(outputConflictNames({ code: "OTHER" })).toBeNull();
    expect(outputConflictNames({ code: OUTPUT_CONFLICT_CODE })).toEqual([]);
    expect(
      outputConflictNames({
        code: OUTPUT_CONFLICT_CODE,
        jobOutcome: { error: { conflictingNames: ["a.png"] } },
      }),
    ).toEqual(["a.png"]);
  });

  it("lists a bounded number of names in the confirmation message", () => {
    const names = Array.from({ length: 10 }, (_, index) => `sayfa_${index + 1}.png`);
    const message = outputConflictMessage(names);

    expect(message).toContain("aynı adlı 10 dosya");
    expect(message).toContain("• sayfa_8.png");
    expect(message).not.toContain("sayfa_9.png");
    expect(message).toContain("… ve 2 dosya daha");
  });
});

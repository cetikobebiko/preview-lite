import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import { getBatchStatus, startBatch } from "./batch.js";
import { CONTRACTS, PROTECTED_CONTRACT_IDS } from "./contracts.js";

const request = (operation, extra = {}) => ({
  inputs: ["/a.pdf"],
  operation,
  outputDir: "/çıktı",
  conflictPolicy: "number",
  ...extra,
});

describe("batch contracts", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ batchId: "b1" });
  });

  it("replaces the string-JSON add_batch_job contract", () => {
    expect(CONTRACTS.addBatchJob).toBeUndefined();
    expect(Object.values(CONTRACTS).some((contract) => contract.command === "add_batch_job")).toBe(false);
    expect(PROTECTED_CONTRACT_IDS).not.toContain("addBatchJob");
    for (const id of ["scanBatchInputs", "startBatch", "cancelBatch", "revealBatchOutputDir"]) {
      expect(PROTECTED_CONTRACT_IDS).toContain(id);
    }
  });

  it("sends typed operations", async () => {
    await startBatch(request({ kind: "export_images", format: "jpeg", dpi: 150 }));
    expect(invokeMock).toHaveBeenCalledWith("start_batch", {
      request: request({ kind: "export_images", format: "jpeg", dpi: 150 }),
    });
    await getBatchStatus();
    expect(invokeMock).toHaveBeenLastCalledWith("get_batch_status", { batchId: null });
  });

  it.each([
    ["missing settings", { kind: "optimize" }],
    ["fields of another operation", { kind: "export_text", format: "txt", dpi: 144 }],
    ["an unknown format", { kind: "export_document", format: "pdf" }],
    ["an unknown kind", { kind: "ocr" }],
    ["string parameters", { kind: "export_text", format: "txt", parameters: "{}" }],
  ])("refuses %s before invoking", async (_label, operation) => {
    await expect(startBatch(request(operation))).rejects.toThrow();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("refuses more than 500 inputs and an unknown conflict policy", async () => {
    const inputs = Array.from({ length: 501 }, (_, index) => `/${index}.pdf`);
    await expect(startBatch(request({ kind: "export_text", format: "txt" }, { inputs }))).rejects.toThrow(/at most 500/);
    await expect(startBatch(request({ kind: "export_text", format: "txt" }, { conflictPolicy: "replace" }))).rejects.toThrow();
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

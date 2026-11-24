import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import {
  BATCH_PROGRESS_EVENT,
  BATCH_SIGNATURE_LOSS_TEXT,
  batchReportText,
  buildBatchOperation,
  confirmRunningBatchStop,
  createBatchTracker,
  runningBatchId,
  signatureLossCount,
} from "./batchController.js";

const file = (index, state = "pending", extra = {}) => ({
  index, inputPath: `/girdi/${index}.pdf`, state, outputs: [], reason: null, ...extra,
});
const counts = (completed = 0, skipped = 0, failed = 0, cancelled = 0) => ({ completed, skipped, failed, cancelled });
const status = (extra = {}) => ({
  batchId: "b1", state: "running", operation: "optimize", outputDir: "/çıktı", total: 2,
  lastSequence: 0, counts: counts(), files: [file(0), file(1)], warnings: [], reason: null, ...extra,
});
const event = (sequence, extra = {}) => ({
  batchId: "b1", sequence, kind: "file_started", terminal: false, total: 2, counts: counts(),
  file: null, status: null, ...extra,
});

function setup({ initial = status() } = {}) {
  let handler = null;
  const unlisten = vi.fn();
  const listen = vi.fn(async (name, callback) => { handler = callback; return unlisten; });
  const getBatchStatus = vi.fn(async () => initial);
  const onChange = vi.fn();
  const tracker = createBatchTracker({ listen, getBatchStatus, onChange });
  return { tracker, listen, getBatchStatus, onChange, unlisten, emit: (payload) => handler({ payload }) };
}

describe("batch tracker", () => {
  it("subscribes before reading the status and applies only later events in order", async () => {
    const harness = setup({ initial: status({ lastSequence: 2 }) });
    const order = [];
    harness.listen.mockImplementation(async (name, callback) => { order.push(name); harness.emitTo = callback; return harness.unlisten; });
    harness.getBatchStatus.mockImplementation(async () => { order.push("status"); return status({ lastSequence: 2 }); });
    await harness.tracker.attach("b1");
    expect(order).toEqual([BATCH_PROGRESS_EVENT, "status"]);

    harness.emitTo({ payload: event(2, { file: file(0, "running") }) });
    expect(harness.tracker.current().files[0].state).toBe("pending");
    harness.emitTo({ payload: event(3, { kind: "file_completed", file: file(0, "completed", { outputs: ["/çıktı/0_küçültülmüş.pdf"] }), counts: counts(1) }) });
    expect(harness.tracker.current().files[0].state).toBe("completed");
    expect(harness.tracker.current().counts.completed).toBe(1);
    expect(harness.tracker.current().lastSequence).toBe(3);
  });

  it("ignores other batches and takes the terminal status as final", async () => {
    const harness = setup();
    await harness.tracker.attach("b1");
    harness.emit(event(1, { batchId: "other", file: file(0, "failed") }));
    expect(harness.tracker.current().files[0].state).toBe("pending");
    const final = status({ state: "cancelled", lastSequence: 1, files: [file(0, "cancelled"), file(1, "cancelled")], counts: counts(0, 0, 0, 2) });
    harness.emit(event(1, { kind: "batch_cancelled", terminal: true, status: final }));
    expect(harness.tracker.current()).toEqual(final);
    expect(harness.onChange).toHaveBeenLastCalledWith(final);
  });

  it("re-reads the status after a sequence gap", async () => {
    const harness = setup();
    await harness.tracker.attach("b1");
    const fresh = status({ lastSequence: 5, counts: counts(1, 1) });
    harness.getBatchStatus.mockResolvedValueOnce(fresh);
    harness.emit(event(4));
    await vi.waitFor(() => expect(harness.tracker.current()).toEqual(fresh));
    expect(harness.getBatchStatus).toHaveBeenCalledTimes(2);
  });

  it("adopts the current batch when reopened and removes its listener on dispose", async () => {
    const harness = setup({ initial: status({ batchId: "b9" }) });
    const adopted = await harness.tracker.attach(null);
    expect(adopted.batchId).toBe("b9");
    expect(harness.getBatchStatus).toHaveBeenCalledWith(null);
    harness.tracker.dispose();
    expect(harness.unlisten).toHaveBeenCalledOnce();
    harness.emit(event(1, { batchId: "b9", file: file(0, "running") }));
    expect(harness.tracker.current().files[0].state).toBe("pending");
  });

  it("drops events that arrive while the status is being read, if they are already in it", async () => {
    let resolveStatus;
    const harness = setup();
    harness.getBatchStatus.mockImplementation(() => new Promise((resolve) => { resolveStatus = resolve; }));
    const attached = harness.tracker.attach("b1");
    await vi.waitFor(() => expect(resolveStatus).toBeTypeOf("function"));
    harness.emit(event(1, { file: file(0, "running") }));
    harness.emit(event(2, { kind: "file_completed", file: file(0, "completed"), counts: counts(1) }));
    resolveStatus(status({ lastSequence: 1, files: [file(0, "running"), file(1)] }));
    await attached;
    expect(harness.tracker.current().lastSequence).toBe(2);
    expect(harness.tracker.current().files[0].state).toBe("completed");
  });
});

describe("closing with a running batch", () => {
  it("does not ask without a running batch", async () => {
    const ask = vi.fn();
    const result = await confirmRunningBatchStop({ getBatchStatus: async () => status({ state: "completed" }), ask });
    expect(result).toBe(true);
    expect(ask).not.toHaveBeenCalled();
  });

  it("cancels the batch when the user agrees and keeps the window otherwise", async () => {
    const cancelBatch = vi.fn(async () => true);
    expect(await confirmRunningBatchStop({ getBatchStatus: async () => status(), ask: async () => true, cancelBatch })).toBe(true);
    expect(cancelBatch).toHaveBeenCalledWith("b1");
    cancelBatch.mockClear();
    expect(await confirmRunningBatchStop({ getBatchStatus: async () => status(), ask: async () => false, cancelBatch })).toBe(false);
    expect(cancelBatch).not.toHaveBeenCalled();
  });

  it("treats an unreadable status as no running batch", async () => {
    expect(await runningBatchId(async () => { throw new Error("x"); })).toBeNull();
  });
});

describe("batch requests and reports", () => {
  it("builds the typed operation for each wizard choice", () => {
    expect(buildBatchOperation("export_images", { imageFormat: "png", dpi: "144" })).toEqual({ kind: "export_images", format: "png", dpi: 144 });
    expect(buildBatchOperation("export_text", { textFormat: "md" })).toEqual({ kind: "export_text", format: "md" });
    expect(buildBatchOperation("security", { securityOp: { op: "remove" } })).toEqual({ kind: "security", op: { op: "remove" } });
    expect(() => buildBatchOperation("ocr", {})).toThrow(TypeError);
  });

  it("writes a report with reasons and outputs", () => {
    const text = batchReportText(status({
      state: "completed",
      counts: counts(1, 1),
      files: [
        file(0, "completed", { outputs: ["/çıktı/0_küçültülmüş.pdf"] }),
        file(1, "skipped", { reason: { code: "BATCH_INPUT_PASSWORD_REQUIRED", message: "PDF parola korumalı" } }),
      ],
    }), "Dosya boyutunu küçült");
    expect(text).toContain("Toplu işlem: Dosya boyutunu küçült");
    expect(text).toContain("Tamamlandı: 1, atlandı: 1, hata: 0, iptal: 0");
    expect(text).toContain("  → /çıktı/0_küçültülmüş.pdf");
    expect(text).toContain("Atlandı: /girdi/1.pdf — PDF parola korumalı (BATCH_INPUT_PASSWORD_REQUIRED)");
    expect(text).not.toContain("Dijital imza");
  });

  // Mirrors batch::tests::signature_loss_is_reported_per_file_for_pdf_copies_only:
  // sanitization/signature.pdf → signaturesInvalidated true, minimal.pdf → false.
  it("names the files whose signatures are not valid in the output", () => {
    const signedStatus = status({
      state: "completed",
      counts: counts(2),
      files: [
        file(0, "completed", { inputPath: "/girdi/imzali.pdf", outputs: ["/çıktı/imzali_korumalı.pdf"], signaturesInvalidated: true }),
        file(1, "completed", { inputPath: "/girdi/imzasiz.pdf", outputs: ["/çıktı/imzasiz_korumalı.pdf"], signaturesInvalidated: false }),
      ],
    });
    expect(signatureLossCount(signedStatus)).toBe(1);
    const lines = batchReportText(signedStatus, "Parola ile koru").split("\n");
    expect(lines).toContain("Dijital imzaları çıktıda geçersiz olan dosya: 1");
    const signedLine = lines.indexOf("Tamamlandı: /girdi/imzali.pdf");
    expect(lines[signedLine + 1]).toBe(`  ${BATCH_SIGNATURE_LOSS_TEXT}`);
    const unsignedLine = lines.indexOf("Tamamlandı: /girdi/imzasiz.pdf");
    expect(lines[unsignedLine + 1]).toBe("  → /çıktı/imzasiz_korumalı.pdf");
  });
});

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createSensitiveDataScanState } from "../../stores/sensitiveDataScanState.svelte.js";
import { createSensitiveDataScanController } from "./sensitiveDataScanController.js";

describe("sensitiveDataScanController", () => {
  let state;
  let startScanMock;
  let getContextMock;
  let controller;

  beforeEach(() => {
    state = createSensitiveDataScanState();
    startScanMock = vi.fn();
    getContextMock = vi.fn().mockReturnValue({
      panelOpen: true,
      isPdf: true,
      sessionId: "session-1",
      contentGeneration: 1,
    });
    controller = createSensitiveDataScanController({
      state,
      startScan: startScanMock,
      getContext: getContextMock,
    });
  });

  it("does not scan when not mounted", () => {
    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);
    expect(startScanMock).not.toHaveBeenCalled();
  });

  it("sends regex and built-in patterns as typed specs in panel order", () => {
    controller.start();
    getContextMock.mockReturnValue({ panelOpen: true, isPdf: true, sessionId: "s1", contentGeneration: 2 });
    startScanMock.mockReturnValue({ jobId: "job-1", promise: new Promise(() => {}), cancel: vi.fn() });
    controller.scan([
      { label: "TC", regex: "\\d{11}", active: true },
      { label: "Kart", builtin: "credit-card", active: true },
      { label: "Tarih", builtin: "date", active: false },
    ]);
    expect(startScanMock).toHaveBeenCalledWith("s1", 2, [
      { kind: "regex", value: "\\d{11}" },
      { kind: "builtin", value: "credit-card" },
    ]);
    expect(state.capturedPatternLabels).toEqual(["TC", "Kart"]);
  });

  it("fails scan when zero active patterns are provided and cancels active job", () => {
    controller.start();
    const cancelMock = vi.fn();
    startScanMock.mockReturnValue({
      jobId: "job-1",
      promise: new Promise(() => {}),
      cancel: cancelMock,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);
    expect(state.isScanning).toBe(true);

    // Call scan with zero active patterns
    controller.scan([{ label: "TC", regex: "\\d{11}", active: false }]);
    expect(cancelMock).toHaveBeenCalled();
    expect(state.isScanning).toBe(false);
    expect(state.error).toBe("En az bir aktif kalıp seçilmelidir.");
  });

  it("executes scan and completes state on successful resolution", async () => {
    controller.start();
    let resolvePromise;
    const promise = new Promise((res) => {
      resolvePromise = res;
    });
    const cancelMock = vi.fn();
    startScanMock.mockReturnValue({
      jobId: "job-100",
      promise,
      cancel: cancelMock,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    expect(state.isScanning).toBe(true);
    expect(state.capturedPatternLabels).toEqual(["TC"]);

    const mockReport = {
      schemaVersion: 2,
      truncated: false,
      warnings: [],
      items: [],
    };
    resolvePromise(mockReport);
    await promise;

    expect(state.isScanning).toBe(false);
    expect(state.report).toEqual(mockReport);
    expect(state.error).toBeNull();
  });

  it("cancels active job and resets state when generation changes (N -> N+1)", async () => {
    controller.start();
    let resolvePromise;
    const promise = new Promise((res) => { resolvePromise = res; });
    const cancelMock = vi.fn();
    startScanMock.mockReturnValue({
      jobId: "job-gen-1",
      promise,
      cancel: cancelMock,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    // Generation N -> N+1
    getContextMock.mockReturnValue({
      panelOpen: true,
      isPdf: true,
      sessionId: "session-1",
      contentGeneration: 2,
    });

    controller.reconcileContext();

    expect(cancelMock).toHaveBeenCalled();
    expect(state.isScanning).toBe(false);
    expect(state.report).toBeNull();

    resolvePromise({ schemaVersion: 2, items: [] });
    await promise;
    expect(state.report).toBeNull();
  });

  it("cancels active job and resets state when panel is closed", async () => {
    controller.start();
    const cancelMock = vi.fn();
    const promise = new Promise(() => {});
    startScanMock.mockReturnValue({
      jobId: "job-panel",
      promise,
      cancel: cancelMock,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    getContextMock.mockReturnValue({
      panelOpen: false,
      isPdf: true,
      sessionId: "session-1",
      contentGeneration: 1,
    });

    controller.reconcileContext();
    expect(cancelMock).toHaveBeenCalled();
    expect(state.isScanning).toBe(false);
  });

  it("cancels active job and resets state when PDF transitions to image", async () => {
    controller.start();
    const cancelMock = vi.fn();
    const promise = new Promise(() => {});
    startScanMock.mockReturnValue({
      jobId: "job-img",
      promise,
      cancel: cancelMock,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    getContextMock.mockReturnValue({
      panelOpen: true,
      isPdf: false,
      sessionId: "session-1",
      contentGeneration: 1,
    });

    controller.reconcileContext();
    expect(cancelMock).toHaveBeenCalled();
    expect(state.isScanning).toBe(false);
  });

  it("cancels pending job during destroy and drops stale resolve/reject", async () => {
    controller.start();
    let rejectPromise;
    const promise = new Promise((_, rej) => { rejectPromise = rej; });
    const cancelMock = vi.fn();
    startScanMock.mockReturnValue({
      jobId: "job-destroy",
      promise,
      cancel: cancelMock,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    controller.destroy();
    expect(cancelMock).toHaveBeenCalled();

    rejectPromise(new Error("Late error"));
    await promise.catch(() => {});
    expect(state.error).toBeNull();
  });

  it("does NOT show state.error when error code is SESSION_JOB_CANCELLED", async () => {
    controller.start();
    let rejectPromise;
    const promise = new Promise((_, rej) => { rejectPromise = rej; });
    startScanMock.mockReturnValue({
      jobId: "job-cancelled",
      promise,
      cancel: vi.fn(),
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    const cancelledError = new Error("Session job cancelled");
    cancelledError.code = "SESSION_JOB_CANCELLED";
    rejectPromise(cancelledError);

    await promise.catch(() => {});

    expect(state.isScanning).toBe(false);
    expect(state.error).toBeNull();
  });

  it("sets state.error on genuine backend failure", async () => {
    controller.start();
    let rejectPromise;
    const promise = new Promise((_, rej) => { rejectPromise = rej; });
    startScanMock.mockReturnValue({
      jobId: "job-err",
      promise,
      cancel: vi.fn(),
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    rejectPromise(new Error("Worker IPC connection failed"));
    await promise.catch(() => {});

    expect(state.isScanning).toBe(false);
    expect(state.error).toBe("Worker IPC connection failed");
  });

  it("cancels active job when scan is called with invalid context", () => {
    controller.start();
    const cancelMock = vi.fn();
    startScanMock.mockReturnValue({
      jobId: "job-ctx",
      promise: new Promise(() => {}),
      cancel: cancelMock,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    getContextMock.mockReturnValue({
      panelOpen: false,
      isPdf: true,
      sessionId: "session-1",
      contentGeneration: 1,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    expect(cancelMock).toHaveBeenCalled();
    expect(state.isScanning).toBe(false);
    expect(state.error).toBe("Belge taranamıyor veya panel kapalı.");
  });

  it("rejects malformed handle and sets state.failScan", () => {
    controller.start();
    // Missing jobId
    startScanMock.mockReturnValueOnce({ promise: Promise.resolve({}), cancel: vi.fn() });
    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);
    expect(state.error).toBe("Scan job başlatılamadı.");

    // Missing promise.then
    startScanMock.mockReturnValueOnce({ jobId: "job-1", cancel: vi.fn() });
    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);
    expect(state.error).toBe("Scan job başlatılamadı.");

    // Missing cancel
    startScanMock.mockReturnValueOnce({ jobId: "job-1", promise: Promise.resolve({}) });
    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);
    expect(state.error).toBe("Scan job başlatılamadı.");
  });

  it("old reject/finally does NOT mutate new scan state", async () => {
    controller.start();
    let reject1;
    const promise1 = new Promise((_, rej) => { reject1 = rej; });
    startScanMock.mockReturnValueOnce({
      jobId: "job-old",
      promise: promise1,
      cancel: vi.fn(),
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    const promise2 = Promise.resolve({ schemaVersion: 2, items: [] });
    startScanMock.mockReturnValueOnce({
      jobId: "job-new",
      promise: promise2,
      cancel: vi.fn(),
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    reject1(new Error("Old job failed"));
    await promise1.catch(() => {});
    await promise2;

    expect(state.error).toBeNull();
    expect(state.report).toEqual({ schemaVersion: 2, items: [] });
  });

  it("second scan cancels first scan and invalidates its result", async () => {
    controller.start();

    const cancel1 = vi.fn();
    let resolve1;
    const promise1 = new Promise((res) => {
      resolve1 = res;
    });

    startScanMock.mockReturnValueOnce({
      jobId: "job-1",
      promise: promise1,
      cancel: cancel1,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    const cancel2 = vi.fn();
    let resolve2;
    const promise2 = new Promise((res) => {
      resolve2 = res;
    });

    startScanMock.mockReturnValueOnce({
      jobId: "job-2",
      promise: promise2,
      cancel: cancel2,
    });

    controller.scan([{ label: "Email", regex: "@", active: true }]);

    expect(cancel1).toHaveBeenCalled();
    expect(state.capturedPatternLabels).toEqual(["Email"]);

    resolve1({ schemaVersion: 2, items: [] });
    await promise1;

    expect(state.isScanning).toBe(true);

    const report2 = { schemaVersion: 2, items: [] };
    resolve2(report2);
    await promise2;

    expect(state.isScanning).toBe(false);
    expect(state.report).toEqual(report2);
  });

  it("handles synchronous throw and rejection in cancel function gracefully", () => {
    controller.start();
    startScanMock.mockReturnValue({
      jobId: "job-1",
      promise: new Promise(() => {}),
      cancel: () => {
        throw new Error("Cancel error");
      },
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    expect(() => controller.cancelActive({ reset: true })).not.toThrow();
    expect(state.isScanning).toBe(false);
  });

  it("handles cancel Promise rejection gracefully", () => {
    controller.start();
    startScanMock.mockReturnValue({
      jobId: "job-1",
      promise: new Promise(() => {}),
      cancel: () => Promise.reject(new Error("Async cancel error")),
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    expect(() => controller.cancelActive({ reset: true })).not.toThrow();
  });

  it("is idempotent on destroy and cancelActive", () => {
    controller.start();
    expect(() => {
      controller.cancelActive();
      controller.cancelActive();
      controller.destroy();
      controller.destroy();
    }).not.toThrow();
  });

  it("does NOT apply old report when zero active patterns scan cancels existing job", async () => {
    controller.start();
    let resolve1;
    const promise1 = new Promise((res) => { resolve1 = res; });
    const cancel1 = vi.fn();
    startScanMock.mockReturnValueOnce({
      jobId: "job-pattern-1",
      promise: promise1,
      cancel: cancel1,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);
    expect(state.isScanning).toBe(true);

    controller.scan([{ label: "TC", regex: "\\d{11}", active: false }]);
    expect(cancel1).toHaveBeenCalled();
    expect(state.isScanning).toBe(false);
    expect(state.error).toBe("En az bir aktif kalıp seçilmelidir.");

    resolve1({ schemaVersion: 2, truncated: false, warnings: [], items: [] });
    await promise1;

    expect(state.report).toBeNull();
  });

  it("does NOT apply old promise resolve/reject result when scan with invalid context cancels existing job", async () => {
    controller.start();
    let resolve1;
    const promise1 = new Promise((res) => { resolve1 = res; });
    const cancel1 = vi.fn();
    startScanMock.mockReturnValueOnce({
      jobId: "job-invalid-ctx",
      promise: promise1,
      cancel: cancel1,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    getContextMock.mockReturnValue({
      panelOpen: false,
      isPdf: true,
      sessionId: "session-1",
      contentGeneration: 1,
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);
    expect(cancel1).toHaveBeenCalled();
    expect(state.error).toBe("Belge taranamıyor veya panel kapalı.");

    resolve1({ schemaVersion: 2, items: [{ pageIndex: 0 }] });
    await promise1;

    expect(state.report).toBeNull();
  });

  it("resets state.error to null when panel is closed after zero-pattern error", () => {
    controller.start();
    controller.scan([{ label: "TC", regex: "\\d{11}", active: false }]);
    expect(state.error).toBe("En az bir aktif kalıp seçilmelidir.");

    getContextMock.mockReturnValue({
      panelOpen: false,
      isPdf: true,
      sessionId: "session-1",
      contentGeneration: 1,
    });

    controller.reconcileContext();
    expect(state.error).toBeNull();
    expect(state.isScanning).toBe(false);
    expect(state.report).toBeNull();
    expect(state.capturedPatternLabels).toEqual([]);
    expect(state.logicalHighlightsByPage).toEqual({});
  });

  it("resets all presentation state on session/generation change after backend error", async () => {
    controller.start();
    let reject1;
    const promise1 = new Promise((_, rej) => { reject1 = rej; });
    startScanMock.mockReturnValueOnce({
      jobId: "job-err-reconcile",
      promise: promise1,
      cancel: vi.fn(),
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    reject1(new Error("Backend IPC error"));
    await promise1.catch(() => {});

    expect(state.error).toBe("Backend IPC error");

    getContextMock.mockReturnValue({
      panelOpen: true,
      isPdf: true,
      sessionId: "session-2",
      contentGeneration: 1,
    });

    controller.reconcileContext();

    expect(state.isScanning).toBe(false);
    expect(state.report).toBeNull();
    expect(state.error).toBeNull();
    expect(state.capturedPatternLabels).toEqual([]);
    expect(state.logicalHighlightsByPage).toEqual({});
  });

  it("clears capturedContext on spontaneous SESSION_JOB_CANCELLED so reconcileContext leaves no stale ownership", async () => {
    controller.start();
    let reject1;
    const promise1 = new Promise((_, rej) => { reject1 = rej; });
    startScanMock.mockReturnValueOnce({
      jobId: "job-spontaneous",
      promise: promise1,
      cancel: vi.fn(),
    });

    controller.scan([{ label: "TC", regex: "\\d{11}", active: true }]);

    const cancelErr = new Error("Spontaneous cancel");
    cancelErr.code = "SESSION_JOB_CANCELLED";
    reject1(cancelErr);
    await promise1.catch(() => {});

    expect(state.isScanning).toBe(false);
    expect(state.error).toBeNull();

    // Call reconcileContext with exact same context
    controller.reconcileContext();
    expect(state.isScanning).toBe(false);
    expect(state.error).toBeNull();
  });
});

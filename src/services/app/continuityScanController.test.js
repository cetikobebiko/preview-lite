import { describe, expect, it, vi } from "vitest";
import { FeatureUnavailableError, guardFeatureAction } from "../../featureActionGuard.js";
import { SignatureLossDeclinedError } from "../signatureLossGuard.js";
import {
  CONTINUITY_SCAN_MODAL,
  CONTINUITY_SCAN_SIGNED_EDIT_LABEL,
  continuityScanErrorMessage,
  createContinuityScanController,
} from "./continuityScanController.js";

const productionMac = { build: "production", platform: "macos" };
const A4 = { width: 595, height: 842 };

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function cancelledError() {
  return Object.assign(new Error("Session job iptal edildi"), { code: "SESSION_JOB_CANCELLED" });
}

function setup({ context = {}, featureContext = productionMac } = {}) {
  const state = {
    context: { isPdf: true, sessionId: "s1", contentGeneration: 4, pageIndex: 1, pageCount: 3, ...context },
  };
  const job = deferred();
  const handle = { jobId: "job-1", promise: job.promise, cancel: vi.fn(() => Promise.resolve()) };
  const modals = [];
  const deps = {
    getContext: () => state.context,
    guard: guardFeatureAction,
    featureContext,
    reportUnavailable: vi.fn((error) => error instanceof FeatureUnavailableError),
    confirmSignedEdit: vi.fn(async () => {}),
    resolvePageFit: vi.fn(async () => A4),
    startScan: vi.fn(() => handle),
    getPageCount: vi.fn(async () => 5),
    applyMutationSnapshot: vi.fn(async () => {}),
    ui: {
      openModal: vi.fn((type, props) => modals.push({ type, props })),
      closeModal: vi.fn(),
    },
    onToast: vi.fn(),
    onError: vi.fn(),
  };
  const controller = createContinuityScanController(deps);
  return { controller, deps, state, job, handle, modals };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("continuity scan controller", () => {
  it("scans after the current page, fits images to the neighbour page and applies one mutation", async () => {
    const { controller, deps, job, modals } = setup();
    const running = controller.scan();
    await flush();

    expect(deps.confirmSignedEdit).toHaveBeenCalledWith("s1", 4, CONTINUITY_SCAN_SIGNED_EDIT_LABEL);
    expect(deps.resolvePageFit).toHaveBeenCalledWith("s1", 4, 1);
    expect(deps.startScan).toHaveBeenCalledWith("s1", 4, 2, A4, productionMac);
    expect(modals).toHaveLength(1);
    expect(modals[0].type).toBe(CONTINUITY_SCAN_MODAL);
    expect(controller.isActive).toBe(true);

    const snapshot = { id: "s1", contentGeneration: 5 };
    job.resolve({ snapshot, kind: "pdf" });
    await expect(running).resolves.toBe(snapshot);

    expect(deps.getPageCount).toHaveBeenCalledWith("s1", 5);
    const [applied, options] = deps.applyMutationSnapshot.mock.calls[0];
    expect(applied).toBe(snapshot);
    // Two pages went in at index 2: pages before stay, later ones shift by two.
    expect([0, 1, 2].map(options.mapPageIndex)).toEqual([0, 1, 4]);
    expect(deps.onToast).toHaveBeenCalledWith(expect.stringContaining("2 sayfa"));
    expect(deps.ui.closeModal).toHaveBeenCalledWith(CONTINUITY_SCAN_MODAL);
    expect(deps.onError).not.toHaveBeenCalled();
    expect(controller.isActive).toBe(false);
  });

  it("clamps an explicit insertion index and uses the end when no page is current", async () => {
    const explicit = setup();
    void explicit.controller.scan({ atIndex: 99 });
    await flush();
    expect(explicit.deps.startScan).toHaveBeenCalledWith("s1", 4, 3, A4, productionMac);
    expect(explicit.deps.resolvePageFit).toHaveBeenCalledWith("s1", 4, 2);

    const start = setup();
    void start.controller.scan({ atIndex: 0 });
    await flush();
    expect(start.deps.startScan).toHaveBeenCalledWith("s1", 4, 0, A4, productionMac);
    expect(start.deps.resolvePageFit).toHaveBeenCalledWith("s1", 4, 0);

    const noPage = setup({ context: { pageIndex: -1 } });
    void noPage.controller.scan();
    await flush();
    expect(noPage.deps.startScan).toHaveBeenCalledWith("s1", 4, 3, A4, productionMac);
  });

  it("needs an open PDF", async () => {
    const { controller, deps } = setup({ context: { isPdf: false } });
    await expect(controller.scan()).resolves.toBeNull();
    expect(deps.startScan).not.toHaveBeenCalled();
    expect(deps.onToast).toHaveBeenCalledWith(expect.stringContaining("açık bir PDF"));
  });

  it("runs one scan at a time", async () => {
    const { controller, deps, job } = setup();
    const first = controller.scan();
    await flush();
    await expect(controller.scan()).resolves.toBeNull();
    expect(deps.startScan).toHaveBeenCalledOnce();
    expect(deps.onToast).toHaveBeenCalledWith(expect.stringContaining("sürüyor"));
    job.reject(cancelledError());
    await first;
  });

  it("cancels the job from the dialog and leaves the document alone", async () => {
    const { controller, deps, job, handle, modals } = setup();
    const running = controller.scan();
    await flush();
    modals[0].props.oncancel();
    expect(handle.cancel).toHaveBeenCalledOnce();
    job.reject(cancelledError());
    await expect(running).resolves.toBeNull();
    expect(deps.applyMutationSnapshot).not.toHaveBeenCalled();
    expect(deps.onToast).toHaveBeenCalledWith(expect.stringContaining("iptal edildi"));
    expect(deps.onError).not.toHaveBeenCalled();
    expect(deps.ui.closeModal).toHaveBeenCalledWith(CONTINUITY_SCAN_MODAL);
    expect(controller.isActive).toBe(false);
  });

  it("does not start the helper when cancelled while the page size is resolving", async () => {
    const { controller, deps } = setup();
    const size = deferred();
    deps.resolvePageFit.mockReturnValueOnce(size.promise);
    const running = controller.scan();
    await flush();
    controller.cancel();
    size.resolve(A4);
    await expect(running).resolves.toBeNull();
    expect(deps.startScan).not.toHaveBeenCalled();
  });

  it("cancels the scan when the active document changes", async () => {
    const { controller, state, handle, job } = setup();
    const running = controller.scan();
    await flush();
    controller.reconcileContext();
    expect(handle.cancel).not.toHaveBeenCalled();
    state.context = { ...state.context, sessionId: "s2" };
    controller.reconcileContext();
    expect(handle.cancel).toHaveBeenCalledOnce();
    job.reject(cancelledError());
    await running;
  });

  it("applies a late result without announcing it once another document is active", async () => {
    const { controller, deps, state, job } = setup();
    const running = controller.scan();
    await flush();
    state.context = { ...state.context, sessionId: "s2" };
    const snapshot = { id: "s1", contentGeneration: 5 };
    job.resolve({ snapshot, kind: "images" });
    await running;
    // documentRuntime.applyMutationSnapshot ignores snapshots of inactive sessions.
    expect(deps.applyMutationSnapshot).toHaveBeenCalledOnce();
    expect(deps.onToast).not.toHaveBeenCalled();
  });

  it.each([
    ["CONTINUITY_SCAN_NO_DEVICE", "Apple hesabı"],
    ["CONTINUITY_SCAN_TIMEOUT", "zamanında"],
    ["CONTINUITY_SCAN_UNSUPPORTED_CONTENT", "desteklenmeyen"],
    ["CONTINUITY_SCAN_LIMIT_EXCEEDED", "100 sayfa"],
    ["STALE_GENERATION", "Belge tarama beklenirken değişti"],
    ["FEATURE_UNAVAILABLE", "macOS 10.14"],
    ["CONTINUITY_SCAN_OUTPUT_INVALID", "bozuk çıktı"],
  ])("reports %s as an unchanged-document error", async (code, text) => {
    const { controller, deps, job } = setup();
    const running = controller.scan();
    await flush();
    job.reject(Object.assign(new Error("bozuk çıktı"), { code }));
    await expect(running).resolves.toBeNull();
    expect(deps.onError).toHaveBeenCalledOnce();
    const message = deps.onError.mock.calls[0][0];
    expect(message).toContain(text);
    expect(message).toMatch(/belge değişmedi|eklenmedi|kullanılamıyor|Apple hesabı/);
    expect(deps.applyMutationSnapshot).not.toHaveBeenCalled();
    expect(deps.ui.closeModal).toHaveBeenCalledWith(CONTINUITY_SCAN_MODAL);
  });

  it("stops quietly when the signed-document notice is declined", async () => {
    const { controller, deps } = setup();
    deps.confirmSignedEdit.mockRejectedValueOnce(new SignatureLossDeclinedError());
    await expect(controller.scan()).resolves.toBeNull();
    expect(deps.startScan).not.toHaveBeenCalled();
    expect(deps.onError).not.toHaveBeenCalled();
  });

  it("reports the feature as unavailable off macOS without starting", async () => {
    const { controller, deps } = setup({ featureContext: { build: "production", platform: "windows" } });
    await expect(controller.scan()).resolves.toBeNull();
    expect(deps.reportUnavailable).toHaveBeenCalledWith(expect.any(FeatureUnavailableError));
    expect(deps.startScan).not.toHaveBeenCalled();
    expect(deps.confirmSignedEdit).not.toHaveBeenCalled();
  });

  it("falls back to the backend message for unknown errors", () => {
    expect(continuityScanErrorMessage({ code: "X", message: "ayrıntı" })).toContain("ayrıntı");
    expect(continuityScanErrorMessage("düz hata")).toContain("düz hata");
  });
});

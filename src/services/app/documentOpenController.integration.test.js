import { describe, expect, it, vi } from "vitest";
import { createDocumentSessionStore } from "../../stores/documentSession.svelte.js";
import { createDocumentOpenController } from "./documentOpenController.js";

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function snapshot(id, lifecycle = "open", overrides = {}) {
  return {
    id,
    lifecycle,
    contentGeneration: 1,
    stateRevision: 1,
    dirty: false,
    displayPath: `/${id}.pdf`,
    fileName: `${id}.pdf`,
    ...overrides,
  };
}

function createHarness({ initialActive = null, decision = { action: "discard" } } = {}) {
  let active = initialActive;
  let candidateSequence = 0;
  let mounted = true;
  const candidates = new Map();
  const documentSession = createDocumentSessionStore();
  if (initialActive) documentSession.commitCandidate(initialActive);

  const terminalTransition = async (token) => {
    if (token.endsWith("document_switch")) {
      const prepared = [...candidates.values()].find(
        (candidate) => candidate.lifecycle === "candidate_activation_prepared",
      );
      active = snapshot(prepared.id, "open", {
        ...prepared,
        lifecycle: "open",
        stateRevision: prepared.stateRevision + 1,
      });
      candidates.set(active.id, active);
      return active;
    }
    const closed = snapshot(active.id, "closed", {
      ...active,
      lifecycle: "closed",
      stateRevision: active.stateRevision + 1,
    });
    active = null;
    return closed;
  };

  const backend = {
    getActiveDocumentSessionSnapshot: vi.fn(async () => active),
    getDocumentSessionSnapshot: vi.fn(async (id) => candidates.get(id) ?? null),
    openDocumentSession: vi.fn(async (path) => {
      const id = `candidate-${++candidateSequence}`;
      const candidate = snapshot(id, "candidate_ready", {
        displayPath: path,
        fileName: path.split("/").pop(),
      });
      candidates.set(id, candidate);
      return { status: "candidate_ready", sessionId: id, snapshot: candidate };
    }),
    authenticateDocumentSession: vi.fn(),
    probeCandidateRender: vi.fn(async () => ({ pageCount: 2, width: 4, height: 3, byteLength: 48 })),
    beginSessionTransition: vi.fn(async (id, kind) => ({ token: `${id}-${kind}`, snapshot: active })),
    saveTransition: vi.fn(terminalTransition),
    discardTransition: vi.fn(terminalTransition),
    cancelTransition: vi.fn(async () => active),
    prepareCandidateActivation: vi.fn(async (candidateId) => {
      const candidate = candidates.get(candidateId);
      if (candidate) {
        candidates.set(candidateId, {
          ...candidate,
          lifecycle: "candidate_activation_prepared",
          stateRevision: candidate.stateRevision + 1,
        });
      }
      return { activationToken: `activation-${candidateId}` };
    }),
    commitCandidateActivation: vi.fn(async (token) => {
      const id = token.replace("activation-", "");
      const candidate = candidates.get(id);
      active = snapshot(id, "open", {
        ...candidate,
        lifecycle: "open",
        stateRevision: (candidate?.stateRevision ?? 2) + 1,
      });
      candidates.set(id, active);
      return { snapshot: active, cleanupWarnings: [] };
    }),
    rollbackCandidateActivation: vi.fn(async (token) => {
      candidates.delete(token.replace("activation-", ""));
      return active;
    }),
    discardCandidateSession: vi.fn(async (id) => candidates.delete(id)),
    readImageFile: vi.fn(async () => ({ width: 2, height: 2, data: new Uint8Array(16) })),
  };

  const lifecycle = {
    requestPassword: vi.fn(),
    dismissPasswordAttempt: vi.fn(),
    reportPasswordError: vi.fn(),
    destroy: vi.fn(() => { mounted = false; }),
  };
  const viewer = {
    activeImageSourcePath: "",
    fileExt: "",
    pageOrder: [],
    currentPageIndex: 0,
    scale: 1,
    visualScale: 1,
    rotation: 0,
    flipH: false,
    flipV: false,
    outlineList: [],
    pdfMetadata: { title: "", author: "", subject: "", keywords: "", creator: "" },
    isDocumentLoading: false,
    isRendering: false,
  };
  const annotation = { activeTool: "pointer" };
  const viewerController = {
    activatePdf: vi.fn(),
    deactivatePdf: vi.fn(),
    paintImage: vi.fn(),
  };
  const setError = vi.fn();
  const onToast = vi.fn();
  const chooseTransition = vi.fn(async () => decision);
  const getOutline = vi.fn(async () => []);
  const getMetadata = vi.fn(async () => ({ title: "", author: "", subject: "", keywords: "", creator: "" }));
  const addRecentFile = vi.fn(async () => {});
  const applyBackendSessionSnapshot = vi.fn((next) => {
    if (next?.id !== documentSession.activeSessionId) return false;
    return documentSession.applySnapshot(next);
  });

  const controller = createDocumentOpenController({
    backend,
    documentSession,
    documentLifecycleController: lifecycle,
    viewer,
    annotation,
    viewerController,
    renderSession: { snapshot: () => ({ contentGeneration: documentSession.contentGeneration }) },
    getCanvas: () => ({ getContext: vi.fn() }),
    getIsMounted: () => mounted,
    tick: async () => {},
    applyBackendSessionSnapshot,
    chooseTransition,
    chooseTransitionSaveRecovery: vi.fn(async () => ({ action: "cancel" })),
    invalidateSearch: vi.fn(),
    restoreDocumentPosition: vi.fn(),
    addRecentFile,
    getOutline,
    getMetadata,
    setError,
    chooseFile: vi.fn(),
    onWarning: vi.fn(),
    onToast,
  });

  return {
    controller,
    backend,
    candidates,
    documentSession,
    lifecycle,
    viewer,
    viewerController,
    setError,
    onToast,
    chooseTransition,
    getOutline,
    getMetadata,
    addRecentFile,
    setActive(value) { active = value; },
    getActive() { return active; },
    setMounted(value) { mounted = value; },
  };
}

describe("createDocumentOpenController integration boundary", () => {
  it("opens clean PDF → new PDF without asking and commits the new snapshot", async () => {
    const harness = createHarness({ initialActive: snapshot("clean") });

    await expect(harness.controller.requestFile("/new.pdf")).resolves.toMatchObject({ status: "committed" });

    expect(harness.chooseTransition).not.toHaveBeenCalled();
    expect(harness.backend.discardTransition).toHaveBeenCalledWith("clean-document_switch");
    expect(harness.documentSession.active?.displayPath).toBe("/new.pdf");
  });

  it("tells the user when the opened PDF is read-only", async () => {
    const harness = createHarness();
    harness.backend.openDocumentSession.mockImplementationOnce(async (path) => {
      const candidate = snapshot("owner-only", "candidate_ready", {
        displayPath: path,
        editable: false,
        readOnlyReason: "Şifreli PDF mutasyonları desteklenmiyor",
      });
      harness.candidates.set(candidate.id, candidate);
      return { status: "candidate_ready", sessionId: candidate.id, snapshot: candidate };
    });

    await expect(harness.controller.requestFile("/owner-only.pdf")).resolves.toMatchObject({ status: "committed" });

    expect(harness.lifecycle.requestPassword).not.toHaveBeenCalled();
    expect(harness.onToast).toHaveBeenCalledWith("Belge salt okunur açıldı: Şifreli PDF mutasyonları desteklenmiyor.");
  });

  it("opens an editable PDF without a read-only notice", async () => {
    const harness = createHarness();

    await expect(harness.controller.requestFile("/plain.pdf")).resolves.toMatchObject({ status: "committed" });

    expect(harness.onToast).not.toHaveBeenCalled();
  });

  it("opens dirty PDF → save → new PDF", async () => {
    const harness = createHarness({ initialActive: snapshot("dirty", "open", { dirty: true }), decision: { action: "save" } });

    await expect(harness.controller.requestFile("/saved-next.pdf")).resolves.toMatchObject({ status: "committed" });

    expect(harness.backend.saveTransition).toHaveBeenCalledWith("dirty-document_switch", null);
    expect(harness.backend.discardTransition).not.toHaveBeenCalled();
    expect(harness.documentSession.active?.displayPath).toBe("/saved-next.pdf");
  });

  it("opens dirty PDF → discard → new PDF", async () => {
    const harness = createHarness({ initialActive: snapshot("dirty", "open", { dirty: true }), decision: { action: "discard" } });

    await expect(harness.controller.requestFile("/discard-next.pdf")).resolves.toMatchObject({ status: "committed" });

    expect(harness.backend.discardTransition).toHaveBeenCalledWith("dirty-document_switch");
    expect(harness.documentSession.active?.displayPath).toBe("/discard-next.pdf");
  });

  it("keeps the dirty PDF on cancel and rolls back candidate activation", async () => {
    const previous = snapshot("dirty", "open", { dirty: true });
    const harness = createHarness({ initialActive: previous, decision: { action: "cancel" } });

    await expect(harness.controller.requestFile("/cancelled.pdf")).resolves.toMatchObject({ status: "cancelled" });

    expect(harness.backend.rollbackCandidateActivation).toHaveBeenCalledWith("activation-candidate-1");
    expect(harness.backend.commitCandidateActivation).not.toHaveBeenCalled();
    expect(harness.documentSession.activeSessionId).toBe("dirty");
    expect(harness.viewer.activeImageSourcePath).toBe("");
  });

  it("authenticates an encrypted candidate with the correct password", async () => {
    const harness = createHarness();
    const encrypted = snapshot("encrypted", "candidate_ready", { displayPath: "/encrypted.pdf" });
    harness.candidates.set(encrypted.id, encrypted);
    harness.backend.openDocumentSession.mockResolvedValue({ status: "password_required", sessionId: encrypted.id, snapshot: null });
    harness.lifecycle.requestPassword.mockResolvedValue("correct-password");
    harness.backend.authenticateDocumentSession.mockResolvedValue(encrypted);

    await expect(harness.controller.requestFile("/encrypted.pdf")).resolves.toMatchObject({ status: "committed" });

    expect(harness.backend.authenticateDocumentSession).toHaveBeenCalledWith("encrypted", "correct-password");
    expect(harness.documentSession.activeSessionId).toBe("encrypted");
  });

  it("retries a wrong password under the same attempt ownership", async () => {
    const harness = createHarness();
    const encrypted = snapshot("encrypted", "candidate_ready", { displayPath: "/encrypted.pdf" });
    harness.candidates.set(encrypted.id, encrypted);
    harness.backend.openDocumentSession.mockResolvedValue({ status: "password_required", sessionId: encrypted.id, snapshot: null });
    harness.lifecycle.requestPassword.mockResolvedValueOnce("wrong").mockResolvedValueOnce("correct");
    harness.backend.authenticateDocumentSession
      .mockRejectedValueOnce(Object.assign(new Error("wrong password"), { code: "DOCUMENT_SESSION_INVALID_PASSWORD" }))
      .mockResolvedValueOnce(encrypted);

    await expect(harness.controller.requestFile("/encrypted.pdf")).resolves.toMatchObject({ status: "committed" });

    expect(harness.lifecycle.requestPassword).toHaveBeenCalledTimes(2);
    const attempts = harness.lifecycle.requestPassword.mock.calls.map(([request]) => request.attemptId);
    expect(new Set(attempts).size).toBe(1);
    expect(harness.lifecycle.reportPasswordError).toHaveBeenCalledWith(expect.objectContaining({ attemptId: attempts[0] }));
  });

  it("reconciles a commit outcome that was initially unknown", async () => {
    const harness = createHarness();
    const authoritative = deferred();
    let activeRead = 0;
    harness.backend.commitCandidateActivation.mockRejectedValue(new Error("response lost"));
    harness.backend.getActiveDocumentSessionSnapshot.mockImplementation(() => {
      activeRead += 1;
      if (activeRead === 1) return Promise.resolve(null);
      if (activeRead <= 4) return Promise.reject(new Error("offline"));
      return authoritative.promise;
    });

    await expect(harness.controller.requestFile("/unknown.pdf")).resolves.toMatchObject({ status: "commit_outcome_unknown" });
    const committed = snapshot("candidate-1", "open", { stateRevision: 3, displayPath: "/unknown.pdf" });
    harness.setActive(committed);
    authoritative.resolve(committed);

    await vi.waitFor(() => expect(harness.documentSession.activeSessionId).toBe("candidate-1"));
    expect(harness.backend.commitCandidateActivation).toHaveBeenCalledOnce();
    expect(harness.viewerController.activatePdf).toHaveBeenCalledOnce();
    expect(harness.setError).toHaveBeenLastCalledWith("");
  });

  it("recovers failed precommit candidate cleanup before returning to idle", async () => {
    const harness = createHarness();
    const recovery = deferred();
    harness.backend.probeCandidateRender.mockRejectedValue(new Error("probe failed"));
    harness.backend.discardCandidateSession
      .mockRejectedValueOnce(new Error("cleanup failed"))
      .mockReturnValueOnce(recovery.promise);

    await expect(harness.controller.requestFile("/broken.pdf")).resolves.toMatchObject({ status: "precommit_recovery_required" });
    expect(harness.controller.coordinator.snapshot().phase).toBe("precommit_recovery_required");
    recovery.resolve();

    await vi.waitFor(() => expect(harness.controller.coordinator.snapshot().phase).toBe("idle"));
    expect(harness.backend.discardCandidateSession).toHaveBeenCalledTimes(2);
    expect(harness.setError).toHaveBeenLastCalledWith("");
  });

  it("gives rapid successive open requests to the latest request only", async () => {
    const harness = createHarness();
    const slowOpen = deferred();
    harness.backend.openDocumentSession.mockImplementationOnce(() => slowOpen.promise);

    const first = harness.controller.requestFile("/slow.pdf");
    await vi.waitFor(() => expect(harness.backend.openDocumentSession).toHaveBeenCalledOnce());
    const latest = harness.controller.requestFile("/latest.pdf");
    const slowCandidate = snapshot("slow-candidate", "candidate_ready", { displayPath: "/slow.pdf" });
    harness.candidates.set(slowCandidate.id, slowCandidate);
    slowOpen.resolve({ status: "candidate_ready", sessionId: slowCandidate.id, snapshot: slowCandidate });

    await expect(first).resolves.toMatchObject({ status: "superseded" });
    await expect(latest).resolves.toMatchObject({ status: "committed" });
    expect(harness.documentSession.active?.displayPath).toBe("/latest.pdf");
    expect(harness.viewerController.activatePdf).toHaveBeenCalledOnce();
  });

  it("switches PDF → image and closes the PDF presentation identity", async () => {
    const harness = createHarness({ initialActive: snapshot("pdf") });

    await expect(harness.controller.requestFile("/photo.png")).resolves.toMatchObject({ status: "committed" });

    expect(harness.backend.discardTransition).toHaveBeenCalledWith("pdf-document_close");
    expect(harness.documentSession.activeSessionId).toBeNull();
    expect(harness.viewer.activeImageSourcePath).toBe("/photo.png");
    expect(harness.viewer.fileExt).toBe("png");
    expect(harness.viewerController.deactivatePdf).toHaveBeenCalledOnce();
  });

  it("switches image → PDF and clears independent image presentation state", async () => {
    const harness = createHarness();
    await harness.controller.requestFile("/photo.png");

    await expect(harness.controller.requestFile("/document.pdf")).resolves.toMatchObject({ status: "committed" });

    expect(harness.documentSession.active?.displayPath).toBe("/document.pdf");
    expect(harness.viewer.activeImageSourcePath).toBe("");
    expect(harness.viewer.fileExt).toBe("pdf");
  });

  it("guards outline and metadata results by both session ID and content generation", async () => {
    const harness = createHarness();
    const outlineA = deferred();
    const outlineB = deferred();
    const metadataA = deferred();
    const metadataB = deferred();
    harness.getOutline.mockReturnValueOnce(outlineA.promise).mockReturnValueOnce(outlineB.promise);
    harness.getMetadata.mockReturnValueOnce(metadataA.promise).mockReturnValueOnce(metadataB.promise);

    await harness.controller.requestFile("/a.pdf");
    await harness.controller.requestFile("/b.pdf");
    outlineA.resolve([{ title: "stale session outline" }]);
    metadataA.resolve({ title: "stale session metadata" });
    await flush();
    expect(harness.viewer.outlineList).toEqual([]);
    expect(harness.viewer.pdfMetadata.title).toBe("");

    harness.documentSession.applySnapshot({
      ...harness.documentSession.active,
      contentGeneration: 2,
      stateRevision: harness.documentSession.stateRevision + 1,
    });
    outlineB.resolve([{ title: "stale generation outline" }]);
    metadataB.resolve({ title: "stale generation metadata" });
    await flush();
    expect(harness.viewer.outlineList).toEqual([]);
    expect(harness.viewer.pdfMetadata.title).toBe("");
  });

  it("does not apply late open or presentation results after stop/destroy", async () => {
    const harness = createHarness();
    const slowOpen = deferred();
    harness.backend.openDocumentSession.mockReturnValueOnce(slowOpen.promise);
    const opening = harness.controller.requestFile("/late.pdf");
    await vi.waitFor(() => expect(harness.backend.openDocumentSession).toHaveBeenCalledOnce());
    const stopping = harness.controller.coordinator.stop();
    const candidate = snapshot("late", "candidate_ready", { displayPath: "/late.pdf" });
    harness.candidates.set(candidate.id, candidate);
    slowOpen.resolve({ status: "candidate_ready", sessionId: candidate.id, snapshot: candidate });

    await expect(stopping).resolves.toMatchObject({ status: "stopped" });
    await expect(opening).resolves.toMatchObject({ status: "superseded" });
    expect(harness.documentSession.activeSessionId).toBeNull();
    expect(harness.viewerController.activatePdf).not.toHaveBeenCalled();

    harness.controller.coordinator.resume();
    const lateOutline = deferred();
    const lateMetadata = deferred();
    harness.getOutline.mockReturnValueOnce(lateOutline.promise);
    harness.getMetadata.mockReturnValueOnce(lateMetadata.promise);
    await harness.controller.requestFile("/committed.pdf");
    harness.lifecycle.destroy();
    lateOutline.resolve([{ title: "after destroy" }]);
    lateMetadata.resolve({ title: "after destroy" });
    await flush();
    expect(harness.viewer.outlineList).toEqual([]);
    expect(harness.viewer.pdfMetadata.title).toBe("");
  });
});

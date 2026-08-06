import { describe, expect, it, vi } from "vitest";
import {
  createDocumentOpenCoordinator,
} from "./documentOpenCoordinator.js";

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

function createDependencies(overrides = {}) {
  let active = overrides.initialActive ?? null;
  let candidateSequence = 0;
  const candidates = new Map();
  const applyTerminalTransition = async (token) => {
    if (token.endsWith("document_switch")) {
      const prepared = [...candidates.values()].find(
        (candidate) => candidate.lifecycle === "candidate_activation_prepared",
      );
      active = snapshot(prepared.id, "open", { stateRevision: prepared.stateRevision + 1 });
      candidates.set(active.id, active);
      return active;
    }
    const closed = snapshot(active.id, "closed", {
      ...active,
      stateRevision: active.stateRevision + 1,
      lifecycle: "closed",
    });
    active = null;
    return closed;
  };
  const dependencies = {
    getActiveDocumentSessionSnapshot: vi.fn(async () => active),
    getDocumentSessionSnapshot: vi.fn(async (id) => candidates.get(id)),
    openDocumentSession: vi.fn(async () => {
      const id = `candidate-${++candidateSequence}`;
      const candidate = snapshot(id, "candidate_ready");
      candidates.set(id, candidate);
      return { status: "candidate_ready", sessionId: id, snapshot: candidate };
    }),
    authenticateDocumentSession: vi.fn(),
    requestPassword: vi.fn(),
    onPasswordError: vi.fn(),
    dismissAttemptDialog: vi.fn(),
    probeCandidateRender: vi.fn(async () => ({
      pageCount: 2,
      width: 16,
      height: 8,
      byteLength: 512,
    })),
    beginSessionTransition: vi.fn(async (id, kind) => ({
      token: `${id}-${kind}`,
      snapshot: active,
    })),
    chooseTransition: vi.fn(async () => ({ action: "discard" })),
    onTransitionSaveError: vi.fn(),
    saveTransition: vi.fn(applyTerminalTransition),
    discardTransition: vi.fn(applyTerminalTransition),
    cancelTransition: vi.fn(),
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
      active = snapshot(id, "open", { stateRevision: 3 });
      candidates.set(id, active);
      return { snapshot: active, cleanupWarnings: [] };
    }),
    rollbackCandidateActivation: vi.fn(async (token) => {
      candidates.delete(token.replace("activation-", ""));
      return active;
    }),
    discardCandidateSession: vi.fn(async (id) => {
      candidates.delete(id);
    }),
    readImageFile: vi.fn(async () => ({
      width: 2,
      height: 2,
      data: new Uint8Array(16),
    })),
    prepareImage: vi.fn(),
    prepareImageActivation: vi.fn(),
    publishCommittedImage: vi.fn(),
    reconcileCommittedImage: vi.fn(),
    activateCommittedImageViewer: vi.fn(),
    afterImageCommit: vi.fn(),
    commitDocumentSnapshot: vi.fn(),
    reconcileCommittedDocumentSnapshot: vi.fn(),
    activateCommittedPdfRender: vi.fn(),
    afterPdfCommit: vi.fn(),
    onViewerActivationError: vi.fn(),
    onCommitOutcomeUnknown: vi.fn(),
    onPrecommitRecoveryRequired: vi.fn(),
    applyAuthoritativeActiveSnapshot: vi.fn(),
    onCleanupWarnings: vi.fn(),
    onSecondaryError: vi.fn(),
    onError: vi.fn(),
    onStateChange: vi.fn(),
  };
  Object.assign(dependencies, overrides);
  return { dependencies, candidates, getActive: () => active, setActive: (value) => { active = value; } };
}

describe("document open coordinator", () => {
  it.each([
    ["first PDF open", null],
    ["image to PDF", null],
  ])("%s prepares activation with an explicit null previous pair", async (_name, initialActive) => {
    const { dependencies } = createDependencies({ initialActive });
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestPdf("/same.pdf")).resolves.toMatchObject({
      status: "committed",
    });

    expect(dependencies.prepareCandidateActivation).toHaveBeenCalledWith(
      "candidate-1",
      null,
      null,
    );
  });

  it("uses the authoritative previous PDF identity and prepared transition", async () => {
    const previous = snapshot("previous", "open");
    const { dependencies } = createDependencies({ initialActive: previous });
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await coordinator.requestPdf("/next.pdf");

    expect(dependencies.prepareCandidateActivation).toHaveBeenCalledWith(
      "candidate-1",
      "previous",
      "previous-document_switch",
    );
  });

  it("keeps only the latest pending request while commit fence is active", async () => {
    const commit = deferred();
    const { dependencies, setActive } = createDependencies();
    dependencies.commitCandidateActivation.mockImplementation(async (token) => {
      const result = await commit.promise;
      setActive(result.snapshot);
      return result;
    });
    const coordinator = createDocumentOpenCoordinator(dependencies);

    const first = coordinator.requestPdf("/first.pdf");
    while (dependencies.commitCandidateActivation.mock.calls.length === 0) await flush();
    const supersededPending = coordinator.requestPdf("/second.pdf");
    const latestPending = coordinator.requestImage("/latest.png");
    await expect(supersededPending).resolves.toEqual({ status: "superseded" });
    expect(dependencies.openDocumentSession).toHaveBeenCalledTimes(1);

    commit.resolve({
      snapshot: snapshot("candidate-1", "open", { stateRevision: 3 }),
      cleanupWarnings: [],
    });
    await expect(first).resolves.toMatchObject({ status: "committed" });
    await expect(latestPending).resolves.toMatchObject({ status: "committed" });
    expect(dependencies.publishCommittedImage).toHaveBeenCalledOnce();
  });

  it("reconciles a committed candidate when the IPC response is lost", async () => {
    const { dependencies, setActive } = createDependencies();
    dependencies.commitCandidateActivation.mockImplementation(async () => {
      const committed = snapshot("candidate-1", "open", { stateRevision: 3 });
      setActive(committed);
      throw new Error("transport lost");
    });
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestPdf("/committed.pdf")).resolves.toMatchObject({
      status: "committed",
      snapshot: { id: "candidate-1" },
    });
    expect(dependencies.rollbackCandidateActivation).not.toHaveBeenCalled();
    expect(dependencies.commitDocumentSnapshot).toHaveBeenCalledWith(
      expect.objectContaining({ id: "candidate-1", lifecycle: "open" }),
    );
  });

  it("keeps the committed PDF active when post-commit render activation fails", async () => {
    const { dependencies } = createDependencies();
    dependencies.activateCommittedPdfRender.mockRejectedValue(new Error("render unavailable"));
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestPdf("/render-failure.pdf")).resolves.toMatchObject({
      status: "committed",
    });
    expect(dependencies.onViewerActivationError).toHaveBeenCalledOnce();
    expect(dependencies.rollbackCandidateActivation).not.toHaveBeenCalled();
  });

  it("reconciles a committed snapshot after a frontend publication failure", async () => {
    const { dependencies } = createDependencies();
    dependencies.commitDocumentSnapshot.mockImplementation(() => {
      throw new Error("frontend publication failed");
    });
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestPdf("/ui-failure.pdf")).resolves.toMatchObject({
      status: "committed",
    });
    expect(dependencies.reconcileCommittedDocumentSnapshot).toHaveBeenCalledWith(
      expect.objectContaining({ id: "candidate-1", lifecycle: "open" }),
      expect.any(Error),
    );
    expect(dependencies.rollbackCandidateActivation).not.toHaveBeenCalled();
  });

  it("validates an image before transactionally deactivating the active PDF", async () => {
    const previous = snapshot("previous", "open");
    const { dependencies } = createDependencies({ initialActive: previous });
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestImage("/image.png")).resolves.toMatchObject({
      status: "committed",
    });

    expect(dependencies.readImageFile.mock.invocationCallOrder[0])
      .toBeLessThan(dependencies.beginSessionTransition.mock.invocationCallOrder[0]);
    expect(dependencies.prepareImageActivation.mock.invocationCallOrder[0])
      .toBeLessThan(dependencies.beginSessionTransition.mock.invocationCallOrder[0]);
    expect(dependencies.discardTransition).toHaveBeenCalledWith(
      "previous-document_close",
    );
    expect(dependencies.publishCommittedImage).toHaveBeenCalledOnce();
  });

  it("keeps the dirty PDF authoritative when image activation preparation fails", async () => {
    const previous = snapshot("previous", "open", { dirty: true });
    const { dependencies, getActive } = createDependencies({
      initialActive: previous,
    });
    dependencies.chooseTransition.mockResolvedValue({ action: "save" });
    dependencies.prepareImageActivation.mockRejectedValue(
      new Error("image activation unavailable"),
    );
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestImage("/image.png")).resolves.toMatchObject({
      status: "failed",
    });

    expect(dependencies.beginSessionTransition).not.toHaveBeenCalled();
    expect(dependencies.saveTransition).not.toHaveBeenCalled();
    expect(dependencies.publishCommittedImage).not.toHaveBeenCalled();
    expect(getActive()).toEqual(previous);
  });

  it("cancels a password-owned PDF candidate when an image supersedes it", async () => {
    const password = deferred();
    const { dependencies } = createDependencies();
    dependencies.openDocumentSession.mockResolvedValueOnce({
      status: "password_required",
      sessionId: "encrypted",
      snapshot: null,
    });
    dependencies.requestPassword.mockReturnValue(password.promise);
    const coordinator = createDocumentOpenCoordinator(dependencies);

    const pdf = coordinator.requestPdf("/encrypted.pdf");
    while (dependencies.requestPassword.mock.calls.length === 0) await flush();
    const image = coordinator.requestImage("/replacement.png");

    await expect(pdf).resolves.toEqual({ status: "superseded" });
    await expect(image).resolves.toMatchObject({ status: "committed" });
    expect(dependencies.discardCandidateSession).toHaveBeenCalledWith("encrypted");
    expect(dependencies.dismissAttemptDialog).toHaveBeenCalled();
  });

  it("cleans a pre-commit candidate when close or unmount stops the coordinator", async () => {
    const password = deferred();
    const { dependencies } = createDependencies();
    dependencies.openDocumentSession.mockResolvedValueOnce({
      status: "password_required",
      sessionId: "closing-candidate",
      snapshot: null,
    });
    dependencies.requestPassword.mockReturnValue(password.promise);
    const coordinator = createDocumentOpenCoordinator(dependencies);

    const opening = coordinator.requestPdf("/closing.pdf");
    while (dependencies.requestPassword.mock.calls.length === 0) await flush();
    await expect(coordinator.stop()).resolves.toEqual({ status: "stopped" });
    await expect(opening).resolves.toMatchObject({ status: "superseded" });
    expect(dependencies.discardCandidateSession).toHaveBeenCalledWith(
      "closing-candidate",
    );
  });

  it("can resume after a cancelled window-close quiesce", async () => {
    const { dependencies } = createDependencies();
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.stop()).resolves.toEqual({ status: "stopped" });
    expect(coordinator.resume()).toBe(true);
    await expect(coordinator.requestImage("/after-cancel.png")).resolves.toMatchObject({
      status: "committed",
    });
  });

  it("enters CommitOutcomeUnknown, queues new requests, and supports retry", async () => {
    const { dependencies, setActive } = createDependencies();
    dependencies.commitCandidateActivation.mockRejectedValue(new Error("transport lost"));
    dependencies.getActiveDocumentSessionSnapshot
      .mockResolvedValueOnce(null)
      .mockRejectedValue(new Error("offline"));
    const coordinator = createDocumentOpenCoordinator(dependencies, {
      reconciliationAttempts: 2,
    });

    await expect(coordinator.requestPdf("/unknown.pdf")).resolves.toMatchObject({
      status: "commit_outcome_unknown",
    });
    const queued = coordinator.requestImage("/queued.png");
    expect(coordinator.snapshot()).toMatchObject({
      phase: "commit_outcome_unknown",
      pendingRequest: { kind: "image" },
    });
    expect(dependencies.publishCommittedImage).not.toHaveBeenCalled();

    const committed = snapshot("candidate-1", "open", { stateRevision: 3 });
    setActive(committed);
    dependencies.getActiveDocumentSessionSnapshot.mockResolvedValue(committed);
    await expect(coordinator.retryReconciliation()).resolves.toEqual({
      status: "committed",
    });
    await expect(queued).resolves.toMatchObject({ status: "committed" });
  });

  it("applies save-transition and candidate activation as one terminal command", async () => {
    const { dependencies, setActive } = createDependencies({
      initialActive: snapshot("previous", "open", { dirty: true }),
    });
    dependencies.chooseTransition.mockResolvedValue({ action: "save" });
    dependencies.saveTransition.mockImplementation(async () => {
      const committed = snapshot("candidate-1", "open", { stateRevision: 3 });
      setActive(committed);
      return committed;
    });
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestPdf("/candidate.pdf")).resolves.toMatchObject({
      status: "committed",
      snapshot: { id: "candidate-1", lifecycle: "open" },
    });
    expect(dependencies.saveTransition).toHaveBeenCalledOnce();
    expect(dependencies.commitCandidateActivation).not.toHaveBeenCalled();
    expect(dependencies.rollbackCandidateActivation).not.toHaveBeenCalled();
  });

  it("cleans a slow password-required candidate without opening a stale dialog", async () => {
    const opened = deferred();
    const { dependencies } = createDependencies();
    dependencies.openDocumentSession.mockReturnValue(opened.promise);
    const coordinator = createDocumentOpenCoordinator(dependencies);

    const pdf = coordinator.requestPdf("/slow.pdf");
    while (dependencies.openDocumentSession.mock.calls.length === 0) await flush();
    const image = coordinator.requestImage("/replacement.png");
    opened.resolve({ status: "password_required", sessionId: "slow-candidate", snapshot: null });

    await expect(pdf).resolves.toEqual({ status: "superseded" });
    await expect(image).resolves.toMatchObject({ status: "committed" });
    expect(dependencies.requestPassword).not.toHaveBeenCalled();
    expect(dependencies.discardCandidateSession).toHaveBeenCalledWith("slow-candidate");
  });

  it("does not publish an invalid-password error from a stale authentication response", async () => {
    const authentication = deferred();
    const { dependencies } = createDependencies();
    dependencies.openDocumentSession.mockResolvedValueOnce({
      status: "password_required", sessionId: "encrypted", snapshot: null,
    });
    dependencies.requestPassword.mockResolvedValue("password");
    dependencies.authenticateDocumentSession.mockReturnValue(authentication.promise);
    const coordinator = createDocumentOpenCoordinator(dependencies);

    const pdf = coordinator.requestPdf("/encrypted.pdf");
    while (dependencies.authenticateDocumentSession.mock.calls.length === 0) await flush();
    const image = coordinator.requestImage("/replacement.png");
    authentication.reject({ code: "DOCUMENT_SESSION_INVALID_PASSWORD" });

    await expect(pdf).resolves.toEqual({ status: "superseded" });
    await expect(image).resolves.toMatchObject({ status: "committed" });
    expect(dependencies.onPasswordError).not.toHaveBeenCalled();
  });

  it("does not open a stale dirty-transition dialog", async () => {
    const transition = deferred();
    const previous = snapshot("previous", "open", { dirty: true });
    const { dependencies } = createDependencies({ initialActive: previous });
    dependencies.beginSessionTransition
      .mockReturnValueOnce(transition.promise)
      .mockImplementation(async (id, kind) => ({ token: `${id}-${kind}`, snapshot: previous }));
    const coordinator = createDocumentOpenCoordinator(dependencies);

    const pdf = coordinator.requestPdf("/next.pdf");
    while (dependencies.beginSessionTransition.mock.calls.length === 0) await flush();
    const image = coordinator.requestImage("/replacement.png");
    transition.resolve({ token: "previous-document_switch", snapshot: previous });

    await expect(pdf).resolves.toEqual({ status: "superseded" });
    await expect(image).resolves.toMatchObject({ status: "committed" });
    expect(dependencies.chooseTransition).not.toHaveBeenCalledWith(
      expect.objectContaining({ attemptId: 1 }),
    );
  });

  it("uses the latest image only when the first image is superseded during preparation", async () => {
    const preparation = deferred();
    const { dependencies } = createDependencies();
    dependencies.prepareImageActivation
      .mockReturnValueOnce(preparation.promise)
      .mockResolvedValueOnce(undefined);
    const coordinator = createDocumentOpenCoordinator(dependencies);

    const first = coordinator.requestImage("/first.png");
    while (dependencies.prepareImageActivation.mock.calls.length === 0) await flush();
    const latest = coordinator.requestImage("/latest.png");
    preparation.resolve();

    await expect(first).resolves.toEqual({ status: "superseded" });
    await expect(latest).resolves.toMatchObject({ status: "committed" });
    expect(dependencies.publishCommittedImage).toHaveBeenCalledTimes(1);
    expect(dependencies.publishCommittedImage).toHaveBeenCalledWith(
      expect.objectContaining({ path: "/latest.png" }),
    );
    expect(dependencies.afterImageCommit).toHaveBeenCalledTimes(1);
  });

  it("reconciliation retry publishes a response-lost image commit exactly once", async () => {
    const previous = snapshot("previous", "open");
    const { dependencies, setActive } = createDependencies({ initialActive: previous });
    dependencies.discardTransition.mockImplementation(async () => {
      setActive(null);
      throw new Error("transport lost");
    });
    dependencies.getActiveDocumentSessionSnapshot
      .mockResolvedValueOnce(previous)
      .mockRejectedValueOnce(new Error("offline"));
    const coordinator = createDocumentOpenCoordinator(dependencies, { reconciliationAttempts: 1 });

    await expect(coordinator.requestImage("/lost.png")).resolves.toMatchObject({
      status: "commit_outcome_unknown",
    });
    dependencies.getActiveDocumentSessionSnapshot.mockResolvedValue(null);
    await expect(coordinator.retryReconciliation()).resolves.toEqual({ status: "committed" });
    expect(dependencies.publishCommittedImage).toHaveBeenCalledTimes(1);
  });

  it("keeps a committed image when viewer or secondary work fails", async () => {
    const { dependencies } = createDependencies();
    dependencies.activateCommittedImageViewer.mockRejectedValue(new Error("viewer failed"));
    dependencies.afterImageCommit.mockRejectedValue(new Error("recent failed"));
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestImage("/image.png")).resolves.toMatchObject({
      status: "committed",
    });
    expect(dependencies.publishCommittedImage).toHaveBeenCalledOnce();
    expect(dependencies.onViewerActivationError).toHaveBeenCalledOnce();
    await flush();
    expect(dependencies.onSecondaryError).toHaveBeenCalledOnce();
  });

  it("publishes backend snapshots for dirty-decision cancel and save", async () => {
    const previous = snapshot("previous", "open", { dirty: true });
    const cancelled = snapshot("previous", "open", { dirty: true, stateRevision: 4 });
    const { dependencies } = createDependencies({ initialActive: previous });
    dependencies.chooseTransition.mockResolvedValue({ action: "cancel" });
    dependencies.rollbackCandidateActivation.mockResolvedValue(cancelled);
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestPdf("/next.pdf")).resolves.toMatchObject({ status: "cancelled" });
    expect(dependencies.applyAuthoritativeActiveSnapshot).toHaveBeenCalledWith(cancelled);
    expect(dependencies.cancelTransition).not.toHaveBeenCalled();
  });

  it("blocks pending opens on cleanup failure and drains the latest request after recovery", async () => {
    const opened = deferred();
    const { dependencies } = createDependencies();
    dependencies.openDocumentSession.mockReturnValue(opened.promise);
    dependencies.discardCandidateSession.mockRejectedValueOnce(new Error("cleanup failed"));
    const coordinator = createDocumentOpenCoordinator(dependencies);

    const pdf = coordinator.requestPdf("/broken.pdf");
    while (dependencies.openDocumentSession.mock.calls.length === 0) await flush();
    const image = coordinator.requestImage("/latest.png");
    opened.resolve({ status: "password_required", sessionId: "broken-candidate", snapshot: null });

    await expect(pdf).resolves.toMatchObject({ status: "precommit_recovery_required" });
    expect(coordinator.snapshot()).toMatchObject({ phase: "precommit_recovery_required" });
    expect(dependencies.publishCommittedImage).not.toHaveBeenCalled();
    await expect(coordinator.retryPrecommitRecovery()).resolves.toEqual({ status: "recovered" });
    await expect(image).resolves.toMatchObject({ status: "committed" });
  });

  it("reconciles a response-lost terminal switch without running candidate commit again", async () => {
    const previous = snapshot("previous", "open");
    const committed = snapshot("candidate-1", "open", { stateRevision: 3 });
    const { dependencies, setActive } = createDependencies({ initialActive: previous });
    dependencies.discardTransition.mockImplementation(async () => {
      setActive(committed);
      throw new Error("response lost");
    });
    dependencies.getActiveDocumentSessionSnapshot
      .mockResolvedValueOnce(previous)
      .mockResolvedValue(committed);
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestPdf("/candidate.pdf")).resolves.toMatchObject({
      status: "committed",
      snapshot: { id: "candidate-1" },
    });
    expect(dependencies.rollbackCandidateActivation).not.toHaveBeenCalled();
    expect(dependencies.commitCandidateActivation).not.toHaveBeenCalled();
    expect(dependencies.applyAuthoritativeActiveSnapshot).toHaveBeenCalledWith(committed);
  });

  it("shares one cleanup sequence between stop and the superseded request", async () => {
    const password = deferred();
    const { dependencies } = createDependencies();
    dependencies.openDocumentSession.mockResolvedValueOnce({
      status: "password_required", sessionId: "once", snapshot: null,
    });
    dependencies.requestPassword.mockReturnValue(password.promise);
    const coordinator = createDocumentOpenCoordinator(dependencies);

    const opening = coordinator.requestPdf("/once.pdf");
    while (dependencies.requestPassword.mock.calls.length === 0) await flush();
    await coordinator.stop();
    await expect(opening).resolves.toMatchObject({ status: "superseded" });
    expect(dependencies.discardCandidateSession).toHaveBeenCalledTimes(1);
  });

  it("shares one bounded reconciliation flight across concurrent retries", async () => {
    const previous = snapshot("previous", "open");
    const authoritative = deferred();
    const { dependencies, setActive } = createDependencies({ initialActive: previous });
    dependencies.discardTransition.mockImplementation(async () => {
      setActive(null);
      throw new Error("transport lost");
    });
    dependencies.getActiveDocumentSessionSnapshot
      .mockResolvedValueOnce(previous)
      .mockRejectedValueOnce(new Error("offline"));
    const coordinator = createDocumentOpenCoordinator(dependencies, {
      reconciliationAttempts: 1,
    });
    await expect(coordinator.requestImage("/single-flight.png")).resolves.toMatchObject({
      status: "commit_outcome_unknown",
    });
    dependencies.getActiveDocumentSessionSnapshot.mockReturnValue(authoritative.promise);

    const first = coordinator.retryReconciliation();
    const second = coordinator.retryReconciliation();
    expect(first).toBe(second);
    expect(dependencies.getActiveDocumentSessionSnapshot).toHaveBeenCalledTimes(3);
    authoritative.resolve(null);

    await expect(first).resolves.toEqual({ status: "committed" });
    await expect(second).resolves.toEqual({ status: "committed" });
    expect(dependencies.publishCommittedImage).toHaveBeenCalledTimes(1);
    expect(dependencies.activateCommittedImageViewer).toHaveBeenCalledTimes(1);
  });

  it("keeps persistent offline reconciliation bounded without recursive retries", async () => {
    const { dependencies } = createDependencies();
    dependencies.commitCandidateActivation.mockRejectedValue(new Error("transport lost"));
    dependencies.getActiveDocumentSessionSnapshot
      .mockResolvedValueOnce(null)
      .mockRejectedValue(new Error("offline"));
    const coordinator = createDocumentOpenCoordinator(dependencies, {
      reconciliationAttempts: 2,
    });

    await expect(coordinator.requestPdf("/offline.pdf")).resolves.toMatchObject({
      status: "commit_outcome_unknown",
    });
    const callsBeforeRetry = dependencies.getActiveDocumentSessionSnapshot.mock.calls.length;
    await expect(coordinator.retryReconciliation()).resolves.toMatchObject({
      status: "commit_outcome_unknown",
    });
    await flush();

    expect(dependencies.getActiveDocumentSessionSnapshot).toHaveBeenCalledTimes(
      callsBeforeRetry + 2,
    );
    expect(coordinator.snapshot().phase).toBe("commit_outcome_unknown");
    expect(dependencies.commitDocumentSnapshot).not.toHaveBeenCalled();
  });

  it("finalizes a committed PDF once when close and UI retries race", async () => {
    const authoritative = deferred();
    const { dependencies, setActive } = createDependencies();
    dependencies.commitCandidateActivation.mockRejectedValue(new Error("response lost"));
    dependencies.getActiveDocumentSessionSnapshot
      .mockResolvedValueOnce(null)
      .mockRejectedValueOnce(new Error("offline"));
    const coordinator = createDocumentOpenCoordinator(dependencies, {
      reconciliationAttempts: 1,
    });
    await coordinator.requestPdf("/race.pdf");
    dependencies.getActiveDocumentSessionSnapshot.mockReturnValue(authoritative.promise);

    const automaticRetry = coordinator.retryReconciliation();
    const closeRetry = coordinator.retryReconciliation();
    const committed = snapshot("candidate-1", "open", { stateRevision: 3 });
    setActive(committed);
    authoritative.resolve(committed);
    await Promise.all([automaticRetry, closeRetry]);

    expect(dependencies.commitDocumentSnapshot).toHaveBeenCalledTimes(1);
    expect(dependencies.activateCommittedPdfRender).toHaveBeenCalledTimes(1);
  });

  it("reconciles authoritative image publication failure before viewer activation", async () => {
    const { dependencies } = createDependencies();
    dependencies.publishCommittedImage.mockImplementation(() => {
      throw new Error("publication failed");
    });
    const coordinator = createDocumentOpenCoordinator(dependencies);

    await expect(coordinator.requestImage("/reconcile.png")).resolves.toMatchObject({
      status: "committed",
    });
    expect(dependencies.reconcileCommittedImage).toHaveBeenCalledWith(
      expect.objectContaining({ path: "/reconcile.png" }),
      expect.any(Error),
    );
    expect(dependencies.activateCommittedImageViewer).toHaveBeenCalledTimes(1);
  });

  it("single-flights recovery retry and keeps state blocked after another failure", async () => {
    const opened = deferred();
    const recovery = deferred();
    const { dependencies } = createDependencies();
    dependencies.openDocumentSession.mockReturnValue(opened.promise);
    dependencies.discardCandidateSession
      .mockRejectedValueOnce(new Error("cleanup failed"))
      .mockReturnValueOnce(recovery.promise);
    const coordinator = createDocumentOpenCoordinator(dependencies);
    const pdf = coordinator.requestPdf("/broken.pdf");
    while (dependencies.openDocumentSession.mock.calls.length === 0) await flush();
    const pending = coordinator.requestImage("/pending.png");
    opened.resolve({ status: "password_required", sessionId: "broken", snapshot: null });
    await expect(pdf).resolves.toMatchObject({ status: "precommit_recovery_required" });

    const first = coordinator.retryPrecommitRecovery();
    const second = coordinator.retryPrecommitRecovery();
    expect(first).toBe(second);
    expect(dependencies.discardCandidateSession).toHaveBeenCalledTimes(2);
    recovery.resolve();
    await expect(first).resolves.toEqual({ status: "recovered" });
    await expect(pending).resolves.toMatchObject({ status: "committed" });

    const failedAgain = deferred();
    dependencies.openDocumentSession.mockReturnValueOnce(failedAgain.promise);
    dependencies.discardCandidateSession.mockRejectedValue(new Error("still failed"));
    const nextPdf = coordinator.requestPdf("/broken-again.pdf");
    while (dependencies.openDocumentSession.mock.calls.length < 2) await flush();
    coordinator.requestImage("/blocked.png");
    failedAgain.resolve({ status: "password_required", sessionId: "broken-again", snapshot: null });
    await expect(nextPdf).resolves.toMatchObject({ status: "precommit_recovery_required" });
    await expect(coordinator.retryPrecommitRecovery()).resolves.toMatchObject({
      status: "precommit_recovery_required",
    });
    expect(coordinator.snapshot().phase).toBe("precommit_recovery_required");
  });
});

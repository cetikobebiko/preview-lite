function deferred() {
  let resolve;
  const promise = new Promise((resolver) => {
    resolve = resolver;
  });
  return { promise, resolve };
}

function errorCode(error) {
  return error?.cause?.code ?? error?.code ?? "";
}

export class CommitOutcomeUnknownError extends Error {
  constructor() {
    super("Backend commit sonucu doğrulanamadı.");
    this.name = "CommitOutcomeUnknownError";
    this.code = "COMMIT_OUTCOME_UNKNOWN";
  }
}

export class PrecommitRecoveryRequiredError extends Error {
  constructor(error) {
    super("Commit öncesi session temizliği doğrulanamadı.");
    this.name = "PrecommitRecoveryRequiredError";
    this.code = "PRECOMMIT_RECOVERY_REQUIRED";
    this.cause = error;
  }
}

export function createDocumentOpenCoordinator(dependencies, options = {}) {
  const reconciliationAttempts = options.reconciliationAttempts ?? 3;
  let nextAttemptId = 0;
  let current = null;
  let pendingLatest = null;
  let commitOutcomeUnknown = null;
  let precommitRecovery = null;
  let reconciliationPromise = null;
  let recoveryPromise = null;
  let stopped = false;

  const notify = () =>
    dependencies.onStateChange?.({
      attemptId: current?.id ?? null,
      phase: commitOutcomeUnknown
        ? "commit_outcome_unknown"
        : precommitRecovery
          ? "precommit_recovery_required"
          : current?.phase ?? "idle",
      hasPendingRequest: Boolean(pendingLatest),
      commitFence: Boolean(current?.commitFence),
    });

  function applyAuthoritativeActiveSnapshot(snapshot) {
    if (snapshot) dependencies.applyAuthoritativeActiveSnapshot?.(snapshot);
  }

  function createRequest(kind, path) {
    const completion = deferred();
    return { id: ++nextAttemptId, kind, path, completion };
  }

  function supersedePending(request) {
    if (pendingLatest) pendingLatest.completion.resolve({ status: "superseded" });
    pendingLatest = request;
    notify();
  }

  function cancelCurrentFor(request) {
    supersedePending(request);
    if (current && !current.commitFence) {
      current.superseded = true;
      current.cancel.resolve({ cancelled: true });
      dependencies.dismissAttemptDialog?.(current.id);
    }
  }

  function enqueue(kind, path) {
    const request = createRequest(kind, path);
    if (stopped) {
      request.completion.resolve({ status: "stopped" });
    } else if (commitOutcomeUnknown || precommitRecovery || current) {
      cancelCurrentFor(request);
    } else {
      void runRequest(request);
    }
    return request.completion.promise;
  }

  async function drainPending() {
    if (current || commitOutcomeUnknown || precommitRecovery || stopped || !pendingLatest) return;
    const request = pendingLatest;
    pendingLatest = null;
    await runRequest(request);
  }

  function isCurrent(context) {
    return current === context && !context.superseded && !stopped && !precommitRecovery;
  }

  async function awaitOwned(context, promise) {
    const result = await Promise.race([
      Promise.resolve(promise).then((value) => ({ value })),
      context.cancel.promise,
    ]);
    return result?.cancelled ? { cancelled: true } : { value: result.value };
  }

  async function rollbackOrDiscardExpiredActivation(context) {
    try {
      const restored = await dependencies.rollbackCandidateActivation(context.activationToken);
      applyAuthoritativeActiveSnapshot(restored);
      context.activationToken = null;
      context.transitionToken = null;
      context.candidateId = null;
      return;
    } catch (error) {
      const candidate = context.candidateId
        ? await dependencies.getDocumentSessionSnapshot(context.candidateId)
        : null;
      if (candidate?.lifecycle !== "candidate_ready") throw error;
      await dependencies.discardCandidateSession(context.candidateId);
      context.candidateId = null;
      context.activationToken = null;
      context.transitionToken = null;
      const active = await dependencies.getActiveDocumentSessionSnapshot();
      applyAuthoritativeActiveSnapshot(active);
    }
  }

  async function performCleanup(context) {
    if (context.activationToken) await rollbackOrDiscardExpiredActivation(context);
    if (context.candidateId) {
      await dependencies.discardCandidateSession(context.candidateId);
      context.candidateId = null;
    }
    if (context.transitionToken) {
      const snapshot = await dependencies.cancelTransition(context.transitionToken);
      applyAuthoritativeActiveSnapshot(snapshot);
      context.transitionToken = null;
    }
  }

  async function cleanupBeforeCommit(context) {
    if (context.cleanupPromise) return context.cleanupPromise;
    context.cleanupError = null;
    const cleanup = performCleanup(context);
    context.cleanupPromise = cleanup;
    try {
      await cleanup;
    } catch (error) {
      context.cleanupPromise = null;
      context.cleanupError = error;
      throw error;
    }
  }

  function enterPrecommitRecovery(context, error) {
    precommitRecovery = context;
    context.phase = "precommit_recovery_required";
    dependencies.onPrecommitRecoveryRequired?.({ attemptId: context.id, error });
    notify();
  }

  async function prepareTransition(context, activeSnapshot, kind, candidateSessionId = null) {
    if (!activeSnapshot) return null;
    const transition = await dependencies.beginSessionTransition(activeSnapshot.id, kind);
    context.transitionToken = transition.token;
    applyAuthoritativeActiveSnapshot(transition.snapshot);
    if (!isCurrent(context)) return null;
    if (kind === "document_switch") {
      const prepared = await dependencies.prepareCandidateActivation(
        candidateSessionId,
        activeSnapshot.id,
        transition.token,
      );
      context.activationToken = prepared.activationToken;
      if (!isCurrent(context)) return null;
    }
    const reconcileTerminalTransition = async (error) => {
      for (let attempt = 0; attempt < reconciliationAttempts; attempt += 1) {
        try {
          const active = await dependencies.getActiveDocumentSessionSnapshot();
          if (kind === "document_switch" && active?.id === candidateSessionId) return active;
          if (kind !== "document_switch" && active === null) {
            return error?.cause?.snapshot ?? error?.snapshot ?? transition.snapshot;
          }
          if (active?.id === activeSnapshot.id) return null;
        } catch {
          // A later bounded attempt may recover the authoritative terminal result.
        }
      }
      throw new CommitOutcomeUnknownError();
    };
    let decision = { action: "discard" };
    if (transition.snapshot.dirty) {
      if (!isCurrent(context)) return null;
      const ownedDecision = await awaitOwned(
        context,
        dependencies.chooseTransition({
          attemptId: context.id,
          kind,
          snapshot: transition.snapshot,
        }),
      );
      if (ownedDecision.cancelled || !isCurrent(context)) return null;
      decision = ownedDecision.value;
    }
    while (decision?.action === "save" || decision?.action === "save_as") {
      try {
        const snapshot = await dependencies.saveTransition(
          transition.token,
          decision.action === "save_as" ? decision.targetPath : null,
        );
        if (kind === "document_switch") applyAuthoritativeActiveSnapshot(snapshot);
        context.transitionSnapshot = snapshot;
        context.activationToken = null;
        return transition.token;
      } catch (error) {
        const reconciled = await reconcileTerminalTransition(error);
        if (reconciled) {
          if (kind === "document_switch") applyAuthoritativeActiveSnapshot(reconciled);
          context.transitionSnapshot = reconciled;
          context.activationToken = null;
          return transition.token;
        }
        if (!isCurrent(context)) return null;
        const ownedRecovery = await awaitOwned(
          context,
          dependencies.onTransitionSaveError?.({
            attemptId: context.id,
            error,
            snapshot: transition.snapshot,
          }),
        );
        if (ownedRecovery.cancelled || !isCurrent(context)) return null;
        decision = ownedRecovery.value ?? { action: "cancel" };
      }
    }
    if (decision?.action === "discard") {
      let snapshot;
      try {
        snapshot = await dependencies.discardTransition(transition.token);
      } catch (error) {
        snapshot = await reconcileTerminalTransition(error);
        if (!snapshot) throw error;
      }
      if (kind === "document_switch") applyAuthoritativeActiveSnapshot(snapshot);
      context.transitionSnapshot = snapshot;
      context.activationToken = null;
      return transition.token;
    }
    if (context.activationToken) {
      const snapshot = await dependencies.rollbackCandidateActivation(context.activationToken);
      applyAuthoritativeActiveSnapshot(snapshot);
      context.activationToken = null;
    } else {
      const snapshot = await dependencies.cancelTransition(transition.token);
      applyAuthoritativeActiveSnapshot(snapshot);
    }
    context.transitionToken = null;
    return null;
  }

  async function reconcilePdfCommit(context) {
    for (let attempt = 0; attempt < reconciliationAttempts; attempt += 1) {
      try {
        const active = await dependencies.getActiveDocumentSessionSnapshot();
        if (active?.id === context.candidateId && active.lifecycle === "open") {
          return { committed: true, snapshot: active, cleanupWarnings: [] };
        }
        const candidate = await dependencies.getDocumentSessionSnapshot(context.candidateId);
        if (active?.id !== context.candidateId && candidate?.lifecycle === "candidate_activation_prepared") {
          const restored = await dependencies.rollbackCandidateActivation(context.activationToken);
          applyAuthoritativeActiveSnapshot(restored);
          context.activationToken = null;
          context.candidateId = null;
          context.transitionToken = null;
          return { committed: false };
        }
        if (active?.id !== context.candidateId && candidate?.lifecycle === "candidate_ready") {
          await dependencies.discardCandidateSession(context.candidateId);
          applyAuthoritativeActiveSnapshot(active);
          context.activationToken = null;
          context.candidateId = null;
          context.transitionToken = null;
          return { committed: false };
        }
      } catch {
        // The next bounded authoritative snapshot attempt decides the outcome.
      }
    }
    throw new CommitOutcomeUnknownError();
  }

  async function reconcileImageCommit(context) {
    for (let attempt = 0; attempt < reconciliationAttempts; attempt += 1) {
      try {
        const active = await dependencies.getActiveDocumentSessionSnapshot();
        if (active === null) return { committed: true };
        if (active?.id === context.previousSessionId) {
          const snapshot = await dependencies.cancelTransition(context.transitionToken);
          applyAuthoritativeActiveSnapshot(snapshot ?? active);
          context.transitionToken = null;
          return { committed: false };
        }
      } catch {
        // Retry until the bounded reconciliation budget is exhausted.
      }
    }
    throw new CommitOutcomeUnknownError();
  }

  async function finalizeCommittedPdf(context, result, probe) {
    if (context.finalizationPromise) return context.finalizationPromise;
    context.finalizationPromise = finalizeCommittedPdfOnce(context, result, probe);
    return context.finalizationPromise;
  }

  async function finalizeCommittedPdfOnce(context, result, probe) {
    const snapshot = result.snapshot;
    try {
      dependencies.commitDocumentSnapshot(snapshot);
    } catch (error) {
      await dependencies.reconcileCommittedDocumentSnapshot?.(snapshot, error);
      dependencies.onPostCommitReconcileError?.({ snapshot, error });
    }
    try {
      await dependencies.activateCommittedPdfRender(snapshot);
    } catch (error) {
      dependencies.onViewerActivationError?.({ kind: "pdf", snapshot, error });
    }
    dependencies.onCleanupWarnings?.(result.cleanupWarnings ?? []);
    void Promise.resolve(dependencies.afterPdfCommit?.({ snapshot, pageCount: probe.pageCount }))
      .catch((error) => dependencies.onSecondaryError?.(error));
  }

  async function finalizeCommittedImage(context) {
    if (context.finalizationPromise) return context.finalizationPromise;
    context.finalizationPromise = finalizeCommittedImageOnce(context);
    return context.finalizationPromise;
  }

  async function finalizeCommittedImageOnce(context) {
    const payload = { path: context.path, decoded: context.decoded };
    try {
      dependencies.publishCommittedImage(payload);
    } catch (error) {
      await dependencies.reconcileCommittedImage(payload, error);
    }
    try {
      await dependencies.activateCommittedImageViewer(payload);
    } catch (error) {
      dependencies.onViewerActivationError?.({ kind: "image", ...payload, error });
    }
    void Promise.resolve(dependencies.afterImageCommit?.(payload))
      .catch((error) => dependencies.onSecondaryError?.(error));
  }

  async function runPdf(context) {
    const previous = await dependencies.getActiveDocumentSessionSnapshot();
    context.previousSessionId = previous?.id ?? null;
    const opened = await dependencies.openDocumentSession(context.path);
    context.candidateId = opened.sessionId;
    if (!isCurrent(context)) {
      await cleanupBeforeCommit(context);
      return { status: "superseded" };
    }
    let candidate = opened.snapshot;
    while (opened.status === "password_required" && !candidate) {
      if (!isCurrent(context)) {
        await cleanupBeforeCommit(context);
        return { status: "superseded" };
      }
      context.phase = "awaiting_password";
      notify();
      const passwordResult = await awaitOwned(
        context,
        dependencies.requestPassword({ attemptId: context.id, path: context.path }),
      );
      if (passwordResult.cancelled || !passwordResult.value || !isCurrent(context)) {
        await cleanupBeforeCommit(context);
        return { status: context.superseded ? "superseded" : "cancelled" };
      }
      try {
        candidate = await dependencies.authenticateDocumentSession(
          context.candidateId,
          passwordResult.value,
        );
        if (!isCurrent(context)) {
          await cleanupBeforeCommit(context);
          return { status: "superseded" };
        }
      } catch (error) {
        if (errorCode(error) !== "DOCUMENT_SESSION_INVALID_PASSWORD") throw error;
        if (!isCurrent(context)) {
          await cleanupBeforeCommit(context);
          return { status: "superseded" };
        }
        dependencies.onPasswordError?.({ attemptId: context.id, error });
      }
    }
    if (!isCurrent(context)) {
      await cleanupBeforeCommit(context);
      return { status: "superseded" };
    }
    context.phase = "validating_candidate";
    notify();
    const probe = await dependencies.probeCandidateRender(context.candidateId, candidate.contentGeneration);
    if (!Number.isInteger(probe.pageCount) || probe.pageCount < 1 ||
      !Number.isInteger(probe.width) || probe.width < 1 ||
      !Number.isInteger(probe.height) || probe.height < 1 ||
      probe.byteLength !== probe.width * probe.height * 4) {
      throw new Error("CANDIDATE_RENDER_PROBE_INVALID");
    }
    context.probe = probe;
    if (!isCurrent(context)) {
      await cleanupBeforeCommit(context);
      return { status: "superseded" };
    }
    context.phase = "awaiting_dirty_decision";
    notify();
    const transitionToken = await prepareTransition(
      context,
      previous,
      "document_switch",
      context.candidateId,
    );
    if (previous && !transitionToken) {
      await cleanupBeforeCommit(context);
      return { status: context.superseded ? "superseded" : "cancelled" };
    }
    if (!isCurrent(context)) {
      await cleanupBeforeCommit(context);
      return { status: "superseded" };
    }
    let result;
    if (previous) {
      context.phase = "commit_dispatching";
      context.commitFence = true;
      notify();
      result = { snapshot: context.transitionSnapshot, cleanupWarnings: [] };
    } else {
      const prepared = await dependencies.prepareCandidateActivation(
        context.candidateId,
        null,
        null,
      );
      context.activationToken = prepared.activationToken;
      if (!isCurrent(context)) {
        await cleanupBeforeCommit(context);
        return { status: "superseded" };
      }
      context.phase = "commit_dispatching";
      context.commitFence = true;
      notify();
      try {
        result = await dependencies.commitCandidateActivation(context.activationToken);
      } catch {
        context.phase = "reconciling_commit";
        notify();
        result = await reconcilePdfCommit(context);
        if (!result.committed) return { status: "rolled_back" };
      }
    }
    context.activationToken = null;
    context.transitionToken = null;
    context.candidateId = null;
    await finalizeCommittedPdf(context, result, context.probe);
    return { status: "committed", snapshot: result.snapshot };
  }

  async function runImage(context) {
    context.phase = "validating_image";
    notify();
    context.decoded = await dependencies.readImageFile(context.path, 0, false, false);
    await dependencies.prepareImage?.({ path: context.path, decoded: context.decoded });
    if (!isCurrent(context)) return { status: "superseded" };
    const previous = await dependencies.getActiveDocumentSessionSnapshot();
    context.previousSessionId = previous?.id ?? null;
    await dependencies.prepareImageActivation?.({ path: context.path, decoded: context.decoded, previous });
    if (!isCurrent(context)) {
      await cleanupBeforeCommit(context);
      return { status: "superseded" };
    }
    if (previous) {
      const transitionToken = await prepareTransition(context, previous, "document_close");
      if (!transitionToken) return { status: context.superseded ? "superseded" : "cancelled" };
    }
    if (!isCurrent(context)) {
      await cleanupBeforeCommit(context);
      return { status: "superseded" };
    }
    context.phase = "commit_dispatching";
    context.commitFence = true;
    notify();
    if (previous) context.transitionToken = null;
    await finalizeCommittedImage(context);
    return { status: "committed", path: context.path };
  }

  async function runRequest(request) {
    const context = {
      ...request,
      cancel: deferred(), superseded: false, commitFence: false, phase: "starting",
      candidateId: null, activationToken: null, transitionToken: null,
      previousSessionId: null, decoded: null, cleanupPromise: null, cleanupError: null,
      transitionSnapshot: null,
      finalizationPromise: null,
    };
    current = context;
    notify();
    try {
      request.completion.resolve(request.kind === "pdf" ? await runPdf(context) : await runImage(context));
    } catch (error) {
      if (error instanceof CommitOutcomeUnknownError) {
        commitOutcomeUnknown = context;
        dependencies.onCommitOutcomeUnknown?.({ attemptId: context.id, error });
        request.completion.resolve({ status: "commit_outcome_unknown", error });
        return;
      }
      if (!context.commitFence) {
        if (context.cleanupError) {
          enterPrecommitRecovery(context, context.cleanupError);
          request.completion.resolve({
            status: "precommit_recovery_required",
            error: new PrecommitRecoveryRequiredError(context.cleanupError),
          });
          return;
        }
        try {
          await cleanupBeforeCommit(context);
        } catch (cleanupError) {
          enterPrecommitRecovery(context, cleanupError);
          request.completion.resolve({
            status: "precommit_recovery_required",
            error: new PrecommitRecoveryRequiredError(cleanupError),
          });
          return;
        }
      }
      dependencies.onError?.({ attemptId: context.id, error });
      request.completion.resolve({ status: "failed", error });
    } finally {
      if (!commitOutcomeUnknown && !precommitRecovery && current === context) {
        current = null;
        dependencies.dismissAttemptDialog?.(context.id);
        notify();
        void drainPending();
      }
    }
  }

  function retryPrecommitRecovery() {
    if (recoveryPromise) return recoveryPromise;
    const context = precommitRecovery;
    if (!context) return Promise.resolve({ status: "idle" });
    const task = (async () => {
      try {
        await cleanupBeforeCommit(context);
        precommitRecovery = null;
        if (current === context) current = null;
        dependencies.dismissAttemptDialog?.(context.id);
        notify();
        void drainPending();
        return { status: "recovered" };
      } catch (error) {
        dependencies.onPrecommitRecoveryRequired?.({ attemptId: context.id, error });
        return {
          status: "precommit_recovery_required",
          error: new PrecommitRecoveryRequiredError(error),
        };
      }
    })();
    recoveryPromise = task;
    task.then(
      () => {
        if (recoveryPromise === task) recoveryPromise = null;
      },
      () => {
        if (recoveryPromise === task) recoveryPromise = null;
      },
    );
    return task;
  }

  function retryReconciliation() {
    if (reconciliationPromise) return reconciliationPromise;
    const context = commitOutcomeUnknown;
    if (!context) return Promise.resolve({ status: "idle" });
    const task = (async () => {
      try {
        const result = context.kind === "pdf"
          ? await reconcilePdfCommit(context)
          : await reconcileImageCommit(context);
        if (result.committed) {
          if (context.kind === "pdf") {
            await finalizeCommittedPdf(context, result, context.probe);
          } else {
            await finalizeCommittedImage(context);
          }
          commitOutcomeUnknown = null;
          current = null;
          notify();
          void drainPending();
          return { status: "committed" };
        }
        commitOutcomeUnknown = null;
        current = null;
        notify();
        void drainPending();
        return { status: "rolled_back" };
      } catch (error) {
        dependencies.onCommitOutcomeUnknown?.({ attemptId: context.id, error });
        return { status: "commit_outcome_unknown", error };
      }
    })();
    reconciliationPromise = task;
    task.then(
      () => {
        if (reconciliationPromise === task) reconciliationPromise = null;
      },
      () => {
        if (reconciliationPromise === task) reconciliationPromise = null;
      },
    );
    return task;
  }

  async function stop() {
    stopped = true;
    if (pendingLatest) {
      pendingLatest.completion.resolve({ status: "stopped" });
      pendingLatest = null;
    }
    if (current?.commitFence && !commitOutcomeUnknown) await current.completion.promise;
    if (commitOutcomeUnknown) {
      notify();
      return { status: "commit_outcome_unknown" };
    }
    if (precommitRecovery) {
      notify();
      return { status: "precommit_recovery_required" };
    }
    if (current && !current.commitFence) {
      current.superseded = true;
      current.cancel.resolve({ cancelled: true });
      dependencies.dismissAttemptDialog?.(current.id);
      try {
        await cleanupBeforeCommit(current);
      } catch (error) {
        enterPrecommitRecovery(current, error);
        return { status: "precommit_recovery_required", error: new PrecommitRecoveryRequiredError(error) };
      }
    }
    notify();
    return { status: "stopped" };
  }

  function resume() {
    if (commitOutcomeUnknown || precommitRecovery) return false;
    stopped = false;
    notify();
    void drainPending();
    return true;
  }

  return Object.freeze({
    requestPdf: (path) => enqueue("pdf", path),
    requestImage: (path) => enqueue("image", path),
    retryReconciliation,
    retryPrecommitRecovery,
    resume,
    stop,
    snapshot: () => ({
      attemptId: current?.id ?? null,
      phase: commitOutcomeUnknown ? "commit_outcome_unknown" : precommitRecovery ? "precommit_recovery_required" : current?.phase ?? "idle",
      pendingRequest: pendingLatest ? { id: pendingLatest.id, kind: pendingLatest.kind, path: pendingLatest.path } : null,
      commitFence: Boolean(current?.commitFence),
    }),
  });
}

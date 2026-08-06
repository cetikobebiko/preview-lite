export const DOCUMENT_SESSION_CONTEXT = Symbol("document-session");

function isSnapshot(value) {
  return Boolean(
    value &&
      typeof value.id === "string" &&
      Number.isSafeInteger(value.contentGeneration) &&
      Number.isSafeInteger(value.stateRevision),
  );
}

export function createDocumentSessionStore() {
  const state = $state({
    sessions: {},
    activeSessionId: null,
    candidateSessionId: null,
    transitionEpoch: 0,
    tombstones: {},
    savingSessionIds: {},
  });

  function applySnapshot(snapshot) {
    if (!isSnapshot(snapshot) || state.tombstones[snapshot?.id]) return false;
    const current = state.sessions[snapshot.id];
    if (
      current &&
      Number.isSafeInteger(current.stateRevision) &&
      snapshot.stateRevision <= current.stateRevision
    ) {
      return false;
    }
    state.sessions[snapshot.id] = snapshot;
    return true;
  }

  function stageCandidate(snapshot) {
    if (!applySnapshot(snapshot) && state.sessions[snapshot?.id] !== snapshot) {
      const current = state.sessions[snapshot?.id];
      if (!current || current.stateRevision !== snapshot?.stateRevision) return false;
    }
    state.candidateSessionId = snapshot.id;
    return true;
  }

  function commitCandidate(snapshot) {
    if (!stageCandidate(snapshot)) return false;
    const previous = state.activeSessionId;
    if (previous && previous !== snapshot.id) {
      delete state.sessions[previous];
      state.tombstones[previous] = true;
    }
    state.activeSessionId = snapshot.id;
    state.candidateSessionId = null;
    if (previous !== snapshot.id) state.transitionEpoch += 1;
    return true;
  }

  function closeSession(sessionId) {
    if (typeof sessionId !== "string" || !sessionId) return false;
    const wasActive = state.activeSessionId === sessionId;
    delete state.sessions[sessionId];
    state.tombstones[sessionId] = true;
    delete state.savingSessionIds[sessionId];
    if (state.candidateSessionId === sessionId) state.candidateSessionId = null;
    if (wasActive) {
      state.activeSessionId = null;
      state.transitionEpoch += 1;
    }
    return wasActive;
  }

  function captureGuard(options = {}) {
    const {
      includeTransitionEpoch = true,
      includeContentGeneration = false,
      includeStateRevision = false,
      targetSessionId = null,
    } = options;
    const sessionId = targetSessionId ?? state.activeSessionId;
    const session = sessionId ? state.sessions[sessionId] ?? null : null;
    return {
      sessionId,
      transitionEpoch: includeTransitionEpoch ? state.transitionEpoch : null,
      contentGeneration: includeContentGeneration && session ? session.contentGeneration : null,
      stateRevision: includeStateRevision && session ? session.stateRevision : null,
    };
  }

  function isGuardCurrent(guard) {
    if (!guard || !guard.sessionId) return false;
    if (state.tombstones[guard.sessionId]) return false;
    if (guard.sessionId !== state.activeSessionId) return false;
    if (guard.transitionEpoch !== null && guard.transitionEpoch !== state.transitionEpoch) {
      return false;
    }
    const currentSession = state.sessions[guard.sessionId];
    if (!currentSession) return false;
    if (
      guard.contentGeneration !== null &&
      guard.contentGeneration !== currentSession.contentGeneration
    ) {
      return false;
    }
    if (
      guard.stateRevision !== null &&
      guard.stateRevision !== currentSession.stateRevision
    ) {
      return false;
    }
    return true;
  }

  function isSaveInProgress(sessionId) {
    if (!sessionId) return false;
    return Boolean(state.savingSessionIds[sessionId]);
  }

  function startSave(sessionId) {
    if (!sessionId || isSaveInProgress(sessionId)) return false;
    state.savingSessionIds[sessionId] = true;
    return true;
  }

  function finishSave(sessionId) {
    if (!sessionId) return;
    delete state.savingSessionIds[sessionId];
  }

  function resetForTests() {
    state.sessions = {};
    state.activeSessionId = null;
    state.candidateSessionId = null;
    state.transitionEpoch = 0;
    state.tombstones = {};
    state.savingSessionIds = {};
  }

  return {
    get active() {
      return state.activeSessionId
        ? state.sessions[state.activeSessionId] ?? null
        : null;
    },
    get candidate() {
      return state.candidateSessionId
        ? state.sessions[state.candidateSessionId] ?? null
        : null;
    },
    get activeSessionId() {
      return state.activeSessionId;
    },
    get transitionEpoch() {
      return state.transitionEpoch;
    },
    get dirty() {
      return this.active?.dirty === true;
    },
    get contentGeneration() {
      return this.active?.contentGeneration ?? 0;
    },
    get stateRevision() {
      return this.active?.stateRevision ?? 0;
    },
    applySnapshot,
    stageCandidate,
    commitCandidate,
    closeSession,
    captureGuard,
    isGuardCurrent,
    isSaveInProgress,
    startSave,
    finishSave,
    resetForTests,
  };
}

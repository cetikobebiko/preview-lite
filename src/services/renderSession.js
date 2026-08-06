export const RENDER_SESSION_CONTEXT = Symbol("render-session");

let fallbackIdSequence = 0;

export function createOpaqueId(prefix = "request") {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  if (randomUuid) return `${prefix}:${randomUuid}`;
  fallbackIdSequence += 1;
  return `${prefix}:${Date.now().toString(36)}:${fallbackIdSequence.toString(36)}`;
}

function frozenSnapshot(sessionId, contentGeneration) {
  return Object.freeze({ sessionId, contentGeneration });
}

export function createRenderSession() {
  let current = frozenSnapshot(null, 0);

  return Object.freeze({
    snapshot() {
      return current;
    },
    activate(sessionId, contentGeneration) {
      current = frozenSnapshot(sessionId, contentGeneration);
      return current;
    },
    clear(contentGeneration) {
      current = frozenSnapshot(null, contentGeneration);
      return current;
    },
  });
}

export function requireActiveRenderSession(
  session,
  sessionId,
  contentGeneration,
) {
  const snapshot = session?.snapshot?.();
  if (
    !snapshot ||
    typeof snapshot.sessionId !== "string" ||
    snapshot.sessionId !== sessionId ||
    snapshot.contentGeneration !== contentGeneration
  ) {
    throw new Error("render_inactive_document");
  }
  return snapshot;
}

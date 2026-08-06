import { describe, expect, it } from "vitest";
import { createDocumentSessionStore } from "./documentSession.svelte.js";

const snapshot = (id, stateRevision, contentGeneration = 1, dirty = false) => ({
  id,
  displayPath: `/${id}.pdf`,
  fileName: `${id}.pdf`,
  contentGeneration,
  stateRevision,
  dirty,
  lifecycle: "open",
  editable: true,
  readOnlyReason: null,
  healthWarning: null,
});

describe("document session store", () => {
  it("does not change transition epoch for same-session mutation/save snapshots", () => {
    const store = createDocumentSessionStore();
    store.commitCandidate(snapshot("a", 1));
    const epoch = store.transitionEpoch;

    expect(store.applySnapshot(snapshot("a", 2, 2, true))).toBe(true);
    expect(store.transitionEpoch).toBe(epoch);
    expect(store.applySnapshot(snapshot("a", 3, 2, false))).toBe(true);
    expect(store.transitionEpoch).toBe(epoch);
  });

  it("increments epoch once when A is replaced by B and rejects late A", () => {
    const store = createDocumentSessionStore();
    store.commitCandidate(snapshot("a", 10));
    const beforeSwitch = store.transitionEpoch;
    store.commitCandidate(snapshot("b", 1));

    expect(store.transitionEpoch).toBe(beforeSwitch + 1);
    store.closeSession("a");
    expect(store.applySnapshot(snapshot("a", 11, 2, true))).toBe(false);
    expect(store.activeSessionId).toBe("b");
  });

  it("accepts higher state revision for active session without epoch changes and rejects equal/stale revisions", () => {
    const store = createDocumentSessionStore();
    store.commitCandidate(snapshot("a", 1));
    const epoch = store.transitionEpoch;

    expect(store.applySnapshot(snapshot("a", 4, 1, true))).toBe(true);
    expect(store.dirty).toBe(true);
    expect(store.stateRevision).toBe(4);
    expect(store.transitionEpoch).toBe(epoch);

    // Monotonicity check: equal or lower stateRevision must be rejected
    expect(store.applySnapshot(snapshot("a", 4, 2, false))).toBe(false);
    expect(store.applySnapshot(snapshot("a", 3, 2, false))).toBe(false);
  });

  it("captures guard and verifies current state", () => {
    const store = createDocumentSessionStore();
    store.commitCandidate(snapshot("session-1", 2, 5));

    const guard = store.captureGuard({
      includeTransitionEpoch: true,
      includeContentGeneration: true,
      includeStateRevision: true,
    });

    expect(guard.sessionId).toBe("session-1");
    expect(store.isGuardCurrent(guard)).toBe(true);

    // Applying mutation (generation 6, revision 3) invalidates contentGeneration and stateRevision guards
    store.applySnapshot(snapshot("session-1", 3, 6));
    expect(store.isGuardCurrent(guard)).toBe(false);
  });

  it("tracks single-flight save per session", () => {
    const store = createDocumentSessionStore();
    expect(store.isSaveInProgress("sess-1")).toBe(false);
    expect(store.startSave("sess-1")).toBe(true);
    expect(store.isSaveInProgress("sess-1")).toBe(true);
    expect(store.startSave("sess-1")).toBe(false); // Second attempt on same session fails

    // Another session is independent
    expect(store.isSaveInProgress("sess-2")).toBe(false);
    expect(store.startSave("sess-2")).toBe(true);

    store.finishSave("sess-1");
    expect(store.isSaveInProgress("sess-1")).toBe(false);
  });

  it("keeps epoch stable for Save As retarget and increments on active close", () => {
    const store = createDocumentSessionStore();
    store.commitCandidate(snapshot("a", 1));
    const epoch = store.transitionEpoch;
    expect(
      store.applySnapshot({
        ...snapshot("a", 2),
        displayPath: "/retargeted.pdf",
        fileName: "retargeted.pdf",
      }),
    ).toBe(true);
    expect(store.transitionEpoch).toBe(epoch);
    store.closeSession("a");
    expect(store.transitionEpoch).toBe(epoch + 1);
  });

  it("handles active session commit, clear, generation updates, and dirty derivation", () => {
    const store = createDocumentSessionStore();
    expect(store.activeSessionId).toBe(null);
    expect(store.active).toBe(null);
    expect(store.dirty).toBe(false);
    expect(store.contentGeneration).toBe(0);

    store.commitCandidate(snapshot("doc-1", 1, 1, false));
    expect(store.activeSessionId).toBe("doc-1");
    expect(store.active?.id).toBe("doc-1");
    expect(store.contentGeneration).toBe(1);
    expect(store.dirty).toBe(false);

    // Apply snapshot updating generation and dirty
    store.applySnapshot(snapshot("doc-1", 2, 2, true));
    expect(store.contentGeneration).toBe(2);
    expect(store.dirty).toBe(true);

    // Close session clears active PDF state
    store.closeSession("doc-1");
    expect(store.activeSessionId).toBe(null);
    expect(store.active).toBe(null);
    expect(store.dirty).toBe(false);
    expect(store.contentGeneration).toBe(0);
  });
});

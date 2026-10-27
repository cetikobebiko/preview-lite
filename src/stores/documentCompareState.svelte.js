import { buildCompareHighlightsByPage } from "../utils/documentCompare.js";

/**
 * The document compare panel's state. A result belongs to the session and content
 * generation it was computed for; `resultFor` hides it once the document changes.
 */
export function createDocumentCompareState() {
  const state = $state({
    isComparing: false,
    otherPath: "",
    currentIsOld: true,
    result: null,
    error: "",
    sessionId: null,
    contentGeneration: null,
    highlightsByPage: {},
  });
  let activeJob = null;
  let sequence = 0;

  return {
    get isComparing() { return state.isComparing; },
    get otherPath() { return state.otherPath; },
    set otherPath(value) { state.otherPath = value ?? ""; },
    get currentIsOld() { return state.currentIsOld; },
    set currentIsOld(value) { state.currentIsOld = !!value; },
    get error() { return state.error; },

    /** The result, if it was computed for this session and content generation. */
    resultFor(sessionId, contentGeneration) {
      return state.result && state.sessionId === sessionId && state.contentGeneration === contentGeneration
        ? state.result
        : null;
    },

    highlightsFor(sessionId, contentGeneration) {
      return this.resultFor(sessionId, contentGeneration) ? state.highlightsByPage : null;
    },

    /** Runs a comparison; `start` is the backend's `startDocumentCompare`. */
    async run(start, sessionId, contentGeneration) {
      if (!sessionId || !state.otherPath || state.isComparing) return;
      const mine = ++sequence;
      state.isComparing = true;
      state.error = "";
      state.result = null;
      state.highlightsByPage = {};
      try {
        activeJob = start(sessionId, contentGeneration, state.otherPath, state.currentIsOld);
        const result = await activeJob.promise;
        if (mine !== sequence) return;
        state.result = result;
        state.sessionId = sessionId;
        state.contentGeneration = contentGeneration;
        state.highlightsByPage = buildCompareHighlightsByPage(result);
      } catch (error) {
        if (mine !== sequence) return;
        state.error = error?.code === "SESSION_JOB_CANCELLED"
          ? ""
          : error?.message || String(error || "Karşılaştırma tamamlanamadı.");
      } finally {
        if (mine === sequence) {
          state.isComparing = false;
          activeJob = null;
        }
      }
    },

    cancel() {
      void activeJob?.cancel?.();
    },

    reset() {
      sequence += 1;
      void activeJob?.cancel?.();
      activeJob = null;
      state.isComparing = false;
      state.result = null;
      state.error = "";
      state.sessionId = null;
      state.contentGeneration = null;
      state.highlightsByPage = {};
    },
  };
}

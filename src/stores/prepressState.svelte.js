import { createPrepressContextKey, isCurrentPrepressEvent } from "../services/app/prepressIdentity.js";

const defaultChannels = () => ({ c: true, m: true, y: true, k: true });

export function createPrepressState() {
  const state = $state({ contextKey: "", coveragePreview: null, processPreview: null, channels: defaultChannels() });
  const reset = () => { state.coveragePreview = null; state.processPreview = null; state.channels = defaultChannels(); };
  return {
    get coveragePreview() { return state.coveragePreview; },
    get processPreview() { return state.processPreview; },
    get channels() { return state.channels; },
    updateContext(context) {
      const key = createPrepressContextKey(context);
      if (key !== state.contextKey) { state.contextKey = key; reset(); }
      return key;
    },
    applyCoveragePreview(detail, context) {
      if (!isCurrentPrepressEvent(detail, context)) return false;
      if (detail.result === null) {
        if (detail.requestId !== state.coveragePreview?.requestId) return false;
        state.coveragePreview = null;
        return true;
      }
      state.processPreview = null;
      state.coveragePreview = { result: detail.result, ...detail };
      return true;
    },
    applyProcessPreview(detail, context) {
      if (!isCurrentPrepressEvent(detail, context)) return false;
      if (detail.result === null) {
        if (detail.requestId !== state.processPreview?.requestId) return false;
        state.processPreview = null;
        state.channels = defaultChannels();
        return true;
      }
      state.coveragePreview = null;
      state.processPreview = { result: detail.result, ...detail };
      state.channels = detail.channels;
      return true;
    },
    applyChannels(detail, context) {
      if (!isCurrentPrepressEvent(detail, context) || detail.requestId !== state.processPreview?.requestId) return false;
      state.channels = detail.channels;
      return true;
    },
    payload(value, pageIndex, context, key) {
      return value?.sessionId === context.sessionId && value?.contentGeneration === context.contentGeneration && value?.physicalPageIndex === context.physicalPageIndex && value.physicalPageIndex === pageIndex ? value[key] ?? null : null;
    },
  };
}

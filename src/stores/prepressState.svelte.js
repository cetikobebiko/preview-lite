import { createPrepressContextKey, isCurrentPrepressEvent } from "../services/app/prepressIdentity.js";

const defaultChannels = () => ({ c: true, m: true, y: true, k: true });

export function createPrepressState() {
  const state = $state({ contextKey: "", heatmap: null, separations: null, channels: defaultChannels(), boxes: null });
  const reset = () => { state.heatmap = null; state.separations = null; state.channels = defaultChannels(); state.boxes = null; };
  return {
    get heatmap() { return state.heatmap; }, set heatmap(value) { state.heatmap = value; },
    get separations() { return state.separations; }, set separations(value) { state.separations = value; },
    get channels() { return state.channels; }, set channels(value) { state.channels = value; },
    get boxes() { return state.boxes; }, set boxes(value) { state.boxes = value; },
    updateContext(context) { const key = createPrepressContextKey(context); if (key !== state.contextKey) { state.contextKey = key; reset(); } return key; },
    accepts(detail, context) { return isCurrentPrepressEvent(detail, context); },
    applyHeatmap(detail, context) { if (!isCurrentPrepressEvent(detail, context)) return false; state.heatmap = detail.heatmap ? { heatmap: detail.heatmap, sessionId: detail.sessionId, contentGeneration: detail.contentGeneration, pageIndex: detail.pageIndex } : null; return true; },
    applySeparations(detail, context) { if (!isCurrentPrepressEvent(detail, context)) return false; state.separations = detail.images ? { images: detail.images, sessionId: detail.sessionId, contentGeneration: detail.contentGeneration, pageIndex: detail.pageIndex } : null; state.channels = detail.channels; return true; },
    applyChannels(detail, context) { if (!isCurrentPrepressEvent(detail, context)) return false; state.channels = detail.channels; return true; },
    applyBoxes(detail, context) { if (!isCurrentPrepressEvent(detail, context)) return false; state.boxes = detail.boxes; return true; },
    payload(value, pageIndex, context, key) { return value?.sessionId === context.sessionId && value?.contentGeneration === context.contentGeneration && value?.pageIndex === context.physicalPageIndex && value.pageIndex === pageIndex ? value[key] ?? null : null; },
  };
}

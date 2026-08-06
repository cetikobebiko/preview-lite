import {
  clampSearchIndex,
  groupSearchMatchesByPage,
  updateActiveSearchMatchesByPage,
} from "../utils/searchResultUtils.js";

const emptyStats = () => ({ rustTotalMatches: 0, rawMatches: 0, normalizedMatches: 0, droppedMatches: 0, droppedBoxes: 0 });

export function createSearchState() {
  const state = $state({
    query: "", results: [], matchesByPage: {}, totalMatches: 0,
    currentMatchIndex: 0, isSearching: false, error: "", requestId: 0,
    normalizationStats: emptyStats(),
  });
  const completedHighlightScrollKeys = new Set();
  const api = {
    reset({ clearQuery = false } = {}) {
      if (clearQuery) state.query = "";
      state.results = []; state.matchesByPage = {}; state.totalMatches = 0;
      state.currentMatchIndex = 0; state.isSearching = false; state.error = "";
      state.normalizationStats = emptyStats(); completedHighlightScrollKeys.clear();
    },
    begin(requestId) { state.requestId = requestId; state.isSearching = true; api.reset(); state.requestId = requestId; },
    applyResult({ response, normalized }) {
      state.results = normalized.results;
      state.totalMatches = normalized.totalMatches;
      state.normalizationStats = normalized.stats;
      state.currentMatchIndex = clampSearchIndex(0, state.totalMatches);
      state.matchesByPage = groupSearchMatchesByPage(state.results, state.currentMatchIndex);
      return response;
    },
    setError(error) { api.reset(); state.error = error?.message || String(error); },
    settle() { state.isSearching = false; },
    selectMatch(index) {
      const next = clampSearchIndex(index, state.totalMatches);
      const previous = state.currentMatchIndex;
      state.currentMatchIndex = next;
      state.matchesByPage = updateActiveSearchMatchesByPage(state.matchesByPage, state.results, previous, next);
      return state.results[next] ?? null;
    },
    claimHighlight(key) { if (completedHighlightScrollKeys.has(key)) return false; completedHighlightScrollKeys.add(key); return true; },
    get query() { return state.query; }, set query(value) { state.query = value; },
    get results() { return state.results; }, set results(value) { state.results = value; },
    get matchesByPage() { return state.matchesByPage; }, set matchesByPage(value) { state.matchesByPage = value; },
    get totalMatches() { return state.totalMatches; }, set totalMatches(value) { state.totalMatches = value; },
    get currentMatchIndex() { return state.currentMatchIndex; }, set currentMatchIndex(value) { state.currentMatchIndex = value; },
    get isSearching() { return state.isSearching; }, set isSearching(value) { state.isSearching = value; },
    get error() { return state.error; }, set error(value) { state.error = value; },
    get requestId() { return state.requestId; }, set requestId(value) { state.requestId = value; },
    get normalizationStats() { return state.normalizationStats; }, set normalizationStats(value) { state.normalizationStats = value; },
    clearHighlightScroll() { completedHighlightScrollKeys.clear(); },
    highlightKeys: completedHighlightScrollKeys,
  };
  return api;
}

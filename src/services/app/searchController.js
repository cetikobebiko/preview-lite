import { createSearchCoordinator } from "../../utils/searchCoordinator.js";
import { normalizeSearchResponse, wrapSearchIndex } from "../../utils/searchResultUtils.js";

export function createSearchController({ state, search, getContext, getPageOrder, setPageIndex, tick, scrollToPage, debounceMs = 300, onDroppedMatches = () => {}, onError = () => {} }) {
  let coordinator = null;

  async function jumpToMatch(index, expectedRequestId = state.requestId) {
    if (expectedRequestId !== state.requestId || state.totalMatches <= 0) return;
    const match = state.selectMatch(index);
    if (!match) return;
    const displayIndex = getPageOrder().findIndex((pageIndex) => pageIndex === match.pageIndex);
    if (displayIndex < 0) return;
    setPageIndex(displayIndex);
    await tick();
    if (expectedRequestId === state.requestId && state.results[state.currentMatchIndex]?.pageIndex === match.pageIndex) scrollToPage(match.pageIndex);
  }

  function start() {
    coordinator = createSearchCoordinator({
      search,
      getContext,
      debounceMs,
      onReset: (snapshot) => { state.requestId = snapshot.searchRequestId; state.reset(); state.requestId = snapshot.searchRequestId; },
      onSearching: () => { state.isSearching = true; },
      onResult: (_response, snapshot) => {
        const normalized = normalizeSearchResponse(_response);
        state.applyResult({ response: _response, normalized });
        if (state.normalizationStats.droppedMatches > 0) onDroppedMatches(state.normalizationStats.droppedMatches);
        if (state.totalMatches > 0) void jumpToMatch(state.currentMatchIndex, snapshot.searchRequestId);
      },
      onError: (error) => { state.setError(error); onError(error); },
      onSettled: () => { state.settle(); },
    });
  }

  function setQuery(query) { state.query = query; coordinator?.setQuery(query); }
  function handleQueryEvent(event) {
    const detail = event?.detail;
    const query = typeof detail === "string"
      ? detail
      : typeof detail?.query === "string"
        ? detail.query
        : "";
    state.query = query;
    setQuery(detail?.isSemanticMode === true ? "" : query);
  }
  function invalidate({ clearQuery = false } = {}) { if (clearQuery) state.query = ""; if (coordinator) coordinator.invalidate(); else { state.requestId += 1; state.reset(); } }
  function next() { if (state.totalMatches) void jumpToMatch(wrapSearchIndex(state.currentMatchIndex, 1, state.totalMatches)); }
  function previous() { if (state.totalMatches) void jumpToMatch(wrapSearchIndex(state.currentMatchIndex, -1, state.totalMatches)); }
  function handleHighlight({ searchRequestId, matchIndex, pageIndex, element }) {
    if (searchRequestId !== state.requestId || matchIndex !== state.currentMatchIndex || state.results[matchIndex]?.pageIndex !== pageIndex || !element?.isConnected) return;
    if (state.claimHighlight(`${searchRequestId}:${matchIndex}:${pageIndex}`)) element.scrollIntoView({ behavior: "auto", block: "center", inline: "center" });
  }
  function destroy() { coordinator?.destroy(); coordinator = null; state.clearHighlightScroll(); }
  return Object.freeze({ start, setQuery, handleQueryEvent, invalidate, next, previous, jumpToMatch, handleHighlight, destroy });
}

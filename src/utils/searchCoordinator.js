export function normalizeSearchQuery(query) {
  return typeof query === "string" ? query.trim() : "";
}

export function createSearchCoordinator({
  search,
  getContext,
  debounceMs = 300,
  onReset = () => {},
  onSearching = () => {},
  onResult = () => {},
  onError = () => {},
  onSettled = () => {},
} = {}) {
  if (typeof search !== "function") {
    throw new TypeError("search işlevi gerekli.");
  }
  if (typeof getContext !== "function") {
    throw new TypeError("getContext işlevi gerekli.");
  }

  let searchRequestId = 0;
  let activeNormalizedQuery = "";
  let debounceTimer = null;
  let destroyed = false;

  function clearDebounce() {
    if (debounceTimer !== null) {
      clearTimeout(debounceTimer);
      debounceTimer = null;
    }
  }

  function snapshotIsCurrent(snapshot) {
    if (destroyed || snapshot.searchRequestId !== searchRequestId) return false;

    const context = getContext() || {};
    return (
      context.mounted === true &&
      context.sessionId === snapshot.sessionId &&
      context.contentGeneration === snapshot.contentGeneration &&
      activeNormalizedQuery === snapshot.normalizedQuery
    );
  }

  async function execute(snapshot) {
    try {
      const response = await search(
        snapshot.sessionId,
        snapshot.contentGeneration,
        snapshot.normalizedQuery,
      );
      if (!snapshotIsCurrent(snapshot) || response?.is_cancelled === true) return;
      onResult(response, snapshot);
    } catch (error) {
      if (snapshotIsCurrent(snapshot)) {
        onError(error, snapshot);
      }
    } finally {
      if (snapshotIsCurrent(snapshot)) {
        onSettled(snapshot);
      }
    }
  }

  function setQuery(query) {
    clearDebounce();
    activeNormalizedQuery = normalizeSearchQuery(query);
    const context = getContext() || {};
    const snapshot = {
      searchRequestId: ++searchRequestId,
      sessionId: typeof context.sessionId === "string" ? context.sessionId : "",
      contentGeneration: context.contentGeneration,
      normalizedQuery: activeNormalizedQuery,
    };

    onReset(snapshot);

    if (
      destroyed ||
      context.mounted !== true ||
      context.isPdf !== true ||
      !snapshot.sessionId ||
      !snapshot.normalizedQuery
    ) {
      return snapshot.searchRequestId;
    }

    onSearching(snapshot);
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      void execute(snapshot);
    }, Math.max(0, debounceMs));

    return snapshot.searchRequestId;
  }

  function invalidate() {
    clearDebounce();
    activeNormalizedQuery = "";
    const context = getContext() || {};
    const snapshot = {
      searchRequestId: ++searchRequestId,
      sessionId: typeof context.sessionId === "string" ? context.sessionId : "",
      contentGeneration: context.contentGeneration,
      normalizedQuery: "",
    };
    onReset(snapshot);
    return snapshot.searchRequestId;
  }

  function destroy() {
    clearDebounce();
    searchRequestId += 1;
    activeNormalizedQuery = "";
    destroyed = true;
  }

  return {
    setQuery,
    invalidate,
    destroy,
    isCurrent: snapshotIsCurrent,
    getRequestId: () => searchRequestId,
    getNormalizedQuery: () => activeNormalizedQuery,
  };
}

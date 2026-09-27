export function createAiState() {
  let endpointDraft = $state("http://127.0.0.1:11434");
  let activeProfile = $state(null);
  let remoteConsent = $state(false);
  let privateConsent = $state(false);
  let llmModel = $state("llama3.2:latest");
  let embedModel = $state("nomic-embed-text:latest");
  let chunkSize = $state(500);
  let chunkOverlap = $state(50);

  let isIndexing = $state(false);
  let activeStreamingCount = $state(0);
  let activeChatMessages = $state([]);
  let summaryStreamingText = $state("");
  let error = $state(null);
  let _indexedMetadata = $state(null);

  const isHydrated = $derived.by(() => Boolean(activeProfile?.endpoint_fingerprint || activeProfile?.endpointFingerprint));
  const anyStreaming = $derived.by(() => activeStreamingCount > 0);
  const indexedMetadata = $derived(_indexedMetadata);

  const modeLabel = $derived.by(() => {
    if (!activeProfile) return "Bilinmeyen Endpoint";
    if (activeProfile.mode === "Loopback") return "Yerel AI — Loopback";
    if (activeProfile.mode === "PrivateNetwork") return "Özel Ağ AI Endpoint";
    if (activeProfile.mode === "RemotePublic") return "Uzak AI Endpoint";
    return activeProfile.mode;
  });

  const isAuthorized = $derived.by(() => {
    if (!activeProfile) return false;
    if (activeProfile.mode === "Loopback") return true;
    if (activeProfile.mode === "RemotePublic") return remoteConsent;
    if (activeProfile.mode === "PrivateNetwork") return remoteConsent && privateConsent;
    return false;
  });

  function isIndexReadyFor(sessionId, generation) {
    if (isIndexing) return false;
    if (!_indexedMetadata) return false;
    const currentFp = activeProfile?.endpoint_fingerprint || activeProfile?.endpointFingerprint;
    // Fail closed if active profile / endpoint fingerprint is absent
    if (!currentFp) return false;

    return (
      _indexedMetadata.sessionId === sessionId &&
      _indexedMetadata.generation === generation &&
      _indexedMetadata.embedModel === embedModel &&
      _indexedMetadata.chunkSize === chunkSize &&
      _indexedMetadata.chunkOverlap === chunkOverlap &&
      _indexedMetadata.endpointFingerprint === currentFp
    );
  }

  function setIndexedMetadata(meta) {
    _indexedMetadata = meta;
  }

  function clearIndexedMetadata() {
    _indexedMetadata = null;
  }

  return {
    get endpointDraft() { return endpointDraft; },
    set endpointDraft(val) { endpointDraft = val; },

    get activeProfile() { return activeProfile; },
    set activeProfile(val) { activeProfile = val; },

    get isHydrated() { return isHydrated; },

    get remoteConsent() { return remoteConsent; },
    set remoteConsent(val) { remoteConsent = val; },

    get privateConsent() { return privateConsent; },
    set privateConsent(val) { privateConsent = val; },

    get llmModel() { return llmModel; },
    set llmModel(val) { llmModel = val; },

    get embedModel() { return embedModel; },
    set embedModel(val) { embedModel = val; },

    get chunkSize() { return chunkSize; },
    set chunkSize(val) { chunkSize = val; },

    get chunkOverlap() { return chunkOverlap; },
    set chunkOverlap(val) { chunkOverlap = val; },

    get isIndexing() { return isIndexing; },
    set isIndexing(val) { isIndexing = val; },

    get indexedMetadata() { return indexedMetadata; },
    isIndexReadyFor,
    setIndexedMetadata,
    clearIndexedMetadata,

    get activeStreamingCount() { return activeStreamingCount; },
    set activeStreamingCount(val) { activeStreamingCount = Math.max(0, val); },

    get anyStreaming() { return anyStreaming; },
    get isStreaming() { return anyStreaming; },
    set isStreaming(val) {
      if (val) {
        if (activeStreamingCount === 0) activeStreamingCount = 1;
      } else {
        activeStreamingCount = Math.max(0, activeStreamingCount - 1);
      }
    },

    get activeChatMessages() { return activeChatMessages; },
    set activeChatMessages(val) { activeChatMessages = val; },

    get summaryStreamingText() { return summaryStreamingText; },
    set summaryStreamingText(val) { summaryStreamingText = val; },

    get error() { return error; },
    set error(val) { error = val; },

    get modeLabel() { return modeLabel; },
    get isAuthorized() { return isAuthorized; },

    incrementStreaming() {
      activeStreamingCount += 1;
    },

    decrementStreaming() {
      activeStreamingCount = Math.max(0, activeStreamingCount - 1);
    },

    resetSummary() {
      summaryStreamingText = "";
      error = null;
    },

    clearError() {
      error = null;
    },
  };
}

export const aiState = createAiState();

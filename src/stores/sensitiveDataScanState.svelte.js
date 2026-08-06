export function createSensitiveDataScanState() {
  const state = $state({
    isScanning: false,
    report: null,
    error: null,
    capturedPatternLabels: [],
    logicalHighlightsByPage: {},
  });

  return {
    beginScan(patternLabels = []) {
      state.isScanning = true;
      state.error = null;
      state.report = null;
      state.logicalHighlightsByPage = {};
      state.capturedPatternLabels = Object.freeze(
        Array.isArray(patternLabels) ? [...patternLabels] : [],
      );
    },

    completeScan(report, logicalHighlightsByPage = {}) {
      state.isScanning = false;
      state.error = null;
      state.report = report ?? null;
      state.logicalHighlightsByPage = logicalHighlightsByPage ?? {};
    },

    failScan(message) {
      state.isScanning = false;
      state.error =
        typeof message === "string"
          ? message
          : message?.message || String(message || "Tarama başarısız oldu.");
      state.report = null;
      state.logicalHighlightsByPage = {};
    },

    reset() {
      state.isScanning = false;
      state.report = null;
      state.error = null;
      state.capturedPatternLabels = [];
      state.logicalHighlightsByPage = {};
    },

    get isScanning() {
      return state.isScanning;
    },
    get report() {
      return state.report;
    },
    get error() {
      return state.error;
    },
    get capturedPatternLabels() {
      return state.capturedPatternLabels;
    },
    get logicalHighlightsByPage() {
      return state.logicalHighlightsByPage;
    },
  };
}

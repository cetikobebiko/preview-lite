// Offline signature verification panel state (Prompt 11 G2). The report belongs to the
// session and content generation it was produced for; `isStaleFor` tells the panel when
// the document has changed since.
export function createSignatureVerificationState() {
  const state = $state({ isRunning: false, report: null, error: null });

  return {
    begin() {
      state.isRunning = true;
      state.error = null;
      state.report = null;
    },
    complete(report) {
      state.isRunning = false;
      state.error = null;
      state.report = report ?? null;
    },
    fail(error) {
      state.isRunning = false;
      state.report = null;
      state.error =
        typeof error === "string" ? error : error?.message || "İmzalar denetlenemedi.";
    },
    reset() {
      state.isRunning = false;
      state.report = null;
      state.error = null;
    },
    isStaleFor(sessionId, contentGeneration) {
      const report = state.report;
      return Boolean(
        report && (report.sessionId !== sessionId || report.contentGeneration !== contentGeneration),
      );
    },
    get isRunning() {
      return state.isRunning;
    },
    get report() {
      return state.report;
    },
    get error() {
      return state.error;
    },
  };
}

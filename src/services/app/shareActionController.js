/** Owns guarded PDF-session versus external-image sharing decisions. */
export function createShareActionController({ getContext, shareSession, sharePath, guard, featureContext, reportUnavailable, onToast }) {
  async function share() {
    try { return await guard("macos_share", featureContext, execute); }
    catch (error) { if (!reportUnavailable(error)) throw error; }
  }
  async function execute() {
    const context = getContext();
    try {
      if (context.isPdf && context.sessionId) await shareSession(context.sessionId, context.contentGeneration, featureContext);
      else if (context.imagePath) await sharePath(context.imagePath, featureContext);
    } catch (error) { console.error("Share Sheet hatası:", error); onToast("Paylaşım hatası: " + error); }
  }
  return Object.freeze({ share });
}

import { guardFeatureAction } from "./featureActionGuard.js";
import { stopSpeaking as stopSpeakingBackend } from "./services/backend/platform.js";

export async function stopSpeaking(context, onStopped) {
  return await guardFeatureAction("text_to_speech", context, async () => {
    await stopSpeakingBackend();
    if (typeof onStopped === "function") {
      onStopped();
    }
  });
}

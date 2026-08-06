/**
 * Decides whether an asynchronous render result still belongs to the current
 * viewer state. PDF identity is session-native; standalone images are
 * path-native by design.
 */
export function evaluateRenderGuard({
  isMounted,
  request,
  activeRequest,
  isPdf,
  capturedSessionId,
  currentSessionId,
  capturedGeneration,
  currentGeneration,
  capturedImagePath,
  currentImagePath,
}) {
  if (!isMounted || request !== activeRequest) return false;
  if (isPdf) {
    return (
      currentSessionId === capturedSessionId &&
      currentGeneration === capturedGeneration
    );
  }
  return currentImagePath === capturedImagePath;
}

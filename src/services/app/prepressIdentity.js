export function createPrepressContextKey({
  isLoadedPdfDocument,
  sessionId,
  contentGeneration,
  physicalPageIndex,
}) {
  return [
    isLoadedPdfDocument ? (sessionId ?? "") : "",
    contentGeneration,
    physicalPageIndex,
  ].join("\u0000");
}

export function isCurrentPrepressEvent(detail, context) {
  return Boolean(
    detail &&
      context?.isLoadedPdfDocument &&
      detail.sessionId === context.sessionId &&
      detail.contentGeneration === context.contentGeneration &&
      detail.physicalPageIndex === context.physicalPageIndex &&
      typeof detail.requestId === "string" && detail.requestId.length > 0,
  );
}

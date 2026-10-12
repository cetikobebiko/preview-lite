// export_session_text_content prefixes every page with this header in "text" format.
const PAGE_HEADER = /^\s*--- Sayfa \d+ ---\s*/;

export function stripExtractedPageHeader(text) {
  return typeof text === "string" ? text.replace(PAGE_HEADER, "") : "";
}

/**
 * Reads the text of exactly one physical page: PDFium text extraction first; for pages
 * without a text layer (scans) it reuses OCR text cached for that same page or runs OCR.
 * Returns { text, method: "pdf" | "ocr" }.
 */
export async function readPageText(context, { extractText, performOcr, featureContext }) {
  const { viewer, sessionId, contentGeneration, pageIndex } = context;
  const extracted = stripExtractedPageHeader(await extractText(sessionId, contentGeneration, [pageIndex], "text"));
  if (extracted.trim()) return { text: extracted.trim(), method: "pdf" };
  if (viewer.documentTextContext && viewer.documentTextPageIndex === pageIndex) {
    return { text: viewer.documentTextContext, method: "ocr" };
  }
  return { text: await performOcr(sessionId, contentGeneration, pageIndex, featureContext), method: "ocr" };
}

import { describe, expect, it, vi } from "vitest";
import { createDocumentScopedStateReconciler } from "./documentScopedState.js";

function setup() {
  const viewer = { documentTextContext: "", documentTextPageIndex: -1, bionicText: "", bionicSource: null };
  const annotation = { annotationsMap: {}, isDrawing: false, currentPoints: [], currentAnnoItem: null };
  const bindHighlightSession = vi.fn();
  const remapHighlightPages = vi.fn();
  const rotateHighlightPage = vi.fn();
  const closeDocumentModals = vi.fn();
  const reconciler = createDocumentScopedStateReconciler({
    viewer, annotation, bindHighlightSession, remapHighlightPages, rotateHighlightPage, closeDocumentModals,
  });
  return { viewer, annotation, bindHighlightSession, remapHighlightPages, rotateHighlightPage, closeDocumentModals, reconciler };
}

function draw(annotation) {
  annotation.annotationsMap = { 0: [{ page_index: 0, tool_type: "pen" }] };
  annotation.isDrawing = true;
  annotation.currentPoints = [{ x: 1, y: 1 }];
  annotation.currentAnnoItem = { page_index: 0, tool_type: "pen" };
}

function fillText(viewer, text) {
  viewer.documentTextContext = text;
  viewer.bionicText = text;
}

describe("document-scoped state reconciler", () => {
  it("clears OCR/Bionic text and rebinds highlights when another PDF becomes active", () => {
    const { viewer, bindHighlightSession, reconciler } = setup();
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 1, imagePath: "" });
    fillText(viewer, "A belgesinin metni");

    expect(reconciler.reconcile({ sessionId: "doc-b", contentGeneration: 1, imagePath: "" })).toBe(true);
    expect(viewer.documentTextContext).toBe("");
    expect(viewer.bionicText).toBe("");
    expect(bindHighlightSession).toHaveBeenLastCalledWith("doc-b");
  });

  it("keeps text while the same document and generation stay active", () => {
    const { viewer, bindHighlightSession, reconciler } = setup();
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 1, imagePath: "" });
    fillText(viewer, "A metni");
    expect(reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 1, imagePath: "" })).toBe(false);
    expect(viewer.documentTextContext).toBe("A metni");
    expect(bindHighlightSession).toHaveBeenCalledTimes(1);
  });

  it("drops text produced from an older content generation of the same document", () => {
    const { viewer, bindHighlightSession, reconciler } = setup();
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 1, imagePath: "" });
    fillText(viewer, "silinen sayfanın OCR metni");
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 2, imagePath: "" });
    expect(viewer.documentTextContext).toBe("");
    expect(viewer.bionicText).toBe("");
    // Highlights stay bound to the same session; page renumbering is remapped explicitly.
    expect(bindHighlightSession).toHaveBeenLastCalledWith("doc-a");
  });

  it("clears on close and on switching between standalone images", () => {
    const { viewer, bindHighlightSession, reconciler } = setup();
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 1, imagePath: "" });
    fillText(viewer, "A metni");
    reconciler.reconcile({ sessionId: null, contentGeneration: 0, imagePath: "" });
    expect(viewer.bionicText).toBe("");
    expect(bindHighlightSession).toHaveBeenLastCalledWith(null);

    reconciler.reconcile({ sessionId: null, contentGeneration: 0, imagePath: "/a.png" });
    fillText(viewer, "görsel metni");
    reconciler.reconcile({ sessionId: null, contentGeneration: 0, imagePath: "/b.png" });
    expect(viewer.documentTextContext).toBe("");
  });

  it("drops canvas drawings and any in-progress gesture when another document becomes active", () => {
    const { annotation, reconciler } = setup();
    reconciler.reconcile({ sessionId: null, contentGeneration: 0, imagePath: "/a.png" });
    draw(annotation);
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 1, imagePath: "" });
    expect(annotation).toEqual({ annotationsMap: {}, isDrawing: false, currentPoints: [], currentAnnoItem: null });

    draw(annotation);
    reconciler.reconcile({ sessionId: null, contentGeneration: 0, imagePath: "/b.png" });
    expect(annotation.annotationsMap).toEqual({});
  });

  it("keeps drawings across a content-generation change of the same document", () => {
    const { annotation, reconciler } = setup();
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 1, imagePath: "" });
    draw(annotation);
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 2, imagePath: "" });
    expect(annotation.annotationsMap[0]).toHaveLength(1);
  });

  it("closes document modals and clears text provenance only when the document changes", () => {
    const { viewer, closeDocumentModals, reconciler } = setup();
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 1, imagePath: "" });
    viewer.documentTextPageIndex = 3;
    viewer.bionicSource = { pageNumber: 4, method: "ocr" };
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 2, imagePath: "" });
    expect(closeDocumentModals).toHaveBeenCalledTimes(1);
    expect(viewer).toMatchObject({ documentTextPageIndex: -1, bionicSource: null });
    reconciler.reconcile({ sessionId: "doc-b", contentGeneration: 1, imagePath: "" });
    expect(closeDocumentModals).toHaveBeenCalledTimes(2);
  });

  it("applies page renumbering to highlights and drawings of the active session only", () => {
    const { annotation, remapHighlightPages, rotateHighlightPage, reconciler } = setup();
    reconciler.reconcile({ sessionId: "doc-a", contentGeneration: 1, imagePath: "" });
    annotation.annotationsMap = { 2: [{ page_index: 2 }] };
    const mapPageIndex = (index) => (index === 2 ? 1 : -1);

    expect(reconciler.applyPageMutation("doc-b", { mapPageIndex })).toBe(false);
    expect(remapHighlightPages).not.toHaveBeenCalled();
    expect(annotation.annotationsMap).toEqual({ 2: [{ page_index: 2 }] });

    expect(reconciler.applyPageMutation("doc-a", { mapPageIndex })).toBe(true);
    expect(remapHighlightPages).toHaveBeenCalledWith("doc-a", mapPageIndex);
    expect(annotation.annotationsMap).toEqual({ 1: [{ page_index: 1 }] });
    expect(rotateHighlightPage).not.toHaveBeenCalled();

    reconciler.applyPageMutation("doc-a", { rotation: { pageIndex: 1, degrees: -90 } });
    expect(rotateHighlightPage).toHaveBeenCalledWith("doc-a", 1, -90);
  });
});

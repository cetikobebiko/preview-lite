export function resolvePhysicalPageIndex(order, displayIndex) {
  if (!Array.isArray(order) || !Number.isInteger(displayIndex) || displayIndex < 0 || displayIndex >= order.length) return -1;
  const physicalIndex = order[displayIndex];
  return Number.isInteger(physicalIndex) && physicalIndex >= 0 && physicalIndex < order.length
    ? physicalIndex
    : -1;
}

export function createViewerState() {
  const state = $state({
    activeImageSourcePath: "",
    // Bitmap size of the painted image canvas ({ width, height } in px); null until one is painted.
    imagePixelSize: null,
    fileExt: "",
    pageOrder: [],
    currentPageIndex: 0,
    scale: 1,
    visualScale: 1,
    isBookMode: false,
    isSinglePageMode: false,
    coverMode: true,
    isSmartDarkMode: false,
    isSplitView: false,
    rotation: 0,
    flipH: false,
    flipV: false,
    renderGenerationId: 0,
    isRendering: false,
    isDocumentLoading: false,
    isZooming: false,
    zoomOriginY: null,
    historyStack: [],
    forwardStack: [],
    pdfMetadata: { title: "", author: "", subject: "", keywords: "", creator: "" },
    outlineList: [],
    documentTextContext: "",
    // Physical page whose text documentTextContext holds; -1 when empty.
    documentTextPageIndex: -1,
    readingTimeSeconds: 0,
    bionicText: "",
    // { pageNumber, method: "pdf" | "ocr" } describing where bionicText came from.
    bionicSource: null,
    isSpeaking: false,
  });

  const spreads = $derived.by(() => {
    if (!state.isBookMode) return state.pageOrder.map((index) => [index]);
    const rows = [];
    let index = 0;
    if (state.coverMode && state.pageOrder.length > 0) { rows.push([state.pageOrder[0]]); index = 1; }
    while (index < state.pageOrder.length) {
      rows.push(state.pageOrder.slice(index, index + 2));
      index += 2;
    }
    return rows;
  });

  return {
    get activeImageSourcePath() { return state.activeImageSourcePath; },
    set activeImageSourcePath(value) { state.activeImageSourcePath = value; },
    get imagePixelSize() { return state.imagePixelSize; }, set imagePixelSize(value) { state.imagePixelSize = value; },
    get fileExt() { return state.fileExt; }, set fileExt(value) { state.fileExt = value; },
    get pageOrder() { return state.pageOrder; }, set pageOrder(value) { state.pageOrder = value; },
    get currentPageIndex() { return state.currentPageIndex; }, set currentPageIndex(value) { state.currentPageIndex = value; },
    get scale() { return state.scale; }, set scale(value) { state.scale = value; },
    get visualScale() { return state.visualScale; }, set visualScale(value) { state.visualScale = value; },
    get isBookMode() { return state.isBookMode; }, set isBookMode(value) { state.isBookMode = value; },
    get isSinglePageMode() { return state.isSinglePageMode; }, set isSinglePageMode(value) { state.isSinglePageMode = value; },
    get coverMode() { return state.coverMode; }, set coverMode(value) { state.coverMode = value; },
    get isSmartDarkMode() { return state.isSmartDarkMode; }, set isSmartDarkMode(value) { state.isSmartDarkMode = value; },
    get isSplitView() { return state.isSplitView; }, set isSplitView(value) { state.isSplitView = value; },
    get rotation() { return state.rotation; }, set rotation(value) { state.rotation = value; },
    get flipH() { return state.flipH; }, set flipH(value) { state.flipH = value; },
    get flipV() { return state.flipV; }, set flipV(value) { state.flipV = value; },
    get renderGenerationId() { return state.renderGenerationId; }, set renderGenerationId(value) { state.renderGenerationId = value; },
    get isRendering() { return state.isRendering; }, set isRendering(value) { state.isRendering = value; },
    get isDocumentLoading() { return state.isDocumentLoading; }, set isDocumentLoading(value) { state.isDocumentLoading = value; },
    get isZooming() { return state.isZooming; }, set isZooming(value) { state.isZooming = value; },
    get zoomOriginY() { return state.zoomOriginY; }, set zoomOriginY(value) { state.zoomOriginY = value; },
    get historyStack() { return state.historyStack; }, set historyStack(value) { state.historyStack = value; },
    get forwardStack() { return state.forwardStack; }, set forwardStack(value) { state.forwardStack = value; },
    get pdfMetadata() { return state.pdfMetadata; }, set pdfMetadata(value) { state.pdfMetadata = value; },
    get outlineList() { return state.outlineList; }, set outlineList(value) { state.outlineList = value; },
    get documentTextContext() { return state.documentTextContext; }, set documentTextContext(value) { state.documentTextContext = value; },
    get documentTextPageIndex() { return state.documentTextPageIndex; }, set documentTextPageIndex(value) { state.documentTextPageIndex = value; },
    get readingTimeSeconds() { return state.readingTimeSeconds; }, set readingTimeSeconds(value) { state.readingTimeSeconds = value; },
    get bionicText() { return state.bionicText; }, set bionicText(value) { state.bionicText = value; },
    get bionicSource() { return state.bionicSource; }, set bionicSource(value) { state.bionicSource = value; },
    get isSpeaking() { return state.isSpeaking; }, set isSpeaking(value) { state.isSpeaking = value; },
    get physicalPageIndex() { return resolvePhysicalPageIndex(state.pageOrder, state.currentPageIndex); },
    // Same array until pageOrder or the book layout changes; the page layout model keys on it.
    get spreads() { return spreads; },
    resetPdfPresentation() {
      state.currentPageIndex = 0;
      state.scale = 1;
      state.visualScale = 1;
      state.zoomOriginY = null;
      state.rotation = 0;
      state.flipH = false;
      state.flipV = false;
      state.outlineList = [];
      state.pdfMetadata = { title: "", author: "", subject: "", keywords: "", creator: "" };
    },
  };
}

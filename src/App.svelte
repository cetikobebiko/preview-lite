<script>
  import { onMount, setContext, tick, untrack } from "svelte";
  import AppHeader from "./components/AppHeader.svelte";
  import AppModalHost from "./components/AppModalHost.svelte";
  import AppOverlayHost from "./components/AppOverlayHost.svelte";
  import AppPresentationHost from "./components/AppPresentationHost.svelte";
  import SearchHost from "./components/SearchHost.svelte";
  import ViewerHudDock from "./components/ViewerHudDock.svelte";
  import ViewerShell from "./components/ViewerShell.svelte";
  import WorkspacePanels from "./components/WorkspacePanels.svelte";
  import {
    ASYNC_PROMPT_CONTEXT,
    createAsyncPromptCoordinator,
  } from "./services/asyncPrompt.svelte.js";
  import { createApplicationRuntime } from "./services/app/applicationRuntime.js";
  import { createDocumentRuntime } from "./services/app/documentRuntime.js";
  import { createFeatureRuntime } from "./services/app/featureRuntime.js";
  import { createPresentationController } from "./services/app/presentationController.js";
  import { createViewerRuntime } from "./services/app/viewerRuntime.js";
  import {
    createRenderSession,
    RENDER_SESSION_CONTEXT,
  } from "./services/renderSession.js";
  import { createAnnotationState } from "./stores/annotationState.svelte.js";
  import {
    createDocumentSessionStore,
    DOCUMENT_SESSION_CONTEXT,
  } from "./stores/documentSession.svelte.js";
  import { createPrepressState } from "./stores/prepressState.svelte.js";
  import { createSearchState } from "./stores/searchState.svelte.js";
  import { createSensitiveDataScanState } from "./stores/sensitiveDataScanState.svelte.js";
  import { createSecureRedactionState } from "./stores/secureRedactionState.svelte.js";
  import { createSanitizationState } from "./stores/sanitizationState.svelte.js";
  import { createUiState } from "./stores/uiState.svelte.js";
  import { createViewerState } from "./stores/viewerState.svelte.js";

  const renderSession = createRenderSession();
  setContext(RENDER_SESSION_CONTEXT, renderSession);
  const documentSession = createDocumentSessionStore();
  setContext(DOCUMENT_SESSION_CONTEXT, documentSession);
  const asyncPrompt = createAsyncPromptCoordinator();
  setContext(ASYNC_PROMPT_CONTEXT, asyncPrompt);

  const ui = createUiState();
  const viewer = createViewerState();
  const annotation = createAnnotationState();
  const searchState = createSearchState();
  const sensitiveScanState = createSensitiveDataScanState();
  const secureRedactionState = createSecureRedactionState();
  const sanitizationState = createSanitizationState();
  const prepress = createPrepressState();
  const presentation = createPresentationController({ ui });

  let canvasRefLeft = $state();
  let viewerAreaRef = $state();
  let isComponentMounted = false;
  const callbacks = {
    isMounted: () => isComponentMounted,
    saveSettings: () => {},
    savePosition: () => {},
    invalidateSearch: () => {},
  };

  const viewerRuntime = createViewerRuntime({
    viewer, annotation, documentSession, renderSession, asyncPrompt, tick,
    getIsMounted: callbacks.isMounted,
    getViewerArea: () => viewerAreaRef,
    getCanvas: () => canvasRefLeft,
    callbacks,
    onToast: presentation.showToast,
    onError: presentation.setError,
  });
  const documentRuntime = createDocumentRuntime({
    documentSession, renderSession, viewer, annotation, ui, tick,
    viewerController: viewerRuntime.controller,
    getCanvas: () => canvasRefLeft,
    getIsMounted: callbacks.isMounted,
    invalidateSearch: (options) => callbacks.invalidateSearch(options),
    onToast: presentation.showToast,
    onError: presentation.setError,
    passwordPresentation: {
      show: presentation.showPasswordAttempt,
      error: presentation.showPasswordError,
      dismiss: presentation.dismissPasswordAttempt,
    },
  });
  callbacks.savePosition = documentRuntime.savePosition;
  const featureRuntime = createFeatureRuntime({
    viewer, annotation, documentSession, searchState, sensitiveScanState, secureRedactionState, sanitizationState, ui, asyncPrompt, tick,
    viewerRuntime, documentRuntime, callbacks,
    onToast: presentation.showToast,
    onError: presentation.setError,
  });
  callbacks.invalidateSearch = featureRuntime.invalidateSearch;
  const applicationRuntime = createApplicationRuntime({
    viewer, ui, documentSession, asyncPrompt,
    presentationController: presentation,
    documentRuntime, viewerRuntime, featureRuntime, callbacks,
    getViewerArea: () => viewerAreaRef,
    setMounted: (value) => { isComponentMounted = value; },
  });

  const colorPalette = ["#ff3b30", "#ffcc00", "#30d158", "#0071e3", "#ffffff", "#000000"];
  const getPrepressContext = () => ({
    isLoadedPdfDocument: documentRuntime.isLoadedPdfDocument,
    sessionId: documentSession.activeSessionId,
    contentGeneration: documentSession.contentGeneration,
    physicalPageIndex: viewer.physicalPageIndex,
  });
  const applyPrepress = (method, event) => prepress[method](event.detail, getPrepressContext());
  const getActivePrepressPayload = (state, pageIndex, _sessionId, _generationId, _physicalPageIndex, payloadKey) =>
    prepress.payload(state, pageIndex, getPrepressContext(), payloadKey);

  $effect(() => {
    const titlePath = documentRuntime.isPdf
      ? (documentRuntime.isLoadedPdfDocument ? documentRuntime.activeDisplayPath : "")
      : viewer.activeImageSourcePath;
    document.title = titlePath ? `${titlePath} - Antigravity` : "Antigravity PDF Reader";
    prepress.updateContext(getPrepressContext());
  });

  $effect(() => {
    documentRuntime.isPdf;
    documentSession.activeSessionId;
    documentSession.contentGeneration;
    untrack(() => featureRuntime.sanitizationInspection?.reconcileContext?.());
  });

  $effect(() => {
    ui.panels.secureRedaction;
    documentRuntime.isPdf;
    documentSession.activeSessionId;
    documentSession.contentGeneration;

    untrack(() => {
      featureRuntime.secureRedaction?.reconcileContext?.();
    });
  });

  $effect(() => {
    ui.panels.sensitiveDataScan;
    documentRuntime.isPdf;
    documentSession.activeSessionId;
    documentSession.contentGeneration;

    untrack(() => {
      featureRuntime.reconcileSensitiveDataContext();
    });
  });

  onMount(applicationRuntime.mount);
</script>

<svelte:window on:keydown={applicationRuntime.handleKeyDown} />

<AppPresentationHost
  {ui}
  {viewer}
  {documentSession}
  featureUi={featureRuntime.featureUi}
  activePdfDisplayPath={documentRuntime.activeDisplayPath}
  fileName={documentRuntime.fileName}
  onToast={presentation.showToast}
  onSessionMutation={(snapshot) => void documentRuntime.applyMutationSnapshot(snapshot)}
/>

<AppModalHost
  {ui}
  featureUi={featureRuntime.featureUi}
  sessionId={documentSession.activeSessionId ?? ""}
  contentGeneration={documentSession.contentGeneration}
  displayPath={documentRuntime.activeDisplayPath}
  onCommandSelect={featureRuntime.commandRouter.handleEvent}
  onSanitize={(event) => featureRuntime.sanitize(event.detail)}
/>

<main class="app-container">
  <AppHeader
    isPdf={documentRuntime.isPdf}
    {viewer}
    {ui}
    fileName={documentRuntime.fileName}
    activePdfDisplayPath={documentRuntime.activeDisplayPath}
    featureUi={featureRuntime.featureUi}
    featureControlLabel={featureRuntime.featureControlLabel}
    onFeatureAction={featureRuntime.runUiAction}
    onSelectFile={documentRuntime.selectFile}
  />

  <SearchHost
    {ui}
    {searchState}
    featureUi={featureRuntime.featureUi}
    onSearch={featureRuntime.handleSearchQuery}
    onNext={featureRuntime.searchNext}
    onPrevious={featureRuntime.searchPrevious}
  />

  <div class="workspace-body">
    <WorkspacePanels
      isPdf={documentRuntime.isPdf}
      {ui}
      {viewer}
      {documentSession}
      {sensitiveScanState}
      {secureRedactionState}
      featureUi={featureRuntime.featureUi}
      physicalPageIndex={viewer.physicalPageIndex}
      isLoadedPdfDocument={documentRuntime.isLoadedPdfDocument}
      onJump={viewerRuntime.navigation.jumpToPage}
      onScanSensitiveData={featureRuntime.scanSensitiveData}
      secureRedaction={featureRuntime.secureRedaction}
      onSessionSnapshot={documentRuntime.applyBackendSessionSnapshot}
      onSeparations={(event) => applyPrepress("applySeparations", event)}
      onHeatmap={(event) => applyPrepress("applyHeatmap", event)}
      onChannels={(event) => applyPrepress("applyChannels", event)}
      onGuides={(event) => applyPrepress("applyBoxes", event)}
      onApplyBoxes={(event) => applyPrepress("applyBoxes", event)}
    />

    <ViewerShell
      isPdf={documentRuntime.isPdf}
      {viewer}
      {annotation}
      {documentSession}
      {searchState}
      spreads={viewer.spreads}
      physicalPageIndex={viewer.physicalPageIndex}
      {prepress}
      featureUi={featureRuntime.featureUi}
      errorMsg={ui.error}
      {getActivePrepressPayload}
      getSensitiveDataHighlights={featureRuntime.getSensitiveDataHighlightsForPage}
      {secureRedactionState}
      secureRedaction={featureRuntime.secureRedaction}
      onSelectFile={documentRuntime.selectFile}
      onPageIntersect={viewerRuntime.navigation.handlePageIntersect}
      onSearchHighlightReady={featureRuntime.handleSearchHighlight}
      onSessionSnapshot={(snapshot) => void documentRuntime.applyMutationSnapshot(snapshot)}
      onCanvasMouseDown={viewerRuntime.annotationCanvas.handleMouseDown}
      onCanvasMouseMove={viewerRuntime.annotationCanvas.handleMouseMove}
      onCanvasMouseUp={viewerRuntime.annotationCanvas.handleMouseUp}
      bind:viewerAreaRef
      bind:canvasRefLeft
    />
  </div>

  <ViewerHudDock
    isPdf={documentRuntime.isPdf} {viewer} {ui} {annotation}
    featureUi={featureRuntime.featureUi} featureControlLabel={featureRuntime.featureControlLabel}
    {colorPalette} isDirty={documentRuntime.isDirty} isLoadedPdfDocument={documentRuntime.isLoadedPdfDocument}
    onFeatureAction={featureRuntime.runUiAction} onGoBack={viewerRuntime.navigation.goBack} onGoForward={viewerRuntime.navigation.goForward}
    onPreviousPage={viewerRuntime.navigation.goToPrevPage} onNextPage={viewerRuntime.navigation.goToNextPage} onPageInputChange={viewerRuntime.navigation.handlePageInputChange}
    onRotateLeft={viewerRuntime.edits.rotateLeft} onRotateRight={viewerRuntime.edits.rotateRight} onFlipH={viewerRuntime.edits.toggleFlipH} onFlipV={viewerRuntime.edits.toggleFlipV}
    onZoomOut={viewerRuntime.navigation.zoomOut} onResetZoom={viewerRuntime.navigation.resetZoom} onZoomIn={viewerRuntime.navigation.zoomIn} onToggleSinglePage={viewerRuntime.navigation.toggleSinglePageMode}
    onToggleBookMode={viewerRuntime.navigation.toggleBookMode} onToggleDarkMode={featureRuntime.toggleDarkMode} onOpenBionic={featureRuntime.openBionicReading}
    onRunOcr={featureRuntime.runOcr} onSpeak={featureRuntime.speak} onStopSpeaking={featureRuntime.stopSpeaking}
    onSave={documentRuntime.save} onBurnIn={featureRuntime.burnInPdf} onExportImage={featureRuntime.exportImage}
  />

  <AppOverlayHost
    {ui}
    toastMsg={ui.toast}
    onPasswordSubmit={documentRuntime.lifecycleController.submitPassword}
    onPasswordCancel={documentRuntime.lifecycleController.cancelPassword}
    {asyncPrompt}
    featureUi={featureRuntime.featureUi}
    sessionId={documentSession.activeSessionId ?? ""}
    contentGeneration={documentSession.contentGeneration}
    isLoadedPdfDocument={documentRuntime.isLoadedPdfDocument}
  />
</main>

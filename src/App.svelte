<script>
  import { onMount, setContext, tick, untrack } from "svelte";
  import AppModalHost from "./components/AppModalHost.svelte";
  import AppOverlayHost from "./components/AppOverlayHost.svelte";
  import AppPresentationHost from "./components/AppPresentationHost.svelte";
  import AppToolbar from "./components/AppToolbar.svelte";
  import MarkupToolbar from "./components/MarkupToolbar.svelte";
  import SearchHost from "./components/SearchHost.svelte";
  import ViewerShell from "./components/ViewerShell.svelte";
  import WorkspacePanels from "./components/WorkspacePanels.svelte";
  import {
    ASYNC_PROMPT_CONTEXT,
    createAsyncPromptCoordinator,
  } from "./services/asyncPrompt.svelte.js";
  import { createApplicationRuntime } from "./services/app/applicationRuntime.js";
  import { createDocumentRuntime } from "./services/app/documentRuntime.js";
  import { createDocumentScopedStateReconciler } from "./services/app/documentScopedState.js";
  import { createFeatureRuntime } from "./services/app/featureRuntime.js";
  import { createPresentationController } from "./services/app/presentationController.js";
  import { createViewerRuntime } from "./services/app/viewerRuntime.js";
  import {
    createRenderSession,
    RENDER_SESSION_CONTEXT,
  } from "./services/renderSession.js";
  import { createAnnotationState } from "./stores/annotationState.svelte.js";
  import {
    createFormDraftState,
    FORM_DRAFTS_CONTEXT,
  } from "./stores/formDraftState.svelte.js";
  import {
    createFormDesignState,
    FORM_DESIGN_CONTEXT,
  } from "./stores/formDesignState.svelte.js";
  import {
    createDocumentSessionStore,
    DOCUMENT_SESSION_CONTEXT,
  } from "./stores/documentSession.svelte.js";
  import { createPrepressState } from "./stores/prepressState.svelte.js";
  import { createSearchState } from "./stores/searchState.svelte.js";
  import { createSensitiveDataScanState } from "./stores/sensitiveDataScanState.svelte.js";
  import { createDocumentCompareState } from "./stores/documentCompareState.svelte.js";
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
  setContext(FORM_DRAFTS_CONTEXT, createFormDraftState());
  const formDesign = createFormDesignState();
  setContext(FORM_DESIGN_CONTEXT, formDesign);

  const ui = createUiState();
  const viewer = createViewerState();
  const annotation = createAnnotationState();
  const searchState = createSearchState();
  const sensitiveScanState = createSensitiveDataScanState();
  const documentCompareState = createDocumentCompareState();
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
  const documentScopedState = createDocumentScopedStateReconciler({
    viewer, annotation,
    closeDocumentModals: () => ui.closeModal("bionic"),
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
    applyPageScopedMutation: documentScopedState.applyPageMutation,
    scrollToPhysicalPage: viewerRuntime.navigation.scrollToPhysicalPage,
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
    const context = {
      sessionId: documentSession.activeSessionId,
      contentGeneration: documentSession.contentGeneration,
      imagePath: viewer.activeImageSourcePath,
    };
    untrack(() => documentScopedState.reconcile(context));
  });

  $effect(() => {
    documentRuntime.isPdf;
    documentSession.activeSessionId;
    documentSession.contentGeneration;
    untrack(() => featureRuntime.sanitization?.reconcileContext?.());
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

<svelte:window onkeydown={applicationRuntime.handleKeyDown} />

<AppPresentationHost
  {ui}
  {viewer}
  {documentSession}
  featureUi={featureRuntime.featureUi}
  activePdfDisplayPath={documentRuntime.activeDisplayPath}
  fileName={documentRuntime.fileName}
  onToast={presentation.showToast}
  onSessionMutation={(snapshot, options) => void documentRuntime.applyMutationSnapshot(snapshot, options)}
/>

<AppModalHost
  {ui}
  featureUi={featureRuntime.featureUi}
  displayPath={documentRuntime.activeDisplayPath}
  {sanitizationState}
  sanitization={featureRuntime.sanitization}
  onCommandSelect={featureRuntime.commandRouter.handleEvent}
/>

<main class="app-container">
  <AppToolbar
    isPdf={documentRuntime.isPdf} {viewer} {ui} {annotation}
    fileName={documentRuntime.fileName} activePdfDisplayPath={documentRuntime.activeDisplayPath}
    featureUi={featureRuntime.featureUi} isDirty={documentRuntime.isDirty}
    onFeatureAction={featureRuntime.runUiAction} onSelectFile={documentRuntime.selectFile}
    onGoBack={viewerRuntime.navigation.goBack} onGoForward={viewerRuntime.navigation.goForward}
    onPreviousPage={viewerRuntime.navigation.goToPrevPage} onNextPage={viewerRuntime.navigation.goToNextPage} onPageInputChange={viewerRuntime.navigation.handlePageInputChange}
    onRotateLeft={viewerRuntime.edits.rotateLeft} onRotateRight={viewerRuntime.edits.rotateRight} onFlipH={viewerRuntime.edits.toggleFlipH} onFlipV={viewerRuntime.edits.toggleFlipV}
    onZoomOut={viewerRuntime.navigation.zoomOut} onResetZoom={viewerRuntime.navigation.resetZoom} onZoomIn={viewerRuntime.navigation.zoomIn} onToggleSinglePage={viewerRuntime.navigation.toggleSinglePageMode}
    onToggleBookMode={viewerRuntime.navigation.toggleBookMode} onToggleDarkMode={featureRuntime.toggleDarkMode} onOpenBionic={featureRuntime.openBionicReading}
    onRunOcr={featureRuntime.runOcr} onSpeak={featureRuntime.speak} onStopSpeaking={featureRuntime.stopSpeaking}
    onSave={documentRuntime.save} onExportImage={featureRuntime.exportImage}
  />

  {#if ui.panels.markup && (documentRuntime.isPdf || viewer.activeImageSourcePath) && featureRuntime.featureUi("annotation").visible}
    <MarkupToolbar {annotation} {ui} featureUi={featureRuntime.featureUi} {colorPalette} {formDesign} />
  {/if}

  <div class="workspace-body">
    <SearchHost
      {ui}
      {searchState}
      featureUi={featureRuntime.featureUi}
      onSearch={featureRuntime.handleSearchQuery}
      onNext={featureRuntime.searchNext}
      onPrevious={featureRuntime.searchPrevious}
    />

    <WorkspacePanels
      isPdf={documentRuntime.isPdf}
      {ui}
      {viewer}
      {documentSession}
      {sensitiveScanState}
      {documentCompareState}
      {secureRedactionState}
      featureUi={featureRuntime.featureUi}
      physicalPageIndex={viewer.physicalPageIndex}
      isLoadedPdfDocument={documentRuntime.isLoadedPdfDocument}
      onJump={viewerRuntime.navigation.jumpToPage}
      onJumpPhysical={viewerRuntime.navigation.jumpToPhysicalPage}
      onScanSensitiveData={featureRuntime.scanSensitiveData}
      secureRedaction={featureRuntime.secureRedaction}
      onSessionSnapshot={documentRuntime.applyBackendSessionSnapshot}
      onProcessPreview={(event) => applyPrepress("applyProcessPreview", event)}
      onCoveragePreview={(event) => applyPrepress("applyCoveragePreview", event)}
      onChannels={(event) => applyPrepress("applyChannels", event)}
    >
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
        {documentCompareState}
        showCompareHighlights={ui.panels.documentCompare}
        {secureRedactionState}
        secureRedaction={featureRuntime.secureRedaction}
        onSelectFile={documentRuntime.selectFile}
        onReopenDocument={() => void documentRuntime.openFile(documentRuntime.activeDisplayPath)}
        onPageIntersect={viewerRuntime.navigation.handlePageIntersect}
        onSearchHighlightReady={featureRuntime.handleSearchHighlight}
        onSessionSnapshot={(snapshot) => void documentRuntime.applyMutationSnapshot(snapshot)}
        onCanvasMouseDown={viewerRuntime.annotationCanvas.handleMouseDown}
        onCanvasMouseMove={viewerRuntime.annotationCanvas.handleMouseMove}
        onCanvasMouseUp={viewerRuntime.annotationCanvas.handleMouseUp}
        bind:viewerAreaRef
        bind:canvasRefLeft
      />
    </WorkspacePanels>
  </div>

  <AppOverlayHost
    {ui}
    toastMsg={ui.toast}
    onPasswordSubmit={documentRuntime.lifecycleController.submitPassword}
    onPasswordCancel={documentRuntime.lifecycleController.cancelPassword}
    {asyncPrompt}
  />
</main>

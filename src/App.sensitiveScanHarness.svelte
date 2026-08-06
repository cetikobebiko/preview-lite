<script>
  import { untrack } from "svelte";
  import { createSensitiveDataScanController } from "./services/app/sensitiveDataScanController.js";
  import { createSensitiveDataScanState } from "./stores/sensitiveDataScanState.svelte.js";
  import { createUiState } from "./stores/uiState.svelte.js";
  import { createDocumentSessionStore } from "./stores/documentSession.svelte.js";

  let { startScanMock = () => {} } = $props();

  export const ui = createUiState();
  ui.setPanel("sensitiveDataScan", true);

  export const documentSession = createDocumentSessionStore();
  const initSnapshot = {
    id: "session-1",
    filePath: "/test.pdf",
    fileKind: "pdf",
    contentGeneration: 1,
    stateRevision: 1,
    physicalPageCount: 5,
    userRotation: 0,
    pageRotations: {},
    displayPath: "/test.pdf",
  };
  documentSession.commitCandidate(initSnapshot);

  let docRuntimeState = $state({ isPdf: true });
  export const documentRuntime = {
    get isPdf() { return docRuntimeState.isPdf; },
    set isPdf(val) { docRuntimeState.isPdf = val; },
  };

  export const sensitiveScanState = createSensitiveDataScanState();

  export const sensitiveScanController = createSensitiveDataScanController({
    state: sensitiveScanState,
    startScan: (...args) => startScanMock(...args),
    getContext: () => ({
      panelOpen: ui.panels.sensitiveDataScan,
      isPdf: documentRuntime.isPdf,
      sessionId: documentSession.activeSessionId,
      contentGeneration: documentSession.contentGeneration,
    }),
  });

  sensitiveScanController.start();

  $effect(() => {
    ui.panels.sensitiveDataScan;
    documentRuntime.isPdf;
    documentSession.activeSessionId;
    documentSession.contentGeneration;

    untrack(() => {
      sensitiveScanController.reconcileContext();
    });
  });
</script>

<div data-testid="harness">
  <span data-testid="error">{sensitiveScanState.error || ""}</span>
</div>

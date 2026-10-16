import { readImageFile, setActiveRenderDocument } from "../backend/index.js";
import { paintBufferToCanvas } from "../../utils/canvasUtils.js";
import { createAnnotationCanvasController } from "./annotationCanvasController.js";
import { paintAnnotations } from "./annotationCanvasRenderer.js";
import { createViewerController } from "./viewerController.js";
import { createViewerEditActions } from "./viewerEditActions.js";
import { createViewerNavigationController } from "./viewerNavigationController.js";
import { createPageLayoutProvider } from "./pageLayoutModel.js";
import { pageSizeStore } from "../pageSizeStore.js";

/** Composes viewer render, navigation and canvas interaction around one viewer controller. */
export function createViewerRuntime({
  viewer,
  annotation,
  documentSession,
  renderSession,
  asyncPrompt,
  tick,
  getIsMounted,
  getViewerArea,
  getCanvas,
  callbacks,
  onToast,
  onError,
}) {
  const isPdf = () => Boolean(documentSession.active);

  const controller = createViewerController({
    setActiveRenderDocument,
    renderSession,
    getRenderContext: () => ({
      mounted: getIsMounted(),
      imagePath: viewer.activeImageSourcePath,
    }),
    readImage: readImageFile,
    paint: paintBufferToCanvas,
    onError: (error) => {
      console.error("Render hatası:", error);
      onError("Görüntüleme hatası: " + error.toString());
    },
    onRenderFinished: () => { viewer.isRendering = false; },
    onImagePainted: (size) => { viewer.imagePixelSize = size; },
  });

  function render(options = { silent: false }) {
    if ((!isPdf() && !viewer.activeImageSourcePath) || (isPdf() && !getCanvas())) return;
    if (isPdf()) return;
    if (options.silent !== true) viewer.isRendering = true;
    onError("");
    const canvas = getCanvas();
    controller.renderImage({
      canvas,
      rotation: viewer.rotation,
      flipH: viewer.flipH,
      flipV: viewer.flipV,
      overlay: () => {
        paintAnnotations({
          canvas,
          pageIndex: viewer.physicalPageIndex,
          annotation,
          currentPageIndex: viewer.physicalPageIndex,
        });
        callbacks.savePosition();
      },
    });
  }

  const navigation = createViewerNavigationController({
    viewer,
    viewerController: controller,
    isPdf,
    getViewerArea,
    tick,
    render,
    saveSettings: () => callbacks.saveSettings(),
    onToast,
    getPageLayout: createPageLayoutProvider({
      viewer,
      documentSession,
      peekPageSizes: pageSizeStore.peekAll,
    }),
  });

  const edits = createViewerEditActions({
    viewer, annotation, render, onToast,
    preserveViewportAnchor: navigation.preserveViewportAnchor,
  });
  const annotationCanvas = createAnnotationCanvasController({
    annotation,
    viewer,
    getCanvases: () => ({ left: getCanvas(), right: null }),
    promptText: (options) => asyncPrompt.request(options),
    render,
  });

  function start() {
    controller.attachViewport({
      element: getViewerArea(),
      viewer,
      isMounted: getIsMounted,
      hasDocument: () => Boolean(documentSession.active || viewer.activeImageSourcePath),
      onRender: render,
      onZoomStart: navigation.captureZoomAnchor,
      onZoomCommit: () => { void navigation.restoreZoomAnchor(); },
    });
  }

  return Object.freeze({
    controller,
    navigation,
    edits,
    annotationCanvas,
    render,
    start,
    stop: controller.stop,
    deactivatePdf: controller.deactivatePdf,
  });
}

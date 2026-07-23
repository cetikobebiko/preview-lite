// CLEAN, NATIVE PDF.JS STATE MANAGEMENT
// PDF.js objects intentionally live in module-scoped plain JavaScript. They must
// never be put into Svelte state, since proxies, canvases and render tasks are
// mutable and not safe for reactive serialization.

import * as pdfjsLib from "pdfjs-dist";
import { invoke } from "@tauri-apps/api/core";

const pdfjsWorkerUrl = new URL(
  "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url
).href;

pdfjsLib.GlobalWorkerOptions.workerSrc = pdfjsWorkerUrl;

export const MAX_CANVAS_DIMENSION = 4096;

// Enough pages for the current view, nearby thumbnails and a search highlight,
// without keeping an entire large document resident in PDF.js memory.
const MAX_PAGE_CACHE_SIZE = 16;

let _searchWorker = null;
let _activeLoadingTask = null;
let _teardownPromise = null;
let _documentEpoch = 0;
let _renderSequence = 0;
const _canvasRenderTasks = new WeakMap();
const _pendingPageRequests = new Map();

async function safelyDestroyDocument(documentProxy) {
  if (!documentProxy || typeof documentProxy.destroy !== "function") return;

  try {
    await documentProxy.destroy();
  } catch (_) {}
}

function safelyCleanupPage(pageProxy) {
  if (!pageProxy || typeof pageProxy.cleanup !== "function") return;

  try {
    pageProxy.cleanup();
  } catch (_) {}
}

function destroySearchWorker() {
  if (!_searchWorker) return;

  try {
    _searchWorker.terminate();
  } catch (_) {}
  _searchWorker = null;
}

export function isPdfCancellationError(error) {
  return ["RenderingCancelledException", "AbortException"].includes(error?.name);
}

function isCurrentDocument(documentProxy) {
  return Boolean(documentProxy) && pdfState.pdfDocProxy === documentProxy;
}

function isActiveRenderRecord(record) {
  return (
    Boolean(record) &&
    !record.cancelled &&
    pdfState.activeRenderTasks.has(record) &&
    _canvasRenderTasks.get(record.canvas) === record
  );
}

function removeRenderRecord(record) {
  if (!record) return;

  pdfState.activeRenderTasks.delete(record);
  if (_canvasRenderTasks.get(record.canvas) === record) {
    _canvasRenderTasks.delete(record.canvas);
  }
}

function cancelRenderRecord(record) {
  if (!record) return;

  record.cancelled = true;
  try {
    record.task?.cancel?.();
  } catch (_) {}

  // A task cancelled before its consumer awaits it must still have a rejection
  // handler, otherwise PDF.js can surface an unhandled promise rejection.
  record.task?.promise?.catch?.(() => {});
}

function pageHasActiveWork(pageIndex, documentProxy) {
  for (const record of pdfState.activeRenderTasks) {
    if (record.pageIndex === pageIndex && record.documentProxy === documentProxy) {
      return true;
    }
  }
  return false;
}

function touchCachedPage(pageIndex, pageProxy) {
  pdfState.pageCache.delete(pageIndex);
  pdfState.pageCache.set(pageIndex, pageProxy);
}

function trimPageCache(documentProxy) {
  while (pdfState.pageCache.size > MAX_PAGE_CACHE_SIZE) {
    let evicted = false;

    for (const [pageIndex, pageProxy] of pdfState.pageCache) {
      const pendingKey = `${_documentEpoch}:${pageIndex}`;
      if (pageHasActiveWork(pageIndex, documentProxy) || _pendingPageRequests.has(pendingKey)) {
        continue;
      }

      pdfState.pageCache.delete(pageIndex);
      safelyCleanupPage(pageProxy);
      evicted = true;
      break;
    }

    // Every cached page is currently being used. Let the next completed task
    // trim it instead of cleaning a page while PDF.js is rendering it.
    if (!evicted) break;
  }
}

async function getPageProxy(documentProxy, pageIndex) {
  if (!isCurrentDocument(documentProxy) || !Number.isInteger(pageIndex) || pageIndex < 0) {
    return null;
  }

  const cached = pdfState.pageCache.get(pageIndex);
  if (cached) {
    touchCachedPage(pageIndex, cached);
    return cached;
  }

  const requestKey = `${_documentEpoch}:${pageIndex}`;
  let pageRequest = _pendingPageRequests.get(requestKey);
  if (!pageRequest) {
    pageRequest = documentProxy.getPage(pageIndex + 1);
    _pendingPageRequests.set(requestKey, pageRequest);
  }

  try {
    const pageProxy = await pageRequest;
    if (!isCurrentDocument(documentProxy)) {
      safelyCleanupPage(pageProxy);
      return null;
    }

    touchCachedPage(pageIndex, pageProxy);
    trimPageCache(documentProxy);
    return pageProxy;
  } finally {
    if (_pendingPageRequests.get(requestKey) === pageRequest) {
      _pendingPageRequests.delete(requestKey);
    }
  }
}

function createRenderRecord(pageIndex, canvas, documentProxy) {
  const record = {
    id: ++_renderSequence,
    pageIndex,
    canvas,
    documentProxy,
    task: null,
    cancelled: false,
  };

  _canvasRenderTasks.set(canvas, record);
  pdfState.activeRenderTasks.add(record);
  return record;
}

function getSafeViewport(pageProxy, scale, rotation) {
  const unscaledViewport = pageProxy.getViewport({ scale: 1, rotation });
  const requestedScale = Number.isFinite(scale) && scale > 0 ? scale : 1;
  const safeScale = Math.min(
    requestedScale,
    MAX_CANVAS_DIMENSION / unscaledViewport.width,
    MAX_CANVAS_DIMENSION / unscaledViewport.height
  );
  const viewport = pageProxy.getViewport({ scale: safeScale, rotation });

  if (!Number.isFinite(viewport.width) || !Number.isFinite(viewport.height)) {
    throw new Error("Geçersiz PDF sayfa boyutu.");
  }

  return viewport;
}

function prepareCanvas(canvas, viewport) {
  const width = Math.max(1, Math.floor(viewport.width));
  const height = Math.max(1, Math.floor(viewport.height));
  canvas.width = width;
  canvas.height = height;

  const context = canvas.getContext("2d", { alpha: false });
  if (!context) throw new Error("Canvas 2D bağlamı oluşturulamadı.");

  context.clearRect(0, 0, width, height);
  return context;
}

export const pdfState = {
  // These are module-local mutable resources, not Svelte reactive values.
  pdfDocProxy: null,
  pageCache: new Map(),
  activeRenderTasks: new Set(),

  async loadDocument(filePath) {
    await this.destroyDocument();
    const loadEpoch = ++_documentEpoch;
    let loadingTask;

    try {
      const data = await invoke("read_binary_file", { path: filePath });
      const uint8Array = new Uint8Array(data);

      loadingTask = pdfjsLib.getDocument({
        data: uint8Array,
        cMapUrl: "/cmaps/",
        cMapPacked: true,
        standardFontDataUrl: "/standard_fonts/",
        isEvalSupported: false,
      });
      _activeLoadingTask = { task: loadingTask, epoch: loadEpoch };

      const documentProxy = await loadingTask.promise;
      const isCurrentLoad =
        loadEpoch === _documentEpoch &&
        _activeLoadingTask?.task === loadingTask;

      if (!isCurrentLoad) {
        void safelyDestroyDocument(documentProxy);
        return null;
      }

      _activeLoadingTask = null;
      this.pdfDocProxy = documentProxy;
      return documentProxy;
    } catch (error) {
      if (_activeLoadingTask?.task === loadingTask) {
        _activeLoadingTask = null;
      }

      if (isPdfCancellationError(error) || loadEpoch !== _documentEpoch) {
        return null;
      }

      throw error;
    }
  },

  destroyDocument() {
    if (_teardownPromise) return _teardownPromise;

    _documentEpoch += 1;

    const loading = _activeLoadingTask;
    _activeLoadingTask = null;
    try {
      loading?.task?.destroy?.();
    } catch (_) {}

    const activeRecords = [...this.activeRenderTasks];
    for (const record of activeRecords) {
      cancelRenderRecord(record);
      removeRenderRecord(record);
    }

    const cachedPages = [...this.pageCache.values()];
    this.pageCache.clear();
    _pendingPageRequests.clear();

    const documentProxy = this.pdfDocProxy;
    this.pdfDocProxy = null;
    destroySearchWorker();

    _teardownPromise = (async () => {
      try {
        await Promise.all(
          activeRecords.map((record) => record.task?.promise?.catch?.(() => {}))
        );
        for (const pageProxy of cachedPages) {
          safelyCleanupPage(pageProxy);
        }
        await safelyDestroyDocument(documentProxy);
      } finally {
        _teardownPromise = null;
      }
    })();

    return _teardownPromise;
  },

  getSearchWorker() {
    if (_searchWorker) return _searchWorker;

    try {
      _searchWorker = new Worker(new URL("./searchWorker.js", import.meta.url), {
        type: "module",
      });
      return _searchWorker;
    } catch (error) {
      console.error("Search worker creation failed:", error);
      return null;
    }
  },
};

// Cancelling by canvas prevents a stale render's finally block from removing a
// newer render task for that same canvas.
export async function cancelCanvasRender(canvas) {
  const record = canvas ? _canvasRenderTasks.get(canvas) : null;
  if (!record) return;

  cancelRenderRecord(record);
  try {
    await record.task?.promise;
  } catch (_) {
    // Cancellation is an expected control-flow path.
  } finally {
    removeRenderRecord(record);
  }
}

export async function cancelPageRender(pageIndex) {
  const records = [...pdfState.activeRenderTasks].filter(
    (record) => record.pageIndex === pageIndex
  );

  await Promise.all(records.map((record) => cancelCanvasRender(record.canvas)));
}

export function cleanupCanvasMemory(canvas, pageIndex = null) {
  if (canvas) {
    // Do not await during component teardown; the task has its own rejection
    // handler and the zero-sized canvas immediately releases its backing store.
    void cancelCanvasRender(canvas);
    try {
      const context = canvas.getContext("2d");
      context?.clearRect(0, 0, canvas.width, canvas.height);
      canvas.width = 0;
      canvas.height = 0;
    } catch (_) {}
  }

  if (pageIndex !== null && pdfState.pageCache.has(pageIndex)) {
    const pageProxy = pdfState.pageCache.get(pageIndex);
    if (!pageHasActiveWork(pageIndex, pdfState.pdfDocProxy)) {
      pdfState.pageCache.delete(pageIndex);
      safelyCleanupPage(pageProxy);
    }
  }
}

export async function renderPdfPage(pageIndex, canvas, scale, rotation = 0) {
  const documentProxy = pdfState.pdfDocProxy;
  if (!canvas || !isCurrentDocument(documentProxy)) return false;

  await cancelCanvasRender(canvas);
  if (!isCurrentDocument(documentProxy)) return false;

  const record = createRenderRecord(pageIndex, canvas, documentProxy);
  try {
    const pageProxy = await getPageProxy(documentProxy, pageIndex);
    if (!pageProxy || !isActiveRenderRecord(record) || !isCurrentDocument(documentProxy)) {
      return false;
    }

    const viewport = getSafeViewport(pageProxy, scale, rotation);
    const canvasContext = prepareCanvas(canvas, viewport);
    if (!isActiveRenderRecord(record) || !isCurrentDocument(documentProxy)) return false;

    record.task = pageProxy.render({ canvasContext, viewport });
    await record.task.promise;
    return isActiveRenderRecord(record) && isCurrentDocument(documentProxy);
  } catch (error) {
    // Non-critical render warnings are silenced for production
    return false;
  } finally {
    removeRenderRecord(record);
    trimPageCache(documentProxy);
  }
}

export async function renderPdfTextLayer(pageIndex, textLayerDiv, scale, rotation = 0) {
  const documentProxy = pdfState.pdfDocProxy;
  if (!textLayerDiv || !isCurrentDocument(documentProxy)) return false;

  try {
    const pageProxy = await getPageProxy(documentProxy, pageIndex);
    if (!pageProxy || !isCurrentDocument(documentProxy)) return false;

    const viewport = getSafeViewport(pageProxy, scale, rotation);
    const textContent = await pageProxy.getTextContent();

    if (!isCurrentDocument(documentProxy)) return false;

    textLayerDiv.innerHTML = "";
    textLayerDiv.style.setProperty("--scale-factor", viewport.scale);

    const textLayer = new pdfjsLib.TextLayer({
      textContentSource: textContent,
      container: textLayerDiv,
      viewport: viewport,
    });
    await textLayer.render();

    return true;
  } catch (error) {
    // Silenced for production
    return false;
  }
}

export async function renderPdfThumbnail(pageIndex, canvas, targetWidth = 140) {
  const documentProxy = pdfState.pdfDocProxy;
  if (!canvas || !isCurrentDocument(documentProxy)) return false;

  await cancelCanvasRender(canvas);
  if (!isCurrentDocument(documentProxy)) return false;

  const record = createRenderRecord(pageIndex, canvas, documentProxy);
  try {
    const pageProxy = await getPageProxy(documentProxy, pageIndex);
    if (!pageProxy || !isActiveRenderRecord(record) || !isCurrentDocument(documentProxy)) {
      return false;
    }

    const unscaledViewport = pageProxy.getViewport({ scale: 1 });
    const thumbnailScale = Math.min(targetWidth / unscaledViewport.width, 0.4);
    const viewport = getSafeViewport(pageProxy, thumbnailScale, 0);
    const canvasContext = prepareCanvas(canvas, viewport);
    if (!isActiveRenderRecord(record) || !isCurrentDocument(documentProxy)) return false;

    record.task = pageProxy.render({ canvasContext, viewport });
    await record.task.promise;
    return isActiveRenderRecord(record) && isCurrentDocument(documentProxy);
  } catch (error) {
    // Silenced for production
    return false;
  } finally {
    removeRenderRecord(record);
    trimPageCache(documentProxy);
  }
}

export async function getDocumentOutline(documentProxy = pdfState.pdfDocProxy) {
  if (!isCurrentDocument(documentProxy)) return [];

  try {
    const outline = await documentProxy.getOutline();
    if (!isCurrentDocument(documentProxy) || !outline?.length) return [];

    async function resolveOutlineNode(node, level = 0) {
      if (!isCurrentDocument(documentProxy)) return null;

      let pageIndex = 0;
      if (node.dest) {
        try {
          let destination = node.dest;
          if (typeof destination === "string") {
            destination = await documentProxy.getDestination(destination);
          }
          if (Array.isArray(destination) && destination[0] !== null) {
            pageIndex = await documentProxy.getPageIndex(destination[0]);
          } else if (typeof destination === "number") {
            pageIndex = destination;
          }
        } catch (error) {
          // Outline destination resolution warning silenced for production
        }
      }

      if (!isCurrentDocument(documentProxy)) return null;
      const children = [];
      for (const childNode of node.items || []) {
        const child = await resolveOutlineNode(childNode, level + 1);
        if (child) children.push(child);
      }

      return {
        title: node.title || "Başlıksız",
        page_index: Math.max(0, Math.min(pageIndex, documentProxy.numPages - 1)),
        level,
        children,
      };
    }

    const nodes = [];
    for (const item of outline) {
      const node = await resolveOutlineNode(item);
      if (node) nodes.push(node);
    }
    return isCurrentDocument(documentProxy) ? nodes : [];
  } catch (error) {
    // Silenced for production
    return [];
  }
}

export async function searchPdfDocument(
  query,
  documentProxy = pdfState.pdfDocProxy,
  shouldContinue = () => true,
  abortSignal = null
) {
  if (!isCurrentDocument(documentProxy) || !query?.trim() || !shouldContinue() || abortSignal?.aborted) {
    return { results: [], limitExceeded: false };
  }

  const trimmedQuery = query.trim();
  const lowerQuery = trimmedQuery.toLowerCase();
  const results = [];
  const MAX_RESULTS = 5000;
  let limitExceeded = false;

  for (let pageNumber = 1; pageNumber <= documentProxy.numPages; pageNumber += 1) {
    if (!isCurrentDocument(documentProxy) || !shouldContinue() || abortSignal?.aborted) {
      return { results: [], limitExceeded: false };
    }

    // Yield to main thread every 2 pages to keep UI responsive
    if (pageNumber % 2 === 0) {
      await new Promise(r => setTimeout(r, 0));
    }

    try {
      const pageProxy = await getPageProxy(documentProxy, pageNumber - 1);
      if (!pageProxy || !isCurrentDocument(documentProxy) || !shouldContinue() || abortSignal?.aborted) {
        return { results: [], limitExceeded: false };
      }

      const textContent = await pageProxy.getTextContent();
      if (!isCurrentDocument(documentProxy) || !shouldContinue() || abortSignal?.aborted) {
        return { results: [], limitExceeded: false };
      }

      let fullPageText = "";
      const itemBounds = [];
      for (const item of textContent.items || []) {
        if (!item || typeof item.str !== "string") continue;

        const start = fullPageText.length;
        fullPageText += `${item.str} `;
        const transform = item.transform || [1, 0, 0, 1, 0, 0];
        itemBounds.push({
          str: item.str,
          start,
          end: fullPageText.length,
          x: transform[4],
          y: transform[5],
          w: item.width || 0,
          h: item.height || (transform[3] ? Math.abs(transform[3]) : 12),
          transform,
        });
      }

      let matchPosition = fullPageText.toLowerCase().indexOf(lowerQuery);
      while (matchPosition !== -1) {
        const matchingItems = itemBounds.filter(
          (item) =>
            item.end > matchPosition && item.start < matchPosition + trimmedQuery.length
        );
        
        const preciseMatchingItems = matchingItems.map((item) => {
          const overlapStart = Math.max(item.start, matchPosition);
          const overlapEnd = Math.min(item.end, matchPosition + trimmedQuery.length);
          const localStart = overlapStart - item.start;
          const localEnd = overlapEnd - item.start;
          const totalLength = item.str.length;
          
          if (totalLength === 0) return item;

          const charWidth = item.w / totalLength;
          const offsetX = localStart * charWidth;
          const matchWidth = (localEnd - localStart) * charWidth;

          return {
            ...item,
            x: item.x + offsetX,
            w: Math.max(matchWidth, 1) // prevent zero-width highlights
          };
        });

        results.push({
          page_index: pageNumber - 1,
          match_index: results.length,
          snippet: fullPageText.substring(
            Math.max(0, matchPosition - 25),
            Math.min(fullPageText.length, matchPosition + trimmedQuery.length + 25)
          ),
          query_length: trimmedQuery.length,
          itemBounds: preciseMatchingItems,
        });

        if (results.length >= MAX_RESULTS) {
          limitExceeded = true;
          break;
        }

        matchPosition = fullPageText
          .toLowerCase()
          .indexOf(lowerQuery, matchPosition + trimmedQuery.length);
      }
      
      if (limitExceeded) break;
    } catch (error) {
      // Silenced for production
    }
  }

  const isValidState = isCurrentDocument(documentProxy) && shouldContinue() && !abortSignal?.aborted;
  return isValidState ? { results, limitExceeded } : { results: [], limitExceeded: false };
}

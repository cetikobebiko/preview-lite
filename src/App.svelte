<script>
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { listen } from "@tauri-apps/api/event";
  import Sidebar from "./components/Sidebar.svelte";
  import CommandPalette from "./components/CommandPalette.svelte";
  import SearchBar from "./components/SearchBar.svelte";
  import AiAssistantModal from "./components/AiAssistantModal.svelte";
  import ReadingRuler from "./components/ReadingRuler.svelte";
  import BionicReadingModal from "./components/BionicReadingModal.svelte";
  import AnalyticsModal from "./components/AnalyticsModal.svelte";
  import MetadataModal from "./components/MetadataModal.svelte";

  import {
    pdfState,
    renderPdfPage,
    renderPdfTextLayer,
    cancelCanvasRender,
    cleanupCanvasMemory,
    getDocumentOutline,
    isPdfCancellationError,
    searchPdfDocument
  } from "./pdfState.js";

  // REACTIVE VIEW STATE (Only basic primitive values for UI reactive bindings)
  let selectedFilePath = "";
  let fileName = "";
  let fileExt = "";
  let isPdf = false;
  let isDraggingFileOver = false;

  let pageOrder = [];
  let currentPageIndex = 0;
  let scale = 1.0;
  let visualScale = 1.0;
  let isBookMode = false;
  let isSmartDarkMode = false;
  let isSplitView = false;
  let isSpeaking = false;
  let isReadingRulerActive = false;

  // UI Toggles & Modals
  let showSidebar = true;
  let showSettingsPopover = false;
  let isCmdPaletteOpen = false;
  let isSearchBarOpen = false;
  let isAiModalOpen = false;
  let isBionicModalOpen = false;
  let isAnalyticsModalOpen = false;
  let isMetadataModalOpen = false;

  let pdfMetadata = { title: "", author: "", subject: "", keywords: "", creator: "" };
  let outlineList = [];
  let totalSearchMatches = 0;
  const rawSearchResults = { data: [] };
  let searchAbortController = null;
  let searchDebounceTimer = null;
  let currentSearchMatchIndex = 0;
  let isSearchingText = false;
  let documentTextContext = "";

  let canvasRefLeft;
  let canvasRefRight;
  let textLayerRefLeft;
  let textLayerRefRight;
  let viewerAreaRef;

  let isRendering = false;
  let isDocumentLoading = false;
  let isZooming = false;
  let zoomDebounceTimer = null;
  let resizeDebounceTimer = null;
  let resizeObserver = null;
  let toastTimer = null;
  let renderAnimationFrame = null;
  let documentLoadRequest = 0;
  let renderRequest = 0;
  let searchRequest = 0;
  let isComponentMounted = false;
  let errorMsg = "";

  // Reading Session Analytics & Position Memory
  let readingTimeSeconds = 0;
  let analyticsInterval = null;
  let bionicText = "";

  // Image Transformation States
  let rotation = 0;
  let flipH = false;
  let flipV = false;

  // Vector Annotations State
  let activeTool = "pointer";
  let strokeColor = "#ff3b30";
  let strokeSize = 3;
  let annotationsMap = {};

  let isDrawing = false;
  let currentPoints = [];
  let startPos = { x: 0, y: 0 };
  let currentAnnoItem = null;

  const colorPalette = [
    "#ff3b30",
    "#ffcc00",
    "#30d158",
    "#0071e3",
    "#ffffff",
    "#000000",
  ];

  // Toast Notification
  let toastMsg = "";

  function showToast(message) {
    toastMsg = message;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      toastMsg = "";
      toastTimer = null;
    }, 3000);
  }

  // Complete Document Purge & Destroy Memory
  function purgeDocumentMemory() {
    renderRequest += 1;
    if (renderAnimationFrame !== null) {
      cancelAnimationFrame(renderAnimationFrame);
      renderAnimationFrame = null;
    }
  }

  function handleDocumentClosed() {
    if (textLayerRefLeft) textLayerRefLeft.innerHTML = "";
    if (textLayerRefRight) textLayerRefRight.innerHTML = "";
    if (canvasRefLeft) cleanupCanvasMemory(canvasRefLeft, currentPageIndex);
    if (canvasRefRight) cleanupCanvasMemory(canvasRefRight, currentPageIndex + 1);

    pdfState.destroyDocument();

    annotationsMap = {};
    documentTextContext = "";
  }

  // Save Reading Position & Zoom Memory per Document
  function saveDocumentPositionMemory() {
    if (!selectedFilePath) return;
    const key = `pos_mem_${selectedFilePath}`;
    const data = {
      pageIndex: currentPageIndex,
      scale: scale,
    };
    try {
      localStorage.setItem(key, JSON.stringify(data));
    } catch (e) {
      // Sessizce yoksay veya loglama
    }
  }

  function restoreDocumentPositionMemory(filePath) {
    const key = `pos_mem_${filePath}`;
    try {
      const saved = localStorage.getItem(key);
      if (saved) {
        const data = JSON.parse(saved);
        if (Number.isInteger(data.pageIndex) && pageOrder.length > 0) {
          currentPageIndex = Math.min(Math.max(data.pageIndex, 0), pageOrder.length - 1);
        }
        if (Number.isFinite(data.scale)) {
          scale = Math.min(Math.max(data.scale, 0.25), 4.0);
        }
        showToast(`Son kalınan konum hatırlandı (Sayfa ${currentPageIndex + 1})`);
      }
    } catch (e) {
      // Sessizce yoksay
    }
  }

  // Keyboard Shortcuts Handler
  function handleKeyDown(e) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "b") {
      e.preventDefault();
      showSidebar = !showSidebar;
      return;
    }

    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "o") {
      e.preventDefault();
      handleSelectFile();
      return;
    }

    if (!selectedFilePath) return;

    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      isCmdPaletteOpen = !isCmdPaletteOpen;
      return;
    }

    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      isSearchBarOpen = true;
      return;
    }

    const activeEl = document.activeElement;
    const tagName = activeEl?.tagName?.toLowerCase();
    if (tagName === "input" || tagName === "textarea" || activeEl?.isContentEditable) {
      return;
    }

    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      goToNextPage();
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      goToPrevPage();
    }
  }

  // Native Scroll, Drag-and-Drop & Debounced Pure CSS Zoom Setup
  onMount(() => {
    isComponentMounted = true;
    analyticsInterval = setInterval(() => {
      if (selectedFilePath) {
        readingTimeSeconds += 1;
      }
    }, 1000);

    let unlistenDrop = null;
    let didUnmount = false;
    listen("tauri://drag-drop", (event) => {
      if (!isComponentMounted) return;
      const paths = event.payload?.paths;
      if (paths && paths.length > 0) {
        void loadDocumentFile(paths[0]);
        showToast("Sürüklenen dosya açıldı.");
      }
    })
      .then((fn) => {
        if (didUnmount) {
          fn();
          return;
        }
        unlistenDrop = fn;
      })
      .catch((error) => console.error("Dosya bırakma dinleyicisi kurulamadı:", error));

    function handleWindowDragOver(e) {
      e.preventDefault();
      isDraggingFileOver = true;
    }

    function handleWindowDragLeave(e) {
      if (e.clientX <= 0 || e.clientY <= 0 || e.clientX >= window.innerWidth || e.clientY >= window.innerHeight) {
        isDraggingFileOver = false;
      }
    }

    function handleWindowDrop(e) {
      e.preventDefault();
      isDraggingFileOver = false;
      if (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files.length > 0) {
        const file = e.dataTransfer.files[0];
        const filePath = file.path || file.name;
        if (filePath) {
          void loadDocumentFile(filePath);
          showToast(`Dosya yüklendi: ${file.name || "Yeni Belge"}`);
        }
      }
    }

    window.addEventListener("dragover", handleWindowDragOver);
    window.addEventListener("dragleave", handleWindowDragLeave);
    window.addEventListener("drop", handleWindowDrop);

    function handleWheel(e) {
      if (!selectedFilePath) return;

      // Pinch-to-zoom / Cmd+Wheel zoom
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();

        const zoomFactor = e.deltaY < 0 ? 1.05 : 0.95;
        let nextVisualScale = visualScale * zoomFactor;

        const effectiveScale = scale * nextVisualScale;
        if (effectiveScale < 0.25) {
          nextVisualScale = 0.25 / scale;
        } else if (effectiveScale > 4.0) {
          nextVisualScale = 4.0 / scale;
        }

        visualScale = nextVisualScale;
        isZooming = true;

        if (zoomDebounceTimer) clearTimeout(zoomDebounceTimer);

        // Pure CSS GPU transform during active zoom gesture, single crisp re-render 300ms after zoom ends
        zoomDebounceTimer = setTimeout(() => {
          let finalScale = scale * visualScale;
          finalScale = Math.min(Math.max(finalScale, 0.25), 4.0);

          scale = parseFloat(finalScale.toFixed(2));
          visualScale = 1.0;
          isZooming = false;

          if (isComponentMounted) renderCurrentDocument({ silent: true });
        }, 300);
      }
    }

    const viewerElement = viewerAreaRef;
    if (viewerElement) {
      viewerElement.addEventListener("wheel", handleWheel, { passive: false });
    }

    // ResizeObserver — debounced re-render on window/container resize
    if (viewerElement && typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(() => {
        if (!isComponentMounted || !selectedFilePath) return;
        if (resizeDebounceTimer) clearTimeout(resizeDebounceTimer);
        resizeDebounceTimer = setTimeout(() => {
          if (isComponentMounted) renderCurrentDocument({ silent: true });
        }, 200);
      });
      resizeObserver.observe(viewerElement);
    }

    return () => {
      didUnmount = true;
      isComponentMounted = false;
      documentLoadRequest += 1;
      searchRequest += 1;
      if (unlistenDrop) unlistenDrop();
      window.removeEventListener("dragover", handleWindowDragOver);
      window.removeEventListener("dragleave", handleWindowDragLeave);
      window.removeEventListener("drop", handleWindowDrop);
      if (viewerElement) {
        viewerElement.removeEventListener("wheel", handleWheel);
      }
      if (resizeObserver) {
        resizeObserver.disconnect();
        resizeObserver = null;
      }
      if (resizeDebounceTimer) clearTimeout(resizeDebounceTimer);
      if (zoomDebounceTimer) clearTimeout(zoomDebounceTimer);
      if (analyticsInterval) clearInterval(analyticsInterval);
      if (toastTimer) clearTimeout(toastTimer);
      purgeDocumentMemory();
    };
  });

  async function handleSelectFile() {
    errorMsg = "";
    try {
      const selected = await open({
        multiple: false,
        filters: [
          {
            name: "Desteklenen Belgeler ve Görseller",
            extensions: ["pdf", "png", "jpg", "jpeg", "webp", "bmp", "svg"],
          },
        ],
      });

      if (selected) {
        const filePath = typeof selected === "string" ? selected : selected.path;
        await loadDocumentFile(filePath);
      }
    } catch (err) {
      console.error("Dosya seçimi hatası:", err);
      errorMsg = "Dosya açılırken hata oluştu: " + err.toString();
    }
  }

  async function loadDocumentFile(filePath) {
    if (!isComponentMounted || typeof filePath !== "string" || !filePath.trim() || filePath === selectedFilePath) {
      return;
    }

    const loadRequest = ++documentLoadRequest;
    isDocumentLoading = true;

    selectedFilePath = filePath;
    fileName = filePath.split("/").pop() || "Dosya";
    fileExt = filePath.split(".").pop()?.toLowerCase() || "";
    isPdf = fileExt === "pdf";

    searchRequest += 1;
    purgeDocumentMemory();
    isRendering = true;
    errorMsg = "";

    rotation = 0;
    flipH = false;
    flipV = false;
    activeTool = "pointer";
    showSettingsPopover = false;
    outlineList = [];
    pdfMetadata = { title: "", author: "", subject: "", keywords: "", creator: "" };
    rawSearchResults.data = [];
    totalSearchMatches = 0;
    isSearchingText = false;
    documentTextContext = "";
    readingTimeSeconds = 0;

    const isCurrentLoad = () =>
      isComponentMounted &&
      loadRequest === documentLoadRequest &&
      selectedFilePath === filePath;

    if (isPdf) {
      try {
        const pdfDoc = await pdfState.loadDocument(filePath);
        if (!isCurrentLoad()) return;
        if (!pdfDoc || !Number.isInteger(pdfDoc.numPages) || pdfDoc.numPages < 1) {
          throw new Error("PDF sayfaları okunamadı.");
        }

        pageOrder = Array.from({ length: pdfDoc.numPages }, (_, index) => index);

        try {
          const pdfjsOutline = await getDocumentOutline(pdfDoc);
          if (!isCurrentLoad()) return;
          if (pdfjsOutline && pdfjsOutline.length > 0) {
            outlineList = pdfjsOutline;
          } else {
            const nativeOutline = await invoke("get_pdf_outline", { path: filePath });
            if (!isCurrentLoad()) return;
            outlineList = Array.isArray(nativeOutline) ? nativeOutline : [];
          }

          const metadata = await invoke("get_pdf_metadata", { path: filePath });
          if (!isCurrentLoad()) return;
          pdfMetadata = metadata || { title: "", author: "", subject: "", keywords: "", creator: "" };
        } catch (err) {
          // Outline / Metadata error is non-critical, silent catch
        }
      } catch (err) {
        if (!isCurrentLoad()) return;
        if (!isPdfCancellationError(err)) {
          console.error("PDF yükleme hatası:", err);
          errorMsg = "PDF yüklenemedi: " + err.toString();
        }
        pageOrder = [];
        isRendering = false;
        isDocumentLoading = false;
        return;
      }
    } else {
      pageOrder = [0];
    }

    if (!isCurrentLoad()) return;

    currentPageIndex = 0;
    scale = 1.0;
    visualScale = 1.0;

    restoreDocumentPositionMemory(filePath);

    await tick();
    if (isCurrentLoad()) {
      isDocumentLoading = false;
      renderCurrentDocument();
    }
  }

  function renderCurrentDocument(options = {}) {
    if (!selectedFilePath || (!canvasRefLeft && !canvasRefRight) || isDocumentLoading) return;

    const request = ++renderRequest;
    const isSilent = options.silent === true;
    const filePath = selectedFilePath;
    const fileIsPdf = isPdf;
    const displayPageIndex = currentPageIndex;
    
    // Page index logic for book mode
    let leftDisplayIndex = displayPageIndex;
    let rightDisplayIndex = null;
    
    if ((isBookMode || isSplitView) && fileIsPdf && leftDisplayIndex > 0) {
      if (leftDisplayIndex % 2 === 0) leftDisplayIndex -= 1;
      if (leftDisplayIndex + 1 < pageOrder.length) rightDisplayIndex = leftDisplayIndex + 1;
    }

    const pageIndexLeft = pageOrder[leftDisplayIndex];
    const pageIndexRight = rightDisplayIndex !== null ? pageOrder[rightDisplayIndex] : null;

    if (renderAnimationFrame !== null) {
      cancelAnimationFrame(renderAnimationFrame);
      renderAnimationFrame = null;
    }
    
    // Anında DOM temizliği (Ghosting/Memory Leak önlemi)
    if (textLayerRefLeft) textLayerRefLeft.innerHTML = "";
    if (textLayerRefRight) textLayerRefRight.innerHTML = "";

    if (canvasRefLeft) void cancelCanvasRender(canvasRefLeft);
    if (canvasRefRight) void cancelCanvasRender(canvasRefRight);

    if (!isSilent) isRendering = true;
    errorMsg = "";

    renderAnimationFrame = requestAnimationFrame(async () => {
      renderAnimationFrame = null;
      const isCurrentRender = () =>
        isComponentMounted &&
        request === renderRequest &&
        selectedFilePath === filePath;

      if (!isCurrentRender()) return;

      try {
        const rawDpi = window.devicePixelRatio || 1.0;
        const cappedDpi = Math.min(rawDpi, 2.0);
        const targetScale = scale * cappedDpi;

        if (fileIsPdf) {
          if (!Number.isInteger(pageIndexLeft) || pageIndexLeft < 0) {
            throw new Error("Görüntülenecek PDF sayfası bulunamadı.");
          }

          if (pdfState.pdfDocProxy) {
            const renderTasks = [];

            if (canvasRefLeft) {
              renderTasks.push((async () => {
                const rendered = await renderPdfPage(pageIndexLeft, canvasRefLeft, targetScale, rotation);
                if (rendered && isCurrentRender() && textLayerRefLeft) {
                  void renderPdfTextLayer(pageIndexLeft, textLayerRefLeft, targetScale, rotation);
                }
              })());
            }

            if (canvasRefRight && pageIndexRight !== null) {
              renderTasks.push((async () => {
                const rendered = await renderPdfPage(pageIndexRight, canvasRefRight, targetScale, rotation);
                if (rendered && isCurrentRender() && textLayerRefRight) {
                  void renderPdfTextLayer(pageIndexRight, textLayerRefRight, targetScale, rotation);
                }
              })());
            }

            await Promise.all(renderTasks);

          } else {
            if (canvasRefLeft) {
              const rendered = await invoke("render_page", { path: filePath, pageIndex: pageIndexLeft, scale: targetScale });
              if (!isCurrentRender()) return;
              paintBufferToCanvas(canvasRefLeft, rendered);
            }
          }
        } else {
          if (canvasRefLeft) {
            const rendered = await invoke("read_image_file", { path: filePath, rotation, flipH, flipV });
            if (!isCurrentRender()) return;
            paintBufferToCanvas(canvasRefLeft, rendered);
          }
        }

        if (!isCurrentRender()) return;

        if (canvasRefLeft) overlayAnnotations(canvasRefLeft, leftDisplayIndex);
        if (canvasRefRight && rightDisplayIndex !== null) overlayAnnotations(canvasRefRight, rightDisplayIndex);

        saveDocumentPositionMemory();

      } catch (err) {
        if (isCurrentRender() && !isPdfCancellationError(err)) {
          console.error("Render hatası:", err);
          errorMsg = "Görüntüleme hatası: " + err.toString();
        }
      } finally {
        if (isCurrentRender()) isRendering = false;
      }
    });
  }

  /** Paint a raw pixel buffer {width, height, data} directly onto a canvas. */
  function paintBufferToCanvas(canvasObj, buffer) {
    if (!canvasObj || !buffer || !Number.isFinite(buffer.width) || !Number.isFinite(buffer.height)) {
      throw new Error("Geçersiz görüntü verisi.");
    }

    const { width, height, data } = buffer;
    const safeWidth = Math.max(1, Math.floor(width));
    const safeHeight = Math.max(1, Math.floor(height));
    if (!data || safeWidth * safeHeight * 4 > data.length) {
      throw new Error("Eksik görüntü piksel verisi.");
    }
    canvasObj.width = safeWidth;
    canvasObj.height = safeHeight;

    const ctx = canvasObj.getContext("2d", { alpha: false });
    if (!ctx) throw new Error("Canvas 2D bağlamı oluşturulamadı.");
    const clampedData = new Uint8ClampedArray(data);
    const imageData = new ImageData(clampedData, safeWidth, safeHeight);
    ctx.putImageData(imageData, 0, 0);
  }

  /** Draw all annotations for a given page onto the canvas. */
  function overlayAnnotations(canvasObj, pageIdx) {
    if (!canvasObj) return;
    const ctx = canvasObj.getContext("2d", { alpha: false });
    if (!ctx) return;

    const pageAnnos = annotationsMap[pageIdx] || [];
    for (const item of pageAnnos) {
      drawSingleAnnotation(ctx, item);
    }

    if (isDrawing && currentAnnoItem && pageIdx === currentPageIndex) {
      drawSingleAnnotation(ctx, currentAnnoItem);
    }

    // Draw all search match highlights present on this page
    if (totalSearchMatches > 0) {
      const origPageIndex = pageOrder[pageIdx] ?? pageIdx;
      const matchesOnPage = rawSearchResults.data.filter(m => m.page_index === origPageIndex);
      
      for (const match of matchesOnPage) {
        const isActive = (match.match_index === currentSearchMatchIndex);
        drawSearchMatchHighlight(ctx, canvasObj, match, isActive);
      }
    }
  }

  function drawSearchMatchHighlight(ctx, canvasObj, match, isActive = false) {
    if (!match || !match.itemBounds || match.itemBounds.length === 0) return;

    ctx.save();
    
    if (isActive) {
      ctx.fillStyle = "rgba(255, 85, 0, 0.4)"; // --accent-industrial with 40% opacity
      ctx.strokeStyle = "#ff5500";             // solid 1px border
      ctx.lineWidth = 1;
    } else {
      ctx.fillStyle = "rgba(255, 85, 0, 0.25)"; // --accent-industrial with 25% opacity
      ctx.strokeStyle = "transparent";
      ctx.lineWidth = 0;
    }

    const pageProxy = pdfState.pageCache.get(match.page_index);
    if (pageProxy) {
      const unscaledViewport = pageProxy.getViewport({ scale: 1.0, rotation: rotation });
      const safeScale = canvasObj.width / unscaledViewport.width;
      const viewport = pageProxy.getViewport({ scale: safeScale, rotation: rotation });

      for (const bound of match.itemBounds) {
        const [l, t, r, b] = viewport.convertToViewportRectangle([
          bound.x,
          bound.y,
          bound.x + bound.w,
          bound.y + bound.h,
        ]);

        const rectX = Math.min(l, r);
        const rectY = Math.min(t, b);
        const rectW = Math.max(Math.abs(r - l), 1); // Exact calculated width
        const rectH = Math.max(Math.abs(b - t), 1); // Exact calculated height

        ctx.beginPath();
        // Minimalist Industrial: sharp edges, no padding
        ctx.rect(rectX, rectY, rectW, rectH);
        
        ctx.fill();
        if (isActive) {
          ctx.stroke();
        }
      }
    }
    ctx.restore();
  }

  function drawSingleAnnotation(ctx, item) {
    ctx.save();
    ctx.strokeStyle = item.color;
    ctx.fillStyle = item.color;
    ctx.lineWidth = item.tool_type === "highlighter" ? item.size * 3 : item.size;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";

    if (item.tool_type === "highlighter") {
      ctx.globalAlpha = 0.4;
    } else {
      ctx.globalAlpha = 1.0;
    }

    switch (item.tool_type) {
      case "pen":
      case "highlighter":
        if (item.points && item.points.length > 1) {
          ctx.beginPath();
          ctx.moveTo(item.points[0].x, item.points[0].y);
          for (let i = 1; i < item.points.length; i++) {
            ctx.lineTo(item.points[i].x, item.points[i].y);
          }
          ctx.stroke();
        }
        break;

      case "rectangle":
        if (item.x !== undefined && item.y !== undefined && item.w !== undefined && item.h !== undefined) {
          ctx.beginPath();
          ctx.rect(item.x, item.y, item.w, item.h);
          ctx.stroke();
        }
        break;

      case "arrow":
        if (item.x !== undefined && item.y !== undefined && item.end_x !== undefined && item.end_y !== undefined) {
          drawArrow(ctx, item.x, item.y, item.end_x, item.end_y, item.size);
        }
        break;

      case "text":
        if (item.x !== undefined && item.y !== undefined && item.text) {
          ctx.font = `${Math.max(14, item.size * 3)}px Inter, sans-serif`;
          ctx.fillText(item.text, item.x, item.y);
        }
        break;
    }
    ctx.restore();
  }

  function drawArrow(ctx, fromx, fromy, tox, toy, width) {
    const headlen = Math.max(12, width * 3);
    const dx = tox - fromx;
    const dy = toy - fromy;
    const angle = Math.atan2(dy, dx);

    ctx.beginPath();
    ctx.moveTo(fromx, fromy);
    ctx.lineTo(tox, toy);
    ctx.stroke();

    ctx.beginPath();
    ctx.moveTo(tox, toy);
    ctx.lineTo(tox - headlen * Math.cos(angle - Math.PI / 6), toy - headlen * Math.sin(angle - Math.PI / 6));
    ctx.lineTo(tox - headlen * Math.cos(angle + Math.PI / 6), toy - headlen * Math.sin(angle + Math.PI / 6));
    ctx.closePath();
    ctx.fill();
  }

  // Metadata Save Handler
  async function handleSaveMetadata(e) {
    const updatedMeta = e.detail;
    try {
      const savePath = await save({
        title: "Meta Verileri Güncellenmiş PDF'i Kaydet",
        defaultPath: fileName.substring(0, fileName.lastIndexOf(".")) + "_meta.pdf",
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });

      if (savePath) {
        await invoke("set_pdf_metadata", {
          sourcePath: selectedFilePath,
          metadata: updatedMeta,
          targetPath: savePath,
        });
        pdfMetadata = updatedMeta;
        showToast("PDF meta verileri güncellendi ve kaydedildi!");
      }
    } catch (err) {
      console.error("Meta veri kaydetme hatası:", err);
      showToast("Meta veri kaydetme hatası: " + err.toString());
    }
  }

  // High-Res Page to Image / SVG Export Handler
  async function handleExportPageImage() {
    try {
      const savePath = await save({
        title: `Sayfa ${currentPageIndex + 1} Görsel/SVG Olarak Aktar`,
        defaultPath: `${fileName.substring(0, fileName.lastIndexOf("."))}_sayfa_${currentPageIndex + 1}.svg`,
        filters: [
          { name: "Vektörel SVG (*.svg)", extensions: ["svg"] },
          { name: "PNG Görsel (*.png)", extensions: ["png"] },
          { name: "JPEG Görsel (*.jpg)", extensions: ["jpg"] },
          { name: "WebP Görsel (*.webp)", extensions: ["webp"] },
        ],
      });

      if (savePath) {
        const ext = savePath.split(".").pop()?.toLowerCase() || "svg";
        await invoke("export_pdf_page_to_image", {
          path: selectedFilePath,
          pageIndex: currentPageIndex,
          targetPath: savePath,
          format: ext,
        });
        showToast(`Sayfa ${currentPageIndex + 1} ${ext.toUpperCase()} olarak aktarıldı!`);
      }
    } catch (err) {
      console.error("Dışa aktarma hatası:", err);
      showToast("Dışa aktarma hatası: " + err.toString());
    }
  }

  // Bionic Reading Handler
  async function handleOpenBionicReading() {
    showToast("Biyonik okuma metni hazırlanıyor...");
    try {
      let text = documentTextContext;
      if (!text) {
        text = await invoke("perform_ocr_on_page", {
          path: selectedFilePath,
          pageIndex: currentPageIndex,
        });
      }
      bionicText = text;
      isBionicModalOpen = true;
    } catch (err) {
      console.error("Biyonik metin hatası:", err);
      showToast("Biyonik metin hazırlanamadı: " + err.toString());
    }
  }

  // Continuity Camera Handler
  async function handleContinuityCamera() {
    if (!isPdf) {
      showToast("Süreklilik kamerası taraması sadece PDF belgeleri için aktiftir.");
      return;
    }
    showToast("iPhone / iPad Süreklilik Kamerası bekleniyor...");
    try {
      const msg = await invoke("trigger_continuity_camera_scan", {
        sourcePdfPath: selectedFilePath,
      });
      showToast(msg);
      await loadDocumentFile(selectedFilePath);
    } catch (err) {
      console.error("Kamera tarama hatası:", err);
      showToast("Kamera taraması: " + err.toString());
    }
  }

  // macOS Text-to-Speech Handler
  async function handleSpeakText() {
    if (isSpeaking) {
      await invoke("stop_speaking");
      isSpeaking = false;
      showToast("Sesli okuma durduruldu.");
      return;
    }

    try {
      showToast("Sayfa metni seslendiriliyor (macOS TTS)...");
      isSpeaking = true;
      let textToRead = documentTextContext;
      if (!textToRead) {
        textToRead = await invoke("perform_ocr_on_page", {
          path: selectedFilePath,
          pageIndex: currentPageIndex,
        });
      }
      await invoke("speak_text", { text: textToRead, voice: "Yelda" });
    } catch (err) {
      console.error("TTS hatası:", err);
      isSpeaking = false;
      showToast("Seslendirme hatası: " + err.toString());
    }
  }

  // macOS Share Sheet Handler
  async function handleShareMacOS() {
    try {
      await invoke("share_file_macos", { filePath: selectedFilePath });
    } catch (err) {
      console.error("Share Sheet hatası:", err);
      showToast("Paylaşım hatası: " + err.toString());
    }
  }

  // Apple Vision Native OCR Handler
  async function handleRunOCR() {
    showToast("Apple Vision OCR çalıştırılıyor...");
    try {
      const text = await invoke("perform_ocr_on_page", {
        path: selectedFilePath,
        pageIndex: currentPageIndex,
      });
      documentTextContext = text;
      showToast("OCR tamamlandı! Metin taptaze tanındı.");
      alert(`Sayfa ${currentPageIndex + 1} OCR Sonucu:\n\n` + text);
    } catch (err) {
      console.error("OCR hatası:", err);
      showToast("OCR yürütülemedi: " + err.toString());
    }
  }

  // Text Search Methods (PDF.js getTextContent + Visual Match Highlighting)
  async function handleSearchQuery(e) {
    const query = typeof e.detail === "string" ? e.detail.trim() : "";
    
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
    if (searchAbortController) searchAbortController.abort();
    
    if (!query || query.length < 2 || !isPdf) {
      rawSearchResults.data = [];
      totalSearchMatches = 0;
      currentSearchMatchIndex = 0;
      isSearchingText = false;
      renderCurrentDocument({ silent: true });
      return;
    }

    searchDebounceTimer = setTimeout(async () => {
      searchAbortController = new AbortController();
      const signal = searchAbortController.signal;
      const request = ++searchRequest;

      const filePath = selectedFilePath;
      const documentProxy = pdfState.pdfDocProxy;
      const isCurrentSearch = () =>
        isComponentMounted &&
        request === searchRequest &&
        selectedFilePath === filePath &&
        pdfState.pdfDocProxy === documentProxy &&
        !signal.aborted;

      if (!documentProxy) {
        rawSearchResults.data = [];
        totalSearchMatches = 0;
        currentSearchMatchIndex = 0;
        return;
      }

      isSearchingText = true;
      rawSearchResults.data = [];
      totalSearchMatches = 0;
      currentSearchMatchIndex = 0;

      try {
        const { results: pdfResults, limitExceeded } = await searchPdfDocument(query, documentProxy, isCurrentSearch, signal);
        if (!isCurrentSearch()) return;
        
        if (pdfResults && pdfResults.length > 0) {
          rawSearchResults.data = pdfResults;
          totalSearchMatches = pdfResults.length;
          currentSearchMatchIndex = 0;
          jumpToSearchMatch(0);
          if (limitExceeded) {
            showToast("5000+ eşleşme bulundu. Tarama durduruldu.");
          }
        } else {
          const nativeResults = await invoke("search_pdf_text", {
            path: filePath,
            query: query,
          });
          if (!isCurrentSearch()) return;
          const nResults = Array.isArray(nativeResults) ? nativeResults : [];
          rawSearchResults.data = nResults;
          totalSearchMatches = nResults.length;
          currentSearchMatchIndex = 0;
          if (totalSearchMatches > 0) {
            jumpToSearchMatch(0);
          } else {
            renderCurrentDocument({ silent: true });
          }
        }
      } catch (err) {
        if (isCurrentSearch() && !isPdfCancellationError(err)) {
          console.error("Metin arama hatası:", err);
        }
      } finally {
        if (isCurrentSearch()) isSearchingText = false;
      }
    }, 250);
  }

  function jumpToSearchMatch(index) {
    if (Number.isInteger(index) && rawSearchResults.data[index]) {
      const targetPageIndex = rawSearchResults.data[index].page_index;
      const displayIdx = pageOrder.findIndex((origIdx) => origIdx === targetPageIndex);
      if (displayIdx !== -1) {
        currentPageIndex = displayIdx;
      } else if (Number.isInteger(targetPageIndex) && targetPageIndex >= 0 && targetPageIndex < pageOrder.length) {
        currentPageIndex = targetPageIndex;
      } else {
        return;
      }
      renderCurrentDocument();
    }
  }

  function handleSearchNext() {
    if (totalSearchMatches === 0) return;
    currentSearchMatchIndex = (currentSearchMatchIndex + 1) % totalSearchMatches;
    jumpToSearchMatch(currentSearchMatchIndex);
  }

  function handleSearchPrev() {
    if (totalSearchMatches === 0) return;
    currentSearchMatchIndex = (currentSearchMatchIndex - 1 + totalSearchMatches) % totalSearchMatches;
    jumpToSearchMatch(currentSearchMatchIndex);
  }

  // Markdown Export (.md)
  async function handleExportNotesMarkdown() {
    let mdContent = `# Belge Notları & Vurguları: ${fileName}\n\n`;
    mdContent += `*Oluşturulma Tarihi: ${new Date().toLocaleString()}*\n\n---\n\n`;

    let totalNotes = 0;
    Object.keys(annotationsMap).forEach((pageIdxStr) => {
      const pageNum = parseInt(pageIdxStr, 10) + 1;
      const annos = annotationsMap[pageIdxStr];
      if (annos && annos.length > 0) {
        mdContent += `## Sayfa ${pageNum}\n`;
        annos.forEach((item) => {
          totalNotes++;
          if (item.tool_type === "text") {
            mdContent += `- 📝 **Metin Notu**: "${item.text}" *(Renk: ${item.color})*\n`;
          } else if (item.tool_type === "highlighter") {
            mdContent += `- 🟡 **Fosforlu Vurgu**: Vurgulanan alan *(Renk: ${item.color})*\n`;
          } else if (item.tool_type === "pen") {
            mdContent += `- ✒️ **Serbest Çizim**: ${item.points?.length || 0} nokta\n`;
          } else if (item.tool_type === "rectangle") {
            mdContent += `- ⏹️ **Dikdörtgen Vurgu**: (${item.x}, ${item.y}) [${item.w}x${item.h}]\n`;
          } else if (item.tool_type === "arrow") {
            mdContent += `- ↗️ **Ok Çizimi**: (${item.x}, ${item.y}) -> (${item.end_x}, ${item.end_y})\n`;
          }
        });
        mdContent += `\n`;
      }
    });

    if (totalNotes === 0) {
      showToast("Dışa aktarılacak henüz not veya vurgu yok!");
      return;
    }

    try {
      const savePath = await save({
        title: "Notları Markdown (.md) Olarak Kaydet",
        defaultPath: fileName.substring(0, fileName.lastIndexOf(".")) + "_Notlar.md",
        filters: [{ name: "Markdown Belgesi (*.md)", extensions: ["md"] }],
      });

      if (savePath) {
        showToast("Notlar Markdown belgesi olarak aktarıldı!");
      }
    } catch (err) {
      console.error("Markdown aktarma hatası:", err);
    }
  }

  // PDF Sayfa Bölme (Split Selected Pages)
  async function handleSplitPdf() {
    const input = prompt("Dışa aktarılacak sayfa numaralarını girin (örn: 1, 2, 5 veya 1-3):");
    if (!input) return;

    try {
      const pageIndices = [];
      const parts = input.split(",");
      parts.forEach((p) => {
        const trimmed = p.trim();
        if (trimmed.includes("-")) {
          const [start, end] = trimmed.split("-").map((n) => parseInt(n, 10));
          if (!isNaN(start) && !isNaN(end)) {
            for (let i = start; i <= end; i++) {
              if (i >= 1 && i <= pageOrder.length) pageIndices.push(i - 1);
            }
          }
        } else {
          const num = parseInt(trimmed, 10);
          if (!isNaN(num) && num >= 1 && num <= pageOrder.length) {
            pageIndices.push(num - 1);
          }
        }
      });

      if (pageIndices.length === 0) {
        showToast("Geçerli sayfa numarası girilmedi!");
        return;
      }

      const savePath = await save({
        title: "Bölünen Sayfaları Yeni PDF Olarak Kaydet",
        defaultPath: fileName.substring(0, fileName.lastIndexOf(".")) + "_Bölünmüş.pdf",
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });

      if (savePath) {
        await invoke("extract_pages", {
          sourcePath: selectedFilePath,
          pageIndices: pageIndices,
          targetPath: savePath,
        });
        showToast(`${pageIndices.length} sayfa yeni PDF belgesine bölündü!`);
      }
    } catch (err) {
      console.error("Bölme hatası:", err);
      errorMsg = "Sayfa bölme başarısız: " + err.toString();
    }
  }

  // Command Palette Dispatch Handler
  function handleCommandSelect(e) {
    const cmdId = e.detail;
    switch (cmdId) {
      case "open-metadata":
        isMetadataModalOpen = true;
        break;
      case "export-page-image":
        handleExportPageImage();
        break;
      case "open-bionic":
        handleOpenBionicReading();
        break;
      case "toggle-ruler":
        isReadingRulerActive = !isReadingRulerActive;
        showToast(isReadingRulerActive ? "Odak Cetveli (Line Focus) Açıldı" : "Odak Cetveli Kapatıldı");
        break;
      case "open-analytics":
        isAnalyticsModalOpen = true;
        break;
      case "camera-scan":
        handleContinuityCamera();
        break;
      case "speak-page":
        handleSpeakText();
        break;
      case "stop-speak":
        invoke("stop_speaking");
        isSpeaking = false;
        showToast("Okuma durduruldu.");
        break;
      case "share-macos":
        handleShareMacOS();
        break;
      case "open-ai-assistant":
        isAiModalOpen = true;
        break;
      case "run-ocr-page":
        handleRunOCR();
        break;
      case "toggle-dark-mode":
        isSmartDarkMode = !isSmartDarkMode;
        showToast(isSmartDarkMode ? "Akıllı Gece Modu Açıldı" : "Normal Mod");
        break;
      case "toggle-book-mode":
        toggleBookMode();
        break;
      case "toggle-split-view":
        isSplitView = !isSplitView;
        renderCurrentDocument();
        showToast(isSplitView ? "Çift Görünüm (Split View) Aktif" : "Tek Görünüm");
        break;
      case "toggle-sidebar":
        showSidebar = !showSidebar;
        break;
      case "open-search":
        isSearchBarOpen = true;
        break;
      case "export-notes-md":
        handleExportNotesMarkdown();
        break;
      case "split-pdf":
        handleSplitPdf();
        break;
      case "merge-pdf":
        handleMergePdf();
        break;
      case "reset-zoom":
        resetZoom();
        break;
      case "burn-in-pdf":
        handleBurnInExport();
        break;
    }
  }

  // Canvas Drawing Events
  function getCanvasCoords(e) {
    const targetCanvas = (isBookMode && e.target === canvasRefRight) ? canvasRefRight : canvasRefLeft;
    if (!targetCanvas) return null;
    const rect = targetCanvas.getBoundingClientRect();
    if (!rect.width || !rect.height || !targetCanvas.width || !targetCanvas.height) return null;
    const scaleX = targetCanvas.width / rect.width;
    const scaleY = targetCanvas.height / rect.height;
    return {
      x: (e.clientX - rect.left) * scaleX,
      y: (e.clientY - rect.top) * scaleY,
    };
  }

  function handleCanvasMouseDown(e) {
    if (activeTool === "pointer") return;

    const coords = getCanvasCoords(e);
    if (!coords) return;
    isDrawing = true;
    startPos = coords;

    if (activeTool === "pen" || activeTool === "highlighter") {
      currentPoints = [coords];
      currentAnnoItem = {
        page_index: currentPageIndex,
        tool_type: activeTool,
        points: currentPoints,
        color: strokeColor,
        size: strokeSize,
        canvas_width: targetCanvas.width,
        canvas_height: targetCanvas.height,
      };
    } else if (activeTool === "text") {
      const userText = prompt("Metin girin:");
      if (userText) {
        const textAnno = {
          page_index: currentPageIndex,
          tool_type: "text",
          x: coords.x,
          y: coords.y,
          text: userText,
          color: strokeColor,
          size: strokeSize,
          canvas_width: targetCanvas.width,
          canvas_height: targetCanvas.height,
        };
        addAnnotationToCurrentPage(textAnno);
      }
      isDrawing = false;
    }
  }

  function handleCanvasMouseMove(e) {
    if (!isDrawing || activeTool === "pointer") return;

    const coords = getCanvasCoords(e);
    if (!coords) return;

    if (activeTool === "pen" || activeTool === "highlighter") {
      currentPoints.push(coords);
      currentAnnoItem.points = currentPoints;
    } else if (activeTool === "rectangle") {
      currentAnnoItem = {
        page_index: currentPageIndex,
        tool_type: "rectangle",
        x: Math.min(startPos.x, coords.x),
        y: Math.min(startPos.y, coords.y),
        w: Math.abs(coords.x - startPos.x),
        h: Math.abs(coords.y - startPos.y),
        color: strokeColor,
        size: strokeSize,
        canvas_width: targetCanvas.width,
        canvas_height: targetCanvas.height,
      };
    } else if (activeTool === "arrow") {
      currentAnnoItem = {
        page_index: currentPageIndex,
        tool_type: "arrow",
        x: startPos.x,
        y: startPos.y,
        end_x: coords.x,
        end_y: coords.y,
        color: strokeColor,
        size: strokeSize,
        canvas_width: targetCanvas.width,
        canvas_height: targetCanvas.height,
      };
    }

    renderCurrentDocument({ silent: true });
  }

  function handleCanvasMouseUp() {
    if (!isDrawing) return;
    isDrawing = false;

    if (currentAnnoItem) {
      addAnnotationToCurrentPage(currentAnnoItem);
      currentAnnoItem = null;
      currentPoints = [];
    }
  }

  function addAnnotationToCurrentPage(item) {
    if (!annotationsMap[currentPageIndex]) {
      annotationsMap[currentPageIndex] = [];
    }
    annotationsMap[currentPageIndex].push(item);
    annotationsMap = { ...annotationsMap };
    renderCurrentDocument({ silent: true });
  }

  function undoLastAnnotation() {
    if (annotationsMap[currentPageIndex] && annotationsMap[currentPageIndex].length > 0) {
      annotationsMap[currentPageIndex].pop();
      annotationsMap = { ...annotationsMap };
      renderCurrentDocument({ silent: true });
      showToast("Son çizim geri alındı.");
    }
  }

  function clearAllAnnotations() {
    annotationsMap[currentPageIndex] = [];
    annotationsMap = { ...annotationsMap };
    renderCurrentDocument({ silent: true });
    showToast("Sayfadaki tüm çizimler temizlendi.");
  }

  async function toggleBookMode() {
    isBookMode = !isBookMode;
    await tick();
    renderCurrentDocument();
    showToast(isBookMode ? "Kitap Modu Aktif" : "Tek Sayfa Modu Aktif");
  }

  function goToNextPage() {
    if (pageOrder.length === 0) return;
    let step = 1;
    if (isBookMode || isSplitView) {
      step = (currentPageIndex === 0) ? 1 : 2;
    }
    
    if (currentPageIndex + step < pageOrder.length) {
      currentPageIndex += step;
      if ((isBookMode || isSplitView) && currentPageIndex > 0 && currentPageIndex % 2 === 0) {
         currentPageIndex -= 1;
      }
      renderCurrentDocument();
    } else if (currentPageIndex < pageOrder.length - 1) {
      currentPageIndex = pageOrder.length - 1;
      if ((isBookMode || isSplitView) && currentPageIndex > 0 && currentPageIndex % 2 === 0) {
         currentPageIndex -= 1;
      }
      renderCurrentDocument();
    }
  }

  function goToPrevPage() {
    if (pageOrder.length === 0) return;
    let step = 1;
    if (isBookMode || isSplitView) {
      if (currentPageIndex <= 2) {
        currentPageIndex = 0;
        renderCurrentDocument();
        return;
      } else {
        step = 2;
      }
    }

    if (currentPageIndex - step >= 0) {
      currentPageIndex -= step;
      if ((isBookMode || isSplitView) && currentPageIndex > 0 && currentPageIndex % 2 === 0) {
         currentPageIndex -= 1;
      }
      renderCurrentDocument();
    } else if (currentPageIndex > 0) {
      currentPageIndex = 0;
      renderCurrentDocument();
    }
  }

  function setDebouncedZoom(targetScale) {
    const nextScale = Math.min(Math.max(targetScale, 0.25), 4.0);
    const newVisualScale = nextScale / scale;
    visualScale = newVisualScale;
    isZooming = true;

    if (zoomDebounceTimer) clearTimeout(zoomDebounceTimer);
    zoomDebounceTimer = setTimeout(() => {
      scale = parseFloat(nextScale.toFixed(2));
      visualScale = 1.0;
      isZooming = false;
      renderCurrentDocument({ silent: true });
    }, 250);
  }

  function zoomIn() {
    const effectiveScale = scale * visualScale;
    if (effectiveScale < 4.0) {
      setDebouncedZoom(effectiveScale + 0.2);
    }
  }

  function zoomOut() {
    const effectiveScale = scale * visualScale;
    if (effectiveScale > 0.25) {
      setDebouncedZoom(effectiveScale - 0.2);
    }
  }

  function resetZoom() {
    setDebouncedZoom(1.0);
  }

  function handlePageInputChange(e) {
    if (pageOrder.length === 0) return;
    const val = parseInt(e.target.value, 10);
    if (!isNaN(val) && val >= 1 && val <= pageOrder.length) {
      currentPageIndex = val - 1;
      renderCurrentDocument();
    } else {
      e.target.value = currentPageIndex + 1;
    }
  }

  // Export Actions
  async function handleBurnInExport() {
    const allItems = [];
    Object.keys(annotationsMap).forEach((pageIdxStr) => {
      const pageIdx = parseInt(pageIdxStr, 10);
      const annos = annotationsMap[pageIdxStr];
      annos.forEach((anno) => {
        allItems.push({ ...anno, page_index: pageIdx });
      });
    });

    if (allItems.length === 0) {
      showToast("Kalıcı işlenecek henüz çizim yapılmadı!");
      return handleExportPdfStructure();
    }

    try {
      const savePath = await save({
        title: "Çizimler İşlenmiş PDF'i Kaydet",
        defaultPath: fileName.substring(0, fileName.lastIndexOf(".")) + "_çizimli.pdf",
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });

      if (savePath) {
        await invoke("burn_annotations_to_pdf", {
          sourcePath: selectedFilePath,
          annotationsJson: JSON.stringify(allItems),
          targetPath: savePath,
        });
        showToast("Çizimler PDF üzerine kalıcı olarak işlendi!");
      }
    } catch (err) {
      console.error("Burn-in hatası:", err);
      errorMsg = "Kalıcı işleme başarısız: " + err.toString();
    }
  }

  function handleReorderPages(newOrder) {
    pageOrder = newOrder;
    if (currentPageIndex >= pageOrder.length) {
      currentPageIndex = pageOrder.length - 1;
    }
    renderCurrentDocument();
    showToast("Sayfa sırası değiştirildi!");
  }

  function handleDeletePage(displayIndex) {
    if (pageOrder.length <= 1) {
      showToast("Tek sayfalı bir belgeden son sayfa silinemez!");
      return;
    }

    pageOrder = pageOrder.filter((_, idx) => idx !== displayIndex);
    if (currentPageIndex >= pageOrder.length) {
      currentPageIndex = pageOrder.length - 1;
    }
    renderCurrentDocument();
    showToast(`Sayfa ${displayIndex + 1} kaldırıldı.`);
  }

  async function handleMergePdf() {
    try {
      const selected = await open({
        title: "Birleştirilecek İkinci PDF Belgesini Seçin",
        multiple: false,
        filters: [{ name: "PDF Belgesi", extensions: ["pdf"] }],
      });

      if (!selected) return;
      const secondPdfPath = typeof selected === "string" ? selected : selected.path;

      const targetPath = await save({
        title: "Birleştirilen PDF Belgesini Kaydet",
        defaultPath: "Birleştirilmiş_Doküman.pdf",
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });

      if (targetPath) {
        await invoke("merge_pdfs", {
          pdfPaths: [selectedFilePath, secondPdfPath],
          targetPath: targetPath,
        });
        showToast("PDF belgeleri başarıyla birleştirildi!");
        await loadDocumentFile(targetPath);
      }
    } catch (err) {
      console.error("PDF birleştirme hatası:", err);
      errorMsg = "Birleştirme başarısız: " + err.toString();
    }
  }

  async function handleExportPdfStructure() {
    if (!isPdf) return handleExportImage();

    try {
      const savePath = await save({
        title: "Düzenlenen PDF Belgesini Dışa Aktar",
        defaultPath: fileName.substring(0, fileName.lastIndexOf(".")) + "_düzenlendi.pdf",
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });

      if (savePath) {
        await invoke("reorder_pdf_pages", {
          sourcePath: selectedFilePath,
          pageOrder: pageOrder,
          targetPath: savePath,
        });
        showToast("Yeni PDF belgesi başarıyla dışa aktarıldı!");
      }
    } catch (err) {
      console.error("Dışa aktarma hatası:", err);
      errorMsg = "Dışa aktarma başarısız: " + err.toString();
    }
  }

  function rotateLeft() {
    rotation = (rotation - 90 + 360) % 360;
    renderCurrentDocument();
  }

  function rotateRight() {
    rotation = (rotation + 90) % 360;
    renderCurrentDocument();
  }

  function toggleFlipH() {
    flipH = !flipH;
    renderCurrentDocument();
  }

  function toggleFlipV() {
    flipV = !flipV;
    renderCurrentDocument();
  }

  async function handleExportImage() {
    try {
      const savePath = await save({
        title: "Farklı Kaydet ve Format Dönüştür",
        defaultPath: fileName.substring(0, fileName.lastIndexOf(".")) + "_dönüştürüldü",
        filters: [
          { name: "WebP Görsel (*.webp)", extensions: ["webp"] },
          { name: "PNG Görsel (*.png)", extensions: ["png"] },
          { name: "JPEG Görsel (*.jpg)", extensions: ["jpg"] },
          { name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] },
        ],
      });

      if (savePath) {
        const targetExt = savePath.split(".").pop()?.toLowerCase() || "png";
        await invoke("convert_and_save_image", {
          sourcePath: selectedFilePath,
          targetPath: savePath,
          format: targetExt,
          rotation: rotation,
          flipH: flipH,
          flipV: flipV,
        });
        showToast(`Dosya başarıyla ${targetExt.toUpperCase()} formatında kaydedildi!`);
      }
    } catch (err) {
      console.error("Dönüştürme hatası:", err);
      errorMsg = "Dönüştürme başarısız: " + err.toString();
    }
  }
</script>

<svelte:window on:keydown={handleKeyDown} />

<!-- Reading Ruler Focus Overlay -->
<ReadingRuler isActive={isReadingRulerActive} />

<!-- PDF Metadata Modal -->
<MetadataModal
  bind:isOpen={isMetadataModalOpen}
  metadata={pdfMetadata}
  on:save={handleSaveMetadata}
  on:close={() => (isMetadataModalOpen = false)}
/>

<!-- Bionic Reading Modal -->
<BionicReadingModal
  bind:isOpen={isBionicModalOpen}
  text={bionicText}
  on:close={() => (isBionicModalOpen = false)}
/>

<!-- Session Reading Analytics Modal -->
<AnalyticsModal
  bind:isOpen={isAnalyticsModalOpen}
  {fileName}
  totalPages={pageOrder.length}
  currentPage={currentPageIndex + 1}
  {readingTimeSeconds}
  {scale}
  on:close={() => (isAnalyticsModalOpen = false)}
/>

<!-- Command Palette Modal -->
<CommandPalette
  bind:isOpen={isCmdPaletteOpen}
  on:select={handleCommandSelect}
  on:close={() => (isCmdPaletteOpen = false)}
/>

<!-- Local AI Assistant Modal -->
<AiAssistantModal
  bind:isOpen={isAiModalOpen}
  documentContext={documentTextContext}
  on:close={() => (isAiModalOpen = false)}
/>

<main class="app-container">
  <!-- Telemetry Header Seal / System Banner -->
  <header class="telemetry-header">
    <div class="telemetry-seal">
      <span class="system-tag">{selectedFilePath ? `[SYS_DOC // ${fileExt.toUpperCase()}]` : "[SYS_IDLE]"}</span>
      <span class="file-title-raw" title={selectedFilePath}>{selectedFilePath ? fileName : "NO_DOCUMENT_LOADED"}</span>
    </div>

    <div class="header-actions">
      {#if isPdf}
        <button
          class="hud-btn {showSidebar ? 'active' : ''}"
          on:click={() => (showSidebar = !showSidebar)}
          title="Sol Paneli Aç/Kapat (Cmd + B)"
        >
          [SIDEBAR]
        </button>
      {/if}

      <button class="hud-btn" on:click={handleSelectFile} title="Farklı Dosya Aç (Cmd + O)">
        [OPEN]
      </button>

      <button class="hud-btn" on:click={() => (isCmdPaletteOpen = true)} title="Komut Paleti (Cmd + K)">
        [CMD]
      </button>
    </div>
  </header>

  <!-- Floating In-Document Search Bar (Cmd + F) -->
  <SearchBar
    isOpen={isSearchBarOpen}
    totalMatches={totalSearchMatches}
    currentMatchIndex={currentSearchMatchIndex}
    isSearching={isSearchingText}
    on:search={handleSearchQuery}
    on:next={handleSearchNext}
    on:prev={handleSearchPrev}
    on:close={() => (isSearchBarOpen = false)}
  />

  <!-- Main Workspace -->
  <div class="workspace-body">
    {#if selectedFilePath && isPdf && showSidebar}
      <Sidebar
        filePath={selectedFilePath}
        {pageOrder}
        {currentPageIndex}
        {outlineList}
        onSelectPage={(idx) => {
          currentPageIndex = idx;
          renderCurrentDocument();
        }}
        onReorderPages={handleReorderPages}
        onDeletePage={handleDeletePage}
        onMergePdf={handleMergePdf}
        onSplitPdf={handleSplitPdf}
      />
    {/if}

    <div class="viewer-area" bind:this={viewerAreaRef}>
      {#if !selectedFilePath}
        <button
          class="drop-zone"
          on:click={handleSelectFile}
          style="all: unset; display: flex; flex-direction: column; align-items: center; padding: 48px 36px; border: 1px dashed var(--border-raw); background: var(--bg-surface); text-align: center; cursor: pointer; font-family: var(--font-mono);"
        >
          <div style="width: 56px; height: 56px; border: 1px solid var(--border-raw); color: var(--accent-industrial); display: flex; align-items: center; justify-content: center; margin-bottom: 16px; font-weight: 700; font-size: 20px;">
            [+]
          </div>
          <h2 style="font-size: 16px; font-weight: 700; margin-bottom: 6px; color: var(--text-primary); text-transform: uppercase;">// DOSYA_YÜKLEME_MERKEZİ</h2>
          <p style="color: var(--text-muted); font-size: 12px; margin-bottom: 20px; max-width: 340px;">Görüntülemek için bilgisayarınızdan PDF veya görsel belgesi seçin ya da pencereye sürükleyin</p>
          <span class="hud-btn active" style="padding: 4px 16px;">[ DOSYA SEÇ ]</span>
        </button>
      {:else}
        <!-- 60FPS GPU Accelerated Canvas Container + Smart Dark Mode -->
        <div
          class="canvas-container {isBookMode || isSplitView ? 'book-mode' : ''} {isSmartDarkMode ? 'smart-dark-mode' : ''}"
          style="transform: scale({visualScale}); transform-origin: top center; will-change: transform; transition: {isZooming ? 'none' : 'transform 0.2s ease-out'};"
        >
          {#if isRendering}
            <div class="loading-overlay">
              <div class="spinner"></div>
              <p style="font-size: 11px; font-weight: 700; color: var(--accent-industrial);">// RENDER_İŞLENİYOR...</p>
            </div>
          {/if}

          {#if errorMsg}
            <div class="error-card">
              <p>{errorMsg}</p>
            </div>
          {/if}

          <!-- Left / Single Canvas -->
          <div class="page-wrapper">
            <canvas
              bind:this={canvasRefLeft}
              class={activeTool === 'pointer' ? 'pointer-mode' : ''}
              on:mousedown={handleCanvasMouseDown}
              on:mousemove={handleCanvasMouseMove}
              on:mouseup={handleCanvasMouseUp}
              on:mouseleave={handleCanvasMouseUp}
            ></canvas>
            {#if isPdf}
              <div bind:this={textLayerRefLeft} class="textLayer"></div>
            {/if}
          </div>

          <!-- Book Mode: Right Canvas -->
          {#if (isBookMode || isSplitView) && isPdf && currentPageIndex > 0 && currentPageIndex + 1 < pageOrder.length}
            <div class="book-divider"></div>
            <div class="page-wrapper">
              <canvas
                bind:this={canvasRefRight}
                class={activeTool === 'pointer' ? 'pointer-mode' : ''}
                on:mousedown={handleCanvasMouseDown}
                on:mousemove={handleCanvasMouseMove}
                on:mouseup={handleCanvasMouseUp}
                on:mouseleave={handleCanvasMouseUp}
              ></canvas>
              <div bind:this={textLayerRefRight} class="textLayer"></div>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </div>

  <!-- Utilitarian HUD Dock (Fixed Control Panel) -->
  {#if selectedFilePath}
    <div class="hud-dock">
      {#if isPdf}
        <div class="hud-group">
          <button class="hud-btn" on:click={goToPrevPage} disabled={currentPageIndex === 0}>[PREV]</button>
          <div class="hud-telemetry-badge">
            <span>PG:</span>
            <input
              type="number"
              class="hud-page-input"
              value={pageOrder.length > 0 ? currentPageIndex + 1 : 0}
              min="1"
              max={Math.max(1, pageOrder.length)}
              disabled={pageOrder.length === 0}
              on:change={handlePageInputChange}
            />
            <span>/ {String(pageOrder.length).padStart(3, '0')}</span>
          </div>
          <button class="hud-btn" on:click={goToNextPage} disabled={currentPageIndex >= pageOrder.length - 1}>[NEXT]</button>
        </div>
      {:else}
        <div class="hud-group">
          <button class="hud-btn" on:click={rotateLeft}>[SOLA 90°]</button>
          <button class="hud-btn" on:click={rotateRight}>[SAĞA 90°]</button>
          <button class="hud-btn {flipH ? 'active' : ''}" on:click={toggleFlipH}>[FLIP_H]</button>
          <button class="hud-btn {flipV ? 'active' : ''}" on:click={toggleFlipV}>[FLIP_V]</button>
        </div>
      {/if}

      <div class="hud-group">
        <button class="hud-btn" on:click={zoomOut} disabled={scale <= 0.25}>[Z:-]</button>
        <button class="hud-btn" on:click={resetZoom}>[Z: {Math.round(scale * visualScale * 100)}%]</button>
        <button class="hud-btn" on:click={zoomIn} disabled={scale >= 4.0}>[Z:+]</button>
      </div>

      <div class="hud-group">
        <button class="hud-btn {activeTool === 'pointer' ? 'active' : ''}" on:click={() => (activeTool = "pointer")}>[PTR]</button>
        <button class="hud-btn {activeTool === 'pen' ? 'active' : ''}" on:click={() => (activeTool = "pen")}>[PEN]</button>
        <button class="hud-btn {activeTool === 'highlighter' ? 'active' : ''}" on:click={() => (activeTool = "highlighter")}>[HL]</button>
        <button class="hud-btn {activeTool === 'rectangle' ? 'active' : ''}" on:click={() => (activeTool = "rectangle")}>[RECT]</button>
        <button class="hud-btn {activeTool === 'arrow' ? 'active' : ''}" on:click={() => (activeTool = "arrow")}>[ARR]</button>
        <button class="hud-btn {activeTool === 'text' ? 'active' : ''}" on:click={() => (activeTool = "text")}>[TXT]</button>
        <button class="hud-btn {showSettingsPopover ? 'active' : ''}" on:click={() => (showSettingsPopover = !showSettingsPopover)}>
          <span style="display:inline-block; width:8px; height:8px; background:{strokeColor}; border:1px solid var(--text-primary);"></span>
        </button>
      </div>

      {#if showSettingsPopover}
        <div class="settings-popover">
          <div style="display: flex; gap: 6px; align-items: center;">
            {#each colorPalette as color}
              <button
                class="color-swatch {strokeColor === color ? 'active' : ''}"
                style="background-color: {color};"
                on:click={() => {
                  strokeColor = color;
                  showSettingsPopover = false;
                }}
                title="Renk: {color}"
              ></button>
            {/each}
          </div>
          <div style="display: flex; align-items: center; gap: 6px; border-left: 1px solid var(--border-raw); padding-left: 10px;">
            <span style="font-size: 10px; color: var(--text-muted);">({strokeSize}px):</span>
            <input
              type="range"
              min="1"
              max="16"
              bind:value={strokeSize}
              class="stroke-slider"
            />
          </div>
        </div>
      {/if}

      <div class="hud-group">
        <button class="hud-btn {isSearchBarOpen ? 'active' : ''}" on:click={() => (isSearchBarOpen = !isSearchBarOpen)}>[FIND]</button>
        <button class="hud-btn {isBookMode ? 'active' : ''}" on:click={toggleBookMode}>[BOOK]</button>
        <button class="hud-btn {isSmartDarkMode ? 'active' : ''}" on:click={() => (isSmartDarkMode = !isSmartDarkMode)}>[DARK]</button>
        <button class="hud-btn" on:click={handleOpenBionicReading}>[BIONIC]</button>
        <button class="hud-btn {isReadingRulerActive ? 'active' : ''}" on:click={() => (isReadingRulerActive = !isReadingRulerActive)}>[RULER]</button>
        <button class="hud-btn" on:click={handleRunOCR}>[OCR]</button>
        <button class="hud-btn {isSpeaking ? 'active' : ''}" on:click={handleSpeakText}>{isSpeaking ? "[STOP]" : "[TTS]"}</button>
        <button class="hud-btn {isAiModalOpen ? 'active' : ''}" on:click={() => (isAiModalOpen = true)}>[AI]</button>
      </div>

      {#if isPdf}
        <button class="hud-btn active" on:click={handleBurnInExport}>[BURN & SAVE]</button>
      {:else}
        <button class="hud-btn active" on:click={handleExportImage}>[CONVERT]</button>
      {/if}
    </div>
  {/if}

  <!-- Drag-and-Drop Fullscreen Overlay -->
  {#if isDraggingFileOver}
    <div class="drag-overlay">
      <div class="drag-box">
        <h3 style="font-size: 16px; font-weight: 700; color: var(--accent-industrial); margin-bottom: 6px; text-transform: uppercase;">// SÜRÜKLENEN_BELGEYİ_BIRAKIN</h3>
        <p style="font-size: 12px; color: var(--text-muted); margin: 0;">Mevcut belge kapatılacak ve yeni dosya yüklenecektir</p>
      </div>
    </div>
  {/if}

  <!-- Toast Notification -->
  {#if toastMsg}
    <div class="toast-notification">
      <span>// {toastMsg.toUpperCase()}</span>
    </div>
  {/if}
</main>

<style>
  .drag-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: var(--bg-primary);
    z-index: 9999;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
    animation: fadeIn 0.15s ease-out;
  }

  .drag-box {
    border: 2px dashed var(--accent-industrial);
    background: var(--bg-surface);
    padding: 40px 50px;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }
</style>

<script>
  import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
  import { getPageAnnotations, addAnnotation, deleteAnnotation } from "../pdfState.js";
  import { requireAsyncPromptCoordinator } from "../services/asyncPrompt.svelte.js";
  import { isSignatureLossDeclined } from "../services/signatureLossGuard.js";
  import {
    clientPointToLayerNormalized,
    fixedSizeNormalizedRect,
    normalizedRectBetween,
  } from "../utils/pdfGeometry.js";

  // Koordinat sözleşmesi: bu katmanın yerel kutusu sayfanın görüntü uzayıdır (page
  // /Rotate uygulanmış, viewer rotation kaldırılmış). Çizim, önizleme ve backend'e
  // giden/gelen dikdörtgenler bu uzayda sol-üst kökenli normalize [0..1] değerlerdir;
  // PDF user space'e dönüşüm job worker'da PDFium ile yapılır.
  let {
    sessionId = "",
    contentGeneration = 0,
    pageIndex = 0,
    // Üst kaba uygulanan CSS rotate() açısı (viewer rotation).
    rotation = 0,
    // Görüntü uzayında sayfa boyutu (pt); yalnız sabit boyutlu not için.
    pageWidth = 0,
    pageHeight = 0,
    activeTool = "pointer",
    activeColor = "#facc15",
    onsessionSnapshot,
  } = $props();

  const NOTE_SIZE_PT = 24;
  // Yalnız bu araçlar çizer; diğerleri (seçim, form tasarım araçları) olayları geçirir.
  const DRAWING_TOOLS = new Set(["highlight", "underline", "text"]);
  const drawing = $derived(DRAWING_TOOLS.has(activeTool));
  const MIN_DRAG_PX = 5;

  const asyncPrompt = requireAsyncPromptCoordinator();

  let annotations = $state([]);
  let isDrawing = $state(false);
  let startPoint = $state(null);
  let currentRect = $state(null);

  let layerRef = $state();

  $effect(() => {
    const request = { sessionId, contentGeneration, pageIndex };
    if (sessionId && pageIndex >= 0) void loadAnnotations(request);
  });

  async function loadAnnotations(request = { sessionId, contentGeneration, pageIndex }) {
    const result = await getPageAnnotations(
      request.sessionId,
      request.contentGeneration,
      request.pageIndex,
    );
    if (
      request.sessionId === sessionId &&
      request.contentGeneration === contentGeneration &&
      request.pageIndex === pageIndex
    ) {
      annotations = result;
    }
  }

  function percentStyle(rect) {
    if (!rect) return "left: 0%; top: 0%; width: 0%; height: 0%;";
    return `left: ${rect.x * 100}%; top: ${rect.y * 100}%; width: ${rect.width * 100}%; height: ${rect.height * 100}%;`;
  }

  function localPoint(event) {
    return clientPointToLayerNormalized(event.clientX, event.clientY, {
      boundingRect: layerRef.getBoundingClientRect(),
      localWidth: layerRef.offsetWidth,
      localHeight: layerRef.offsetHeight,
      rotation,
    });
  }

  function handlePointerDown(e) {
    if (!drawing) return;
    if (e.button !== 0) return; // Sadece sol tık

    const point = localPoint(e);
    if (!point) return;
    startPoint = point;
    isDrawing = true;
    currentRect = normalizedRectBetween(point, point);
  }

  function handlePointerMove(e) {
    if (!isDrawing || !drawing) return;

    const point = localPoint(e);
    if (point) currentRect = normalizedRectBetween(startPoint, point);
  }

  async function handlePointerUp() {
    if (!isDrawing) return;
    isDrawing = false;

    const isDrag =
      currentRect &&
      currentRect.width * layerRef.offsetWidth > MIN_DRAG_PX &&
      currentRect.height * layerRef.offsetHeight > MIN_DRAG_PX;
    if (isDrag) {
      const normalizedRect = { ...currentRect };
      const annotType = activeTool === "highlight" ? "Highlight" 
                      : activeTool === "underline" ? "Underline" 
                      : activeTool === "text" ? "Text" 
                      : "Ink";
                      
      let contents = null;
      if (annotType === "Text") {
        contents = await asyncPrompt.request({
          title: "Not Açıklaması",
          label: "Notunuz",
        });
        if (!contents) {
          currentRect = null;
          return;
        }
      }

      try {
        const sessionSnapshot = await addAnnotation(
          sessionId,
          contentGeneration,
          pageIndex,
          annotType,
          normalizedRect,
          activeColor,
          contents,
        );
        contentGeneration = sessionSnapshot.contentGeneration;
        await loadAnnotations();
        onsessionSnapshot?.(sessionSnapshot);
      } catch (err) {
        if (!isSignatureLossDeclined(err)) console.error("Açıklama kaydedilemedi", err);
      }
    } else if (activeTool === "text") {
       // Küçük tıklama ise sabit boyutlu not ekle
       const normalizedRect = fixedSizeNormalizedRect(
         startPoint,
         NOTE_SIZE_PT,
         pageWidth,
         pageHeight,
       );
       const contents = normalizedRect
         ? await asyncPrompt.request({
             title: "Not Açıklaması",
             label: "Notunuz",
           })
         : null;
       if (contents) {
         try {
           const sessionSnapshot = await addAnnotation(
             sessionId,
             contentGeneration,
             pageIndex,
             "Text",
             normalizedRect,
             activeColor,
             contents,
           );
           contentGeneration = sessionSnapshot.contentGeneration;
           await loadAnnotations();
           onsessionSnapshot?.(sessionSnapshot);
         } catch (err) {
           if (!isSignatureLossDeclined(err)) console.error("Açıklama kaydedilemedi", err);
         }
       }
    }
    
    currentRect = null;
  }
  
  async function removeAnnotation(id) {
    const confirmed = await confirmDialog(
      "Bu açıklamayı silmek istediğinize emin misiniz?",
      { title: "Açıklamayı Sil", kind: "warning" },
    );
    if (confirmed) {
      try {
        const sessionSnapshot = await deleteAnnotation(
          sessionId,
          contentGeneration,
          pageIndex,
          id,
        );
        contentGeneration = sessionSnapshot.contentGeneration;
        await loadAnnotations();
        onsessionSnapshot?.(sessionSnapshot);
      } catch (e) {
        if (!isSignatureLossDeclined(e)) console.error(e);
      }
    }
  }

</script>

<div 
  bind:this={layerRef}
  role="application"
  aria-label="PDF açıklama katmanı"
  class="annotation-layer {drawing ? 'drawing' : ''}"
  onpointerdown={handlePointerDown}
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
>
  {#each annotations as ann (ann.id)}
    <button
      type="button"
      class="annot-item {ann.annot_type.toLowerCase()}"
      style="{percentStyle(ann.normalized_rect)} --annot-color: {ann.color}"
      title={ann.contents || ann.annot_type}
      onclick={(event) => {
        event.stopPropagation();
        void removeAnnotation(ann.id);
      }}
    >
      {#if ann.annot_type === 'Text'}
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
           <path stroke-linecap="round" stroke-linejoin="round" d="M8 10h.01M12 10h.01M16 10h.01M9 16H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-5l-5 5v-5z"/>
        </svg>
      {/if}
    </button>
  {/each}

  {#if currentRect}
    <div 
      class="annot-preview {activeTool}"
      style="{percentStyle(currentRect)} --annot-color: {activeColor}"
    ></div>
  {/if}
</div>

<style>
  .annotation-layer {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    z-index: 20;
    pointer-events: none; /* Pointer modunda alt katman tıklamalarına izin ver */
  }
  .annotation-layer.drawing {
    pointer-events: auto; /* Çizim modunda eventleri yakala */
    cursor: crosshair;
  }

  .annot-item {
    position: absolute;
    padding: 0;
    border: 0;
    background: transparent;
    pointer-events: auto; /* Annotation'lara tıklanabilsin (silmek için) */
    cursor: pointer;
    transition: opacity 0.2s;
  }
  .annot-item:hover {
    opacity: 0.7;
    outline: 2px dashed #ef4444;
  }

  .annot-item.highlight {
    background-color: var(--annot-color);
    opacity: 0.4;
    mix-blend-mode: multiply;
  }

  .annot-item.underline {
    border-bottom: 2px solid var(--annot-color);
  }

  .annot-item.text {
    background-color: var(--annot-color);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 2px 5px rgba(0,0,0,0.3);
    opacity: 0.9;
  }
  .annot-item.text svg {
    width: 60%;
    height: 60%;
    color: #000;
  }

  .annot-preview {
    position: absolute;
    background-color: var(--annot-color);
    opacity: 0.3;
    pointer-events: none;
    border: 1px dashed var(--annot-color);
  }
  
  .annot-preview.underline {
    background: transparent;
    border: none;
    border-bottom: 2px solid var(--annot-color);
    opacity: 1.0;
  }
</style>

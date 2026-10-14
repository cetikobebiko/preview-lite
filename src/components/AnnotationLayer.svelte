<script>
  import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
  import { getPageAnnotations, addAnnotation, deleteAnnotation } from "../pdfState.js";
  import { requireAsyncPromptCoordinator } from "../services/asyncPrompt.svelte.js";

  let {
    sessionId = "",
    contentGeneration = 0,
    pageIndex = 0,
    scale = 1.0,
    pageWidth = 0,
    pageHeight = 0,
    activeTool = "pointer",
    activeColor = "#facc15",
    onsessionSnapshot,
  } = $props();
  
  const asyncPrompt = requireAsyncPromptCoordinator();

  let annotations = $state([]);
  let isDrawing = $state(false);
  let startX = $state(0);
  let startY = $state(0);
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

  // PDF koordinatlarını DOM koordinatlarına dönüştür (Yüzde bazlı)
  function pdfToDomRect(pdfRect) {
    if (!pdfRect || !pageWidth || !pageHeight) return { left: 0, top: 0, width: 0, height: 0 };
    return {
      left: (pdfRect.x / pageWidth) * 100,
      top: ((pageHeight - pdfRect.y - pdfRect.height) / pageHeight) * 100,
      width: (pdfRect.width / pageWidth) * 100,
      height: (pdfRect.height / pageHeight) * 100
    };
  }

  // DOM koordinatlarını PDF koordinatlarına dönüştür (Pixel bazlı girişten)
  function domToPdfRect(domRect) {
    let x = domRect.left / scale;
    let width = domRect.width / scale;
    let height = domRect.height / scale;
    let y = pageHeight - (domRect.top / scale) - height; // bottom
    return { x, y, width, height };
  }

  function handlePointerDown(e) {
    if (activeTool === "pointer") return;
    if (e.button !== 0) return; // Sadece sol tık

    const rect = layerRef.getBoundingClientRect();
    startX = e.clientX - rect.left;
    startY = e.clientY - rect.top;
    isDrawing = true;

    currentRect = { left: startX, top: startY, width: 0, height: 0 };
  }

  function handlePointerMove(e) {
    if (!isDrawing || activeTool === "pointer") return;
    
    const rect = layerRef.getBoundingClientRect();
    const currentX = e.clientX - rect.left;
    const currentY = e.clientY - rect.top;
    
    const left = Math.min(startX, currentX);
    const top = Math.min(startY, currentY);
    const width = Math.abs(currentX - startX);
    const height = Math.abs(currentY - startY);
    
    currentRect = { left, top, width, height };
  }

  async function handlePointerUp(e) {
    if (!isDrawing) return;
    isDrawing = false;
    
    if (currentRect && currentRect.width > 5 && currentRect.height > 5) {
      const pdfRect = domToPdfRect(currentRect);
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
          pdfRect,
          activeColor,
          contents,
        );
        contentGeneration = sessionSnapshot.contentGeneration;
        await loadAnnotations();
        onsessionSnapshot?.(sessionSnapshot);
      } catch (err) {
        console.error("Açıklama kaydedilemedi", err);
      }
    } else if (activeTool === "text") {
       // Küçük tıklama ise sabit boyutlu not ekle
       const pdfRect = domToPdfRect({ left: startX, top: startY, width: 24, height: 24 });
       const contents = await asyncPrompt.request({
         title: "Not Açıklaması",
         label: "Notunuz",
       });
       if (contents) {
         const sessionSnapshot = await addAnnotation(
           sessionId,
           contentGeneration,
           pageIndex,
           "Text",
           pdfRect,
           activeColor,
           contents,
         );
         contentGeneration = sessionSnapshot.contentGeneration;
         await loadAnnotations();
         onsessionSnapshot?.(sessionSnapshot);
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
        console.error(e);
      }
    }
  }

</script>

<div 
  bind:this={layerRef}
  role="application"
  aria-label="PDF açıklama katmanı"
  class="annotation-layer {activeTool !== 'pointer' ? 'drawing' : ''}"
  onpointerdown={handlePointerDown}
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
>
  {#each annotations as ann (ann.id)}
    {@const domRect = pdfToDomRect(ann.rect)}
    <button
      type="button"
      class="annot-item {ann.annot_type.toLowerCase()}"
      style="left: {domRect.left}%; top: {domRect.top}%; width: {domRect.width}%; height: {domRect.height}%; --annot-color: {ann.color}"
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
      style="left: {currentRect.left}px; top: {currentRect.top}px; width: {currentRect.width}px; height: {currentRect.height}px; --annot-color: {activeColor}"
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

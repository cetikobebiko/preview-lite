<script>
  import { getContext, untrack } from "svelte";
  import { getPageFormFields } from "../pdfState.js";
  import {
    createFormDesignState,
    FORM_DESIGN_CONTEXT,
  } from "../stores/formDesignState.svelte.js";
  import { clientPointToLayerNormalized, normalizedRectBetween } from "../utils/pdfGeometry.js";

  // Form tasarım katmanı (docs/engineering/form-authoring-plan.md, Faz B). Koordinat
  // sözleşmesi AnnotationLayer ile aynıdır: katmanın yerel kutusu sayfanın görüntü uzayıdır,
  // dikdörtgenler sol-üst kökenli normalize [0..1] değerlerdir. Her bitmiş hareket (çizim,
  // taşıma, boyutlandırma, silme) tek bir mutasyon olarak gönderilir.
  let {
    sessionId = "",
    contentGeneration = 0,
    pageIndex = 0,
    // Üst kaba uygulanan CSS rotate() açısı (viewer rotation).
    rotation = 0,
    // Görüntü uzayında sayfa boyutu (pt); tıklamayla eklenen alanların varsayılan boyutu için.
    pageWidth = 0,
    pageHeight = 0,
    activeTool = "form-select",
    onsessionSnapshot,
  } = $props();

  // Tıklamayla (sürüklemeden) eklenen alanların boyutu, pt.
  const DEFAULT_SIZES = Object.freeze({
    "form-text": [180, 22],
    "form-checkbox": [14, 14],
    "form-radio": [14, 14],
    "form-combo": [150, 22],
    "form-list": [150, 60],
  });
  const MIN_DRAG_PX = 6;
  const DEFAULT_OPTIONS = Object.freeze(["Seçenek 1", "Seçenek 2", "Seçenek 3"]);

  const design = getContext(FORM_DESIGN_CONTEXT) ?? createFormDesignState();

  let widgets = $state([]);
  let layerRef = $state();
  // Sürüklenen çizim/taşıma/boyutlandırma: { mode, start, origin, rect, widget? }
  let gesture = $state(null);

  $effect(() => {
    const request = { sessionId, contentGeneration, pageIndex };
    widgets = [];
    if (!request.sessionId || request.pageIndex < 0) return;
    let current = true;
    getPageFormFields(request.sessionId, request.contentGeneration, request.pageIndex)
      .then((result) => {
        if (!current) return;
        widgets = Array.isArray(result) ? result.filter((widget) => widget.normalized_rect) : [];
        untrack(() => reselect(request));
      })
      .catch((error) => {
        if (current) console.error("Form alanları yüklenemedi", error);
      });
    return () => {
      current = false;
    };
  });

  function reselect(request) {
    const pending = design.pendingSelect;
    const selection = design.selection;
    if (pending?.sessionId === request.sessionId && pending.pageIndex === request.pageIndex) {
      const widget = pending.newest
        ? widgets.reduce((last, item) => (!last || item.annot_index > last.annot_index ? item : last), null)
        : widgets.find((item) => item.field_name === pending.fieldName);
      design.clearPendingSelect();
      if (widget) selectWidget(widget);
      return;
    }
    // Seçim bu sayfanın eski bir generation'ına aitse geçersizdir.
    if (
      selection?.sessionId === request.sessionId &&
      selection.pageIndex === request.pageIndex &&
      selection.contentGeneration !== request.contentGeneration
    ) {
      design.clearSelection();
    }
  }

  function context() {
    return { sessionId, contentGeneration, onsessionSnapshot };
  }

  function selectWidget(widget) {
    design.select({ sessionId, contentGeneration, pageIndex, widget, onsessionSnapshot });
  }

  function isSelected(widget) {
    const selection = design.selection;
    return (
      selection?.sessionId === sessionId &&
      selection.pageIndex === pageIndex &&
      selection.widget.annot_index === widget.annot_index
    );
  }

  function percentStyle(rect) {
    return `left: ${rect.x * 100}%; top: ${rect.y * 100}%; width: ${rect.width * 100}%; height: ${rect.height * 100}%;`;
  }

  function localPoint(event) {
    if (!layerRef) return null;
    return clientPointToLayerNormalized(event.clientX, event.clientY, {
      boundingRect: layerRef.getBoundingClientRect(),
      localWidth: layerRef.offsetWidth,
      localHeight: layerRef.offsetHeight,
      rotation,
    });
  }

  function clamp(value, min, max) {
    return Math.min(Math.max(value, min), max);
  }

  // Dikdörtgeni sayfanın içinde tutar.
  function insidePage(rect) {
    const width = clamp(rect.width, 0.001, 1);
    const height = clamp(rect.height, 0.001, 1);
    return {
      x: clamp(rect.x, 0, 1 - width),
      y: clamp(rect.y, 0, 1 - height),
      width,
      height,
    };
  }

  function sizedRect(point, [widthPt, heightPt]) {
    if (!(pageWidth > 0) || !(pageHeight > 0)) return null;
    return insidePage({
      x: point.x,
      y: point.y,
      width: widthPt / pageWidth,
      height: heightPt / pageHeight,
    });
  }

  function largeEnough(rect) {
    return (
      layerRef &&
      rect.width * layerRef.offsetWidth >= MIN_DRAG_PX &&
      rect.height * layerRef.offsetHeight >= MIN_DRAG_PX
    );
  }

  function radioDefaults() {
    const selected = design.selection?.widget;
    const names = new Set(widgets.map((widget) => widget.field_name));
    let group = selected?.kind === "RadioButton" ? selected.field_name : null;
    if (!group) {
      let n = 1;
      while (names.has(`Group${n}`)) n += 1;
      group = `Group${n}`;
    }
    const members = widgets.filter((widget) => widget.kind === "RadioButton" && widget.field_name === group).length;
    return { group, export: `Choice${members + 1}` };
  }

  function newField(tool) {
    switch (tool) {
      case "form-text":
        return { kind: "text", multiline: false };
      case "form-checkbox":
        return { kind: "checkbox" };
      case "form-radio":
        return { kind: "radio", ...radioDefaults() };
      case "form-combo":
        return { kind: "combo", options: [...DEFAULT_OPTIONS], editable: false };
      case "form-list":
        return { kind: "list", options: [...DEFAULT_OPTIONS], multi: false };
      default:
        return null;
    }
  }

  function handlePointerDown(event) {
    if (event.button !== 0 || design.busy) return;
    const point = localPoint(event);
    if (!point) return;
    const box = event.target.closest?.("[data-annot-index]");
    if (activeTool === "form-select") {
      if (!box) {
        design.clearSelection();
        return;
      }
      const widget = widgets.find((item) => String(item.annot_index) === box.dataset.annotIndex);
      if (!widget) return;
      selectWidget(widget);
      gesture = {
        mode: event.target.dataset.handle === "resize" ? "resize" : "move",
        start: point,
        origin: { ...widget.normalized_rect },
        rect: { ...widget.normalized_rect },
        widget,
      };
    } else if (newField(activeTool)) {
      gesture = { mode: "draw", start: point, rect: normalizedRectBetween(point, point) };
    } else {
      return;
    }
    layerRef.setPointerCapture?.(event.pointerId);
    event.preventDefault();
  }

  function handlePointerMove(event) {
    if (!gesture) return;
    const point = localPoint(event);
    if (!point) return;
    const dx = point.x - gesture.start.x;
    const dy = point.y - gesture.start.y;
    if (gesture.mode === "draw") {
      gesture.rect = normalizedRectBetween(gesture.start, point);
    } else if (gesture.mode === "move") {
      gesture.rect = insidePage({ ...gesture.origin, x: gesture.origin.x + dx, y: gesture.origin.y + dy });
    } else {
      gesture.rect = insidePage({
        ...gesture.origin,
        width: Math.max(gesture.origin.width + dx, 0.005),
        height: Math.max(gesture.origin.height + dy, 0.005),
      });
    }
  }

  function handlePointerUp(event) {
    const finished = gesture;
    gesture = null;
    layerRef?.releasePointerCapture?.(event.pointerId);
    if (!finished) return;
    if (finished.mode === "draw") {
      const field = newField(activeTool);
      const rect = largeEnough(finished.rect)
        ? insidePage(finished.rect)
        : sizedRect(finished.start, DEFAULT_SIZES[activeTool]);
      if (!field || !rect) return;
      void design.add(context(), { op: "add", pageIndex, normalizedRect: rect, field, name: null });
      return;
    }
    const moved = ["x", "y", "width", "height"].some(
      (key) => Math.abs(finished.rect[key] - finished.origin[key]) > 1e-4,
    );
    if (moved) void design.moveSelected(finished.rect);
  }

  function handleKeyDown(event) {
    if (event.key !== "Delete" && event.key !== "Backspace") return;
    if (activeTool !== "form-select" || !design.selection || design.busy) return;
    const editing = event.target?.closest?.("input, textarea, select, [contenteditable='true']");
    if (editing) return;
    if (design.selection.sessionId !== sessionId || design.selection.pageIndex !== pageIndex) return;
    event.preventDefault();
    void design.deleteSelected();
  }

  function displayRect(widget) {
    return gesture?.widget?.annot_index === widget.annot_index ? gesture.rect : widget.normalized_rect;
  }

  const KIND_LABELS = Object.freeze({
    Text: "Metin",
    Checkbox: "Onay kutusu",
    RadioButton: "Radio",
    ComboBox: "Açılır liste",
    ListBox: "Liste",
    PushButton: "Düğme",
    Signature: "İmza",
    Unknown: "Alan",
  });
</script>

<svelte:window onkeydown={handleKeyDown} />

<div
  bind:this={layerRef}
  class="form-design-layer"
  class:creating={activeTool !== "form-select"}
  class:busy={design.busy}
  role="application"
  aria-label="Form tasarım katmanı"
  onpointerdown={handlePointerDown}
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
  onpointercancel={() => (gesture = null)}
>
  {#each widgets as widget (widget.annot_index)}
    {@const selected = isSelected(widget)}
    <div
      class="design-field"
      class:selected
      class:hidden-field={widget.hidden}
      style={percentStyle(displayRect(widget))}
      data-annot-index={widget.annot_index}
      title="{KIND_LABELS[widget.kind] ?? 'Alan'}: {widget.field_name}"
    >
      <span class="design-label">{widget.field_name}</span>
      {#if selected && activeTool === "form-select"}
        <span class="resize-handle" data-handle="resize" aria-label="Boyutlandır"></span>
      {/if}
    </div>
  {/each}

  {#if gesture?.mode === "draw"}
    <div class="draw-preview" style={percentStyle(gesture.rect)}></div>
  {/if}
</div>

<style>
  .form-design-layer {
    position: absolute;
    inset: 0;
    z-index: 25;
    pointer-events: auto;
    cursor: default;
    touch-action: none;
    user-select: none;
  }

  .form-design-layer.creating {
    cursor: crosshair;
  }

  .form-design-layer.busy {
    cursor: progress;
  }

  .design-field {
    position: absolute;
    box-sizing: border-box;
    border: 1px solid #2563eb;
    background: rgba(37, 99, 235, 0.12);
    cursor: move;
  }

  .form-design-layer.creating .design-field {
    cursor: crosshair;
  }

  .design-field.hidden-field {
    border-style: dashed;
    opacity: 0.6;
  }

  .design-field.selected {
    border: 2px solid #f59e0b;
    background: rgba(245, 158, 11, 0.15);
  }

  .design-label {
    position: absolute;
    left: 0;
    bottom: 100%;
    max-width: 100%;
    overflow: hidden;
    padding: 0 3px;
    border-radius: 2px 2px 0 0;
    background: #2563eb;
    color: #ffffff;
    font: 10px/14px system-ui, sans-serif;
    white-space: nowrap;
    text-overflow: ellipsis;
    pointer-events: none;
  }

  .design-field.selected .design-label {
    background: #f59e0b;
  }

  .resize-handle {
    position: absolute;
    right: -5px;
    bottom: -5px;
    width: 10px;
    height: 10px;
    border: 1px solid #ffffff;
    border-radius: 2px;
    background: #f59e0b;
    cursor: nwse-resize;
  }

  .draw-preview {
    position: absolute;
    box-sizing: border-box;
    border: 1px dashed #2563eb;
    background: rgba(37, 99, 235, 0.1);
    pointer-events: none;
  }
</style>

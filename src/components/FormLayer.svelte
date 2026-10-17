<script>
  import { getContext, untrack } from "svelte";
  import { getPageFormFields } from "../pdfState.js";
  import {
    createFormDraftState,
    FORM_DRAFTS_CONTEXT,
  } from "../stores/formDraftState.svelte.js";

  // Koordinat sözleşmesi AnnotationLayer ile aynıdır: katmanın yerel kutusu sayfanın
  // görüntü uzayıdır ve widget dikdörtgenleri sol-üst kökenli normalize [0..1]
  // değerlerdir. Faz 2: kullanıcının girdiği değerler yalnız taslaktır, belgeye
  // yazılmaz (docs/engineering/form-filling-plan.md).
  //
  // Değişmemiş bir alanın görünümünü PDFium bitmap'i zaten çizer; kontrol o yüzden
  // şeffaftır ve yalnız odakta ya da taslak varken opak çizilir. Böylece metin iki kez
  // görünmez.
  let { sessionId = "", contentGeneration = 0, pageIndex = 0 } = $props();

  const EDITABLE_KINDS = new Set(["Text", "Checkbox", "RadioButton", "ComboBox", "ListBox"]);
  // Tek satırlık alanda font, alan yüksekliğinin bu oranı; üst sınır sayfa yüksekliğine göre.
  const SINGLE_LINE_FONT_RATIO = 0.62;
  const MAX_FONT_CQH = 2.2;
  const MULTILINE_FONT_CQH = 1.5;

  const drafts = getContext(FORM_DRAFTS_CONTEXT) ?? createFormDraftState();

  let widgets = $state([]);

  $effect(() => {
    const request = { sessionId, contentGeneration, pageIndex };
    widgets = [];
    if (!request.sessionId || request.pageIndex < 0) return;
    untrack(() => drafts.bindSession(request.sessionId));
    let current = true;
    getPageFormFields(request.sessionId, request.contentGeneration, request.pageIndex)
      .then((result) => {
        if (current) widgets = Array.isArray(result) ? result : [];
      })
      .catch((error) => {
        if (current) console.error("Form alanları yüklenemedi", error);
      });
    return () => {
      current = false;
    };
  });

  const fields = $derived(
    widgets.filter(
      (widget) => !widget.hidden && widget.normalized_rect && EDITABLE_KINDS.has(widget.kind),
    ),
  );

  function widgetKey(widget) {
    return `${pageIndex}:${widget.annot_index}`;
  }

  function fieldStyle(widget) {
    const rect = widget.normalized_rect;
    const fontCqh = widget.multiline || widget.kind === "ListBox"
      ? MULTILINE_FONT_CQH
      : Math.min(rect.height * 100 * SINGLE_LINE_FONT_RATIO, MAX_FONT_CQH);
    return `left: ${rect.x * 100}%; top: ${rect.y * 100}%; width: ${rect.width * 100}%; height: ${rect.height * 100}%; font-size: ${fontCqh}cqh;`;
  }

  function originalSelection(widget) {
    return widget.options.flatMap((option, index) => (option.selected ? [index] : []));
  }

  function textValue(widget) {
    const draft = drafts.get(widget.field_name);
    return draft?.kind === "text" ? draft.value : (widget.value ?? "");
  }

  function isChecked(widget) {
    const draft = drafts.get(widget.field_name);
    if (widget.kind === "RadioButton") {
      return draft?.kind === "radio" ? draft.widget === widgetKey(widget) : Boolean(widget.checked);
    }
    return draft?.kind === "checkbox" ? draft.checked : Boolean(widget.checked);
  }

  function selection(widget) {
    const draft = drafts.get(widget.field_name);
    return draft?.kind === "choice" ? draft.selected : originalSelection(widget);
  }

  function sameSelection(left, right) {
    return left.length === right.length && left.every((value, index) => value === right[index]);
  }

  function setText(widget, value) {
    if (value === (widget.value ?? "")) drafts.discard(widget.field_name);
    else drafts.set(widget.field_name, { kind: "text", value });
  }

  function setCheckbox(widget, checked) {
    if (checked === Boolean(widget.checked)) drafts.discard(widget.field_name);
    else drafts.set(widget.field_name, { kind: "checkbox", checked });
  }

  function selectRadio(widget) {
    // Grubun belgede seçili düğmesine geri dönmek taslağı kaldırır.
    if (widget.checked) drafts.discard(widget.field_name);
    else drafts.set(widget.field_name, { kind: "radio", widget: widgetKey(widget) });
  }

  function setSelection(widget, selected) {
    if (sameSelection(selected, originalSelection(widget))) drafts.discard(widget.field_name);
    else drafts.set(widget.field_name, { kind: "choice", selected });
  }

  function selectedIndices(select) {
    return Array.from(select.selectedOptions, (option) => Number(option.value)).filter(
      (value) => Number.isInteger(value) && value >= 0,
    );
  }
</script>

<div class="form-layer" role="group" aria-label="PDF form alanları">
  {#each fields as widget (widget.annot_index)}
    {@const dirty = drafts.has(widget.field_name)}
    {@const label = widget.field_name || "Form alanı"}
    <div
      class="form-field kind-{widget.kind.toLowerCase()}"
      class:dirty
      class:read-only={widget.read_only}
      style={fieldStyle(widget)}
      data-annot-index={widget.annot_index}
    >
      {#if widget.kind === "Text" && widget.multiline}
        <textarea
          aria-label={label}
          aria-required={widget.required}
          disabled={widget.read_only}
          value={textValue(widget)}
          oninput={(event) => setText(widget, event.currentTarget.value)}
        ></textarea>
      {:else if widget.kind === "Text"}
        <input
          type={widget.password ? "password" : "text"}
          aria-label={label}
          aria-required={widget.required}
          disabled={widget.read_only}
          value={textValue(widget)}
          oninput={(event) => setText(widget, event.currentTarget.value)}
        />
      {:else if widget.kind === "Checkbox"}
        <input
          type="checkbox"
          aria-label={label}
          aria-required={widget.required}
          disabled={widget.read_only}
          checked={isChecked(widget)}
          onchange={(event) => setCheckbox(widget, event.currentTarget.checked)}
        />
      {:else if widget.kind === "RadioButton"}
        <input
          type="radio"
          name={`form:${pageIndex}:${widget.field_name}`}
          aria-label={label}
          aria-required={widget.required}
          disabled={widget.read_only}
          checked={isChecked(widget)}
          onchange={() => selectRadio(widget)}
        />
      {:else}
        {@const selected = selection(widget)}
        {@const multiple = widget.kind === "ListBox" || widget.multi_select}
        <select
          aria-label={label}
          aria-required={widget.required}
          disabled={widget.read_only}
          multiple={multiple}
          size={widget.kind === "ListBox" ? Math.max(2, widget.options.length) : undefined}
          onchange={(event) => setSelection(widget, selectedIndices(event.currentTarget))}
        >
          {#if !multiple && selected.length === 0}
            <option value="-1" selected disabled></option>
          {/if}
          {#each widget.options as option, index (index)}
            <option value={index} selected={selected.includes(index)}>{option.label}</option>
          {/each}
        </select>
      {/if}
    </div>
  {/each}
</div>

<style>
  .form-layer {
    position: absolute;
    inset: 0;
    container-type: size;
    pointer-events: none;
  }

  .form-field {
    position: absolute;
    box-sizing: border-box;
    pointer-events: auto;
    background: rgba(59, 130, 246, 0.08);
    outline: 1px solid rgba(59, 130, 246, 0.25);
  }

  .form-field:hover {
    outline-color: rgba(59, 130, 246, 0.7);
  }

  .form-field.read-only {
    background: transparent;
    outline: none;
  }

  .form-field input,
  .form-field textarea,
  .form-field select {
    box-sizing: border-box;
    width: 100%;
    height: 100%;
    margin: 0;
    padding: 0 0.15em;
    border: 0;
    border-radius: 0;
    background: transparent;
    font: inherit;
    color: transparent;
    caret-color: #111827;
    appearance: none;
    outline: none;
  }

  /* Liste kutusunda WebKit seçenek metnini ve seçim vurgusunu `color` ile gizlemiyor;
     değişmemiş liste kutusu tamamen saydamdır (etkileşim sürer), görünümü bitmap çizer. */
  .form-field.kind-listbox select {
    opacity: 0;
    padding: 0;
  }

  .form-field textarea {
    resize: none;
    padding-top: 0.1em;
    line-height: 1.15;
  }

  .form-field input:focus,
  .form-field textarea:focus,
  .form-field select:focus,
  .form-field.dirty input,
  .form-field.dirty textarea,
  .form-field.dirty select {
    opacity: 1;
    background: #ffffff;
    color: #111827;
  }

  .form-field:focus-within {
    outline: 2px solid #2563eb;
  }

  /* Taslakta widget dikdörtgeninin tamamı örtülür: bitmap'teki eski görünüm
     (ör. yuvarlak radio'nun köşelerinde kalan işaret) sızmaz. */
  .form-field.dirty {
    background: #ffffff;
    outline: 2px solid #f59e0b;
  }

  .form-field input:disabled,
  .form-field textarea:disabled,
  .form-field select:disabled {
    cursor: not-allowed;
  }

  /* Checkbox ve radio: değişmemişken bitmap çizer; taslakta kendi işaretimizi çizeriz. */
  .kind-checkbox input,
  .kind-radiobutton input {
    cursor: pointer;
  }

  .form-field.dirty.kind-radiobutton input {
    border-radius: 50%;
  }

  .form-field.dirty.kind-checkbox input:checked,
  .form-field.dirty.kind-radiobutton input:checked {
    background:
      radial-gradient(circle, #111827 0 32%, transparent 34%),
      #ffffff;
  }

  .form-field.dirty.kind-checkbox input:checked {
    background:
      linear-gradient(45deg, transparent 44%, #111827 44% 56%, transparent 56%),
      linear-gradient(-45deg, transparent 44%, #111827 44% 56%, transparent 56%),
      #ffffff;
  }

  .form-field select option {
    color: #111827;
  }
</style>

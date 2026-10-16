<script>
  import { onDestroy } from "svelte";
  import {
    generateSessionRgbDerivedCoveragePreview,
    generateSessionRgbDerivedProcessPreview,
    inspectSessionPrintResources,
  } from "../services/backend/prepress.js";
  import FeatureNotice from "./FeatureNotice.svelte";

  let {
    scale = 1.5, sessionId = "", pageIndex = -1, contentGeneration = 0, isDocumentReady = false,
    onprocessPreviewReady, oncoveragePreviewReady, onchannelsChanged,
  } = $props();
  let report = $state(null), reportError = $state(""), reportHandle = $state(null), reportRequestId = $state("");
  let processResult = $state(null), processError = $state(""), processHandle = $state(null), processRequestId = $state("");
  let coverageResult = $state(null), coverageError = $state(""), coverageHandle = $state(null), coverageRequestId = $state("");
  let thresholdPercent = $state(300);
  let channels = $state({ c: true, m: true, y: true, k: true });
  let observedContext = $state("");
  let destroyed = $state(false);
  let fallbackRequestSequence = 0;

  const canUsePage = $derived(isDocumentReady && typeof sessionId === "string" && sessionId.length > 0 && Number.isSafeInteger(contentGeneration) && contentGeneration >= 0 && Number.isInteger(pageIndex) && pageIndex >= 0);
  const currentPageBoxes = $derived(report?.pageBoxes?.find((entry) => entry.physicalPageIndex === pageIndex) ?? null);

  $effect(() => {
    const context = canUsePage ? `${sessionId}\u0000${contentGeneration}\u0000${pageIndex}` : "";
    if (context !== observedContext) { observedContext = context; resetAll(); }
  });

  function nextRequestId(prefix) {
    fallbackRequestSequence += 1;
    return globalThis.crypto?.randomUUID?.() ?? `${prefix}-${Date.now()}-${fallbackRequestSequence}`;
  }
  function snapshot(requestId) { return { sessionId, contentGeneration, physicalPageIndex: pageIndex, requestId }; }
  function isCurrent(expected, currentRequestId) {
    return !destroyed && canUsePage && expected.requestId === currentRequestId && expected.sessionId === sessionId && expected.contentGeneration === contentGeneration && expected.physicalPageIndex === pageIndex;
  }
  function cancelHandle(handle) { if (handle?.cancel) void handle.cancel().catch(() => {}); }
  function clearProcess(emit = true) {
    const clearIdentity = processResult?.identity ?? null;
    cancelHandle(processHandle); processHandle = null; processRequestId = ""; processResult = null;
    channels = { c: true, m: true, y: true, k: true };
    if (emit && clearIdentity) onprocessPreviewReady?.({ ...clearIdentity, result: null });
  }
  function clearCoverage(emit = true) {
    const clearIdentity = coverageResult?.identity ?? null;
    cancelHandle(coverageHandle); coverageHandle = null; coverageRequestId = ""; coverageResult = null;
    if (emit && clearIdentity) oncoveragePreviewReady?.({ ...clearIdentity, result: null });
  }
  function resetAll() {
    cancelHandle(reportHandle); reportHandle = null; reportRequestId = ""; report = null; reportError = "";
    processError = ""; coverageError = ""; clearProcess(); clearCoverage();
  }

  async function inspectResources() {
    if (!canUsePage) return;
    cancelHandle(reportHandle);
    const requestId = nextRequestId("inspection"), expected = snapshot(requestId);
    reportRequestId = requestId; reportError = "";
    reportHandle = inspectSessionPrintResources(sessionId, contentGeneration, pageIndex, requestId);
    const handle = reportHandle;
    try { const value = await handle.promise; if (isCurrent(expected, reportRequestId)) report = value; }
    catch (error) { if (isCurrent(expected, reportRequestId)) reportError = error instanceof Error ? error.message : String(error); }
    finally { if (isCurrent(expected, reportRequestId)) reportHandle = null; }
  }

  async function generateProcessPreview() {
    if (!canUsePage) return;
    clearCoverage(); cancelHandle(processHandle);
    const requestId = nextRequestId("process"), expected = snapshot(requestId);
    processRequestId = requestId; processError = "";
    processHandle = generateSessionRgbDerivedProcessPreview(sessionId, contentGeneration, pageIndex, requestId, scale);
    const handle = processHandle;
    try {
      const value = await handle.promise;
      if (!isCurrent(expected, processRequestId)) return;
      processResult = value; onprocessPreviewReady?.({ result: value, channels, ...expected });
    } catch (error) { if (isCurrent(expected, processRequestId)) processError = error instanceof Error ? error.message : String(error); }
    finally { if (isCurrent(expected, processRequestId)) processHandle = null; }
  }

  async function generateCoveragePreview() {
    if (!canUsePage) return;
    clearProcess(); cancelHandle(coverageHandle);
    const requestId = nextRequestId("coverage"), expected = snapshot(requestId);
    coverageRequestId = requestId; coverageError = "";
    coverageHandle = generateSessionRgbDerivedCoveragePreview(sessionId, contentGeneration, pageIndex, requestId, Number(thresholdPercent), scale);
    const handle = coverageHandle;
    try {
      const value = await handle.promise;
      if (!isCurrent(expected, coverageRequestId)) return;
      coverageResult = value; oncoveragePreviewReady?.({ result: value, ...expected });
    } catch (error) { if (isCurrent(expected, coverageRequestId)) coverageError = error instanceof Error ? error.message : String(error); }
    finally { if (isCurrent(expected, coverageRequestId)) coverageHandle = null; }
  }

  function toggleChannel(channel) {
    if (!processResult) return;
    channels = { ...channels, [channel]: !channels[channel] };
    onchannelsChanged?.({ channels, sessionId, contentGeneration, physicalPageIndex: pageIndex, requestId: processResult.identity.requestId });
  }
  const boxText = (box) => box?.coordinates ? box.coordinates.map((value) => value.toFixed(2)).join(", ") : "Yok";

  onDestroy(() => { destroyed = true; resetAll(); });
</script>

<div class="prepress-panel">
  <section>
    <h4>Print Resource Inspection</h4>
    <FeatureNotice featureId="print_resource_inspection" />
    <button onclick={inspectResources} disabled={!canUsePage || reportHandle}>{reportHandle ? "İnceleniyor..." : "PDF kaynaklarını incele"}</button>
    {#if reportError}<p class="error">{reportError}</p>{/if}
    {#if report}
      <div class="report" data-summary-status={report.summaryStatus}>
        <p><strong>Özet:</strong> {report.summaryStatus}</p>
        <p><strong>PDF:</strong> {report.documentFacts.pdfVersion} · {report.documentFacts.pageCount} sayfa</p>
        <p><strong>Conformance claim:</strong> false</p>
        <h5>Renk uzayı tanımları</h5>
        {#if report.colorSpaces.length}<ul>{#each report.colorSpaces as item}<li>{item.family} · {item.referenceCount} referans · usage_not_checked</li>{/each}</ul>{:else}<p>Uygulanan kontrollerde tanım bulunmadı.</p>{/if}
        <p><strong>ICC basic_header_only:</strong> {report.iccProfiles.length}</p>
        <p><strong>OutputIntent dictionary:</strong> {report.outputIntents.length} (tam profil/PDF/X policy doğrulanmadı)</p>
        <p><strong>Spot tanımı:</strong> {report.spotColors.length}</p>
        <p><strong>Transparency/overprint kaynağı:</strong> {report.transparency.length}</p>
        <p><strong>Font kaynağı:</strong> {report.fonts.length}</p>
        <h5>Sayfa {pageIndex + 1} kutuları (PDF point, salt okunur)</h5>
        {#if currentPageBoxes}
          <dl class="boxes"><dt>MediaBox</dt><dd>{boxText(currentPageBoxes.mediaBox)}</dd><dt>CropBox</dt><dd>{boxText(currentPageBoxes.cropBox)}</dd><dt>BleedBox</dt><dd>{boxText(currentPageBoxes.bleedBox)}</dd><dt>TrimBox</dt><dd>{boxText(currentPageBoxes.trimBox)}</dd><dt>ArtBox</dt><dd>{boxText(currentPageBoxes.artBox)}</dd><dt>Rotate</dt><dd>{currentPageBoxes.rotationDegrees}°</dd></dl>
        {:else}<p>Bu sayfa için kutu gözlemi yok.</p>{/if}
        <h5>Coverage / limitations</h5>
        <p>İncelenen: {report.coverage.inspectedAreas.join(", ")}</p>
        <p>İncelenmeyen: {report.coverage.notInspectedAreas.join(", ")}</p>
        {#if report.coverage.incompleteAreas.length}<p>Eksik: {report.coverage.incompleteAreas.join(", ")}</p>{/if}
        <ul>{#each report.coverage.limitations as limitation}<li>{limitation}</li>{/each}</ul>
      </div>
    {/if}
  </section>

  <section>
    <h4>RGB-türevli tahmini C/M/Y/K görünümü</h4>
    <FeatureNotice featureId="rgb_derived_print_preview" />
    <p class="permanent-warning">Bu görsel ekran RGB render'ından türetilir. Gerçek separasyon, spot plate, ICC transform veya RIP çıktısı değildir.</p>
    <button onclick={generateProcessPreview} disabled={!canUsePage || processHandle}>{processHandle ? "Hazırlanıyor..." : "Tahmini C/M/Y/K görünümü üret"}</button>
    {#if processError}<p class="error">{processError}</p>{/if}
    {#if processResult}
      <div class="legend">APPROXIMATE RGB-DERIVED PREVIEW</div>
      <div class="channels">{#each ["c", "m", "y", "k"] as channel}<label class:active={channels[channel]}><input type="checkbox" checked={channels[channel]} onchange={() => toggleChannel(channel)} />{channel.toUpperCase()}</label>{/each}</div>
      <p>Methodology: {processResult.metadata.methodology}</p>
      <p>colorManaged: false · outputIntentApplied: false · spotColorsPreserved: false</p>
      <button class="secondary" onclick={() => clearProcess()}>Önizlemeyi kapat</button>
    {/if}
  </section>

  <section>
    <h4>RGB-türevli tahmini kanal toplamı</h4>
    <p class="permanent-warning">Bu değer gerçek Total Area Coverage veya mürekkep tüketimi değildir.</p>
    <label class="threshold">Tahmini eşik (%) <input type="number" bind:value={thresholdPercent} min="0" max="400" step="10" /></label>
    <button onclick={generateCoveragePreview} disabled={!canUsePage || coverageHandle}>{coverageHandle ? "Hazırlanıyor..." : "Tahmini eşik ısı haritası üret"}</button>
    {#if coverageError}<p class="error">{coverageError}</p>{/if}
    {#if coverageResult}
      <div class="legend">APPROXIMATE RGB-DERIVED PREVIEW</div>
      <p>{coverageResult.pixelStatistics.pixelsAboveThreshold} / {coverageResult.pixelStatistics.totalPixels} piksel eşik üstünde</p>
      <p>Ortalama tahmini kanal toplamı: {coverageResult.pixelStatistics.averageEstimatedChannelSumPercent.toFixed(1)}%</p>
      <button class="secondary" onclick={() => clearCoverage()}>Önizlemeyi kapat</button>
    {/if}
  </section>
</div>

<style>
  .prepress-panel { padding: 12px; color: var(--foreground); font-size: 12px; }
  section { margin-bottom: 12px; padding: 12px; border: 1px solid var(--border); border-radius: var(--radius); background: var(--card); }
  h4, h5 { margin: 0 0 8px; font-weight: 600; }
  h4 { font-size: 13px; }
  h5 { margin-top: 12px; font-size: 11px; color: var(--muted-foreground); }
  p { margin: 4px 0; }
  button { width: 100%; margin-top: 8px; padding: 6px 10px; border: 1px solid transparent; border-radius: calc(var(--radius) * 0.8); background: var(--primary); color: var(--primary-foreground); font-weight: 500; cursor: pointer; }
  button:hover:not(:disabled) { filter: brightness(1.08); }
  button:disabled { opacity: .5; cursor: not-allowed; }
  button.secondary { border-color: var(--border); background: var(--secondary); color: var(--secondary-foreground); }
  .permanent-warning { padding: 8px 10px; border-radius: calc(var(--radius) * 0.8); background: var(--warning); color: var(--warning-foreground); }
  .error { color: var(--destructive); overflow-wrap: anywhere; }
  .legend { margin: 8px 0; padding: 4px 8px; border-radius: 999px; background: #ffcc00; color: #1d1d1f; font-size: 11px; font-weight: 600; text-align: center; }
  .report { overflow-wrap: anywhere; }
  ul { padding-left: 18px; list-style: disc; }
  .boxes { display: grid; grid-template-columns: auto 1fr; gap: 4px 8px; }
  .boxes dt { color: var(--muted-foreground); }
  .boxes dd { margin: 0; font-family: var(--font-code); font-variant-numeric: tabular-nums; }
  .channels { display: flex; gap: 4px; }
  .channels label { flex: 1; padding: 4px; border: 1px solid var(--border); border-radius: calc(var(--radius) * 0.6); text-align: center; color: var(--muted-foreground); cursor: pointer; }
  .channels label.active { border-color: var(--primary); color: var(--foreground); background: color-mix(in srgb, var(--primary) 10%, transparent); }
  .channels input { display: none; }
  .threshold { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .threshold input { width: 70px; padding: 2px 6px; border: 1px solid var(--input); border-radius: calc(var(--radius) * 0.6); background: var(--background); }
</style>

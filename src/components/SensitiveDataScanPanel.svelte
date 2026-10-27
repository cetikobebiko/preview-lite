<script>
  import FeatureNotice from "./FeatureNotice.svelte";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";

  let { scanState, onScan = () => {}, onNavigate = () => {}, canPromote = false, onPromote = () => {} } = $props();

  const featureContext = getRuntimeFeatureContext();
  const scanPresentation = getFeaturePresentation("sensitive_data_scan", featureContext);

  // `builtin` patterns run Rust matchers (redaction::patterns): card numbers must pass the
  // Luhn check, phone numbers need grouping or a label, dates must be real dates.
  let patterns = $state([
    { label: "TC Kimlik Numarası", regex: "\\b[1-9][0-9]{10}\\b", active: true },
    { label: "Kredi Kartı", builtin: "credit-card", active: true },
    { label: "E-Posta", builtin: "email", active: true },
    { label: "IBAN", regex: "\\bTR\\d{2}\\s?(?:\\d{4}\\s?){5}\\d{2}\\b", active: true },
    { label: "Telefon Numarası", builtin: "phone", active: true },
    { label: "Tarih", builtin: "date", active: false },
    { label: "ABD Sosyal Güvenlik Numarası (SSN)", builtin: "ssn", active: false },
  ]);

  function handleScan() {
    onScan(patterns);
  }

  function handleItemClick(pageIndex) {
    if (typeof onNavigate === "function") {
      onNavigate(pageIndex);
    }
  }

  function getCategoryLabel(patternIndex) {
    return scanState?.capturedPatternLabels?.[patternIndex] || "Hassas Veri";
  }
</script>

<div class="sensitive-data-scan-panel">
  <FeatureNotice featureId="sensitive_data_scan" compact />

  <div class="warnings-box">
    <p>• Tarama text extraction ve seçili pattern’lerle sınırlıdır.</p>
    <p>• OCR yapılmamış görsel metni bulmayabilir.</p>
    <p>• Sonuçlar eksik veya hatalı olabilir.</p>
    <p>• Sonuçlar PDF’den içerik kaldırmaz.</p>
    <p>• Güvenli redaksiyon bu sürümde kullanılamaz.</p>
  </div>

  {#if scanState?.error}
    <p class="desc warning-text">{scanState.error}</p>
  {/if}

  <div class="patterns">
    {#each patterns as pattern}
      <label class="pattern-item">
        <input type="checkbox" bind:checked={pattern.active} />
        <span>{pattern.label}</span>
      </label>
    {/each}
  </div>

  <button
    class="scan-btn"
    onclick={handleScan}
    disabled={scanState?.isScanning || !scanPresentation.runnable}
  >
    {scanState?.isScanning ? "Taranıyor..." : "Belgeyi Tara"}
  </button>

  {#if scanState?.report}
    {#if (scanState.report.warnings && scanState.report.warnings.length > 0) || scanState.report.truncated}
      <div class="scan-warnings">
        {#each scanState.report.warnings as warning}
          <p class="warning-text">• {warning.message}</p>
        {/each}
        {#if scanState.report.truncated && !scanState.report.warnings.some((w) => w.code === "SENSITIVE_SCAN_ITEM_LIMIT_EXCEEDED")}
          <p class="warning-text">• Tarama sonuçları kısmen kesildi.</p>
        {/if}
      </div>
    {/if}

    {#if scanState.report.items && scanState.report.items.length > 0}
      <div class="results">
        <h4>Bulunanlar ({scanState.report.items.length})</h4>
        <div class="item-list">
          {#each scanState.report.items as item}
            <div
              class="result-item"
              role="button"
              tabindex="0"
              onclick={() => handleItemClick(item.pageIndex)}
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === " ") handleItemClick(item.pageIndex);
              }}
            >
              <div class="item-type">{getCategoryLabel(item.patternIndex)}</div>
              <div class="item-text">{item.textPreview}</div>
              <div class="item-page">Kaynak sayfa {item.pageIndex + 1}</div>
              {#if canPromote && item.geometryStatus === "resolved" && item.pageGeometry && item.sourceRectsPdfPoints?.length > 0}
                <button
                  type="button"
                  class="promote-btn"
                  onclick={(event) => { event.stopPropagation(); onPromote(item); }}
                >Redaksiyon adayı olarak ekle</button>
              {/if}
            </div>
          {/each}
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .sensitive-data-scan-panel {
    padding: 12px;
    color: var(--foreground);
    display: flex;
    flex-direction: column;
    min-height: 100%;
    font-size: 13px;
  }

  .warnings-box {
    font-size: 11px;
    color: var(--warning-foreground);
    background: var(--warning);
    padding: 10px;
    border-radius: var(--radius);
    margin: 8px 0 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .warnings-box p {
    margin: 0;
    line-height: 1.35;
  }

  .desc {
    font-size: 12px;
    color: var(--muted-foreground);
    margin-bottom: 12px;
  }

  .warning-text {
    color: var(--destructive);
    font-size: 12px;
    margin: 2px 0;
  }

  .scan-warnings {
    background: color-mix(in srgb, var(--destructive) 10%, transparent);
    border-radius: var(--radius);
    padding: 8px;
    margin-bottom: 12px;
  }

  .patterns {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 12px;
    background: var(--card);
    border: 1px solid var(--border);
    padding: 10px 12px;
    border-radius: var(--radius);
  }

  .pattern-item {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }

  .pattern-item input {
    accent-color: var(--primary);
  }

  .scan-btn {
    background: var(--primary);
    color: var(--primary-foreground);
    border: none;
    padding: 7px 10px;
    border-radius: calc(var(--radius) * 0.8);
    cursor: pointer;
    font-weight: 500;
    margin-bottom: 16px;
  }

  .scan-btn:hover:not(:disabled) {
    filter: brightness(1.08);
  }

  .scan-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .results {
    display: flex;
    flex-direction: column;
    flex: 1;
  }

  h4 {
    margin: 0 0 8px 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted-foreground);
  }

  .item-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .result-item {
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 8px 10px;
    cursor: pointer;
  }

  .result-item:hover {
    border-color: color-mix(in srgb, var(--primary) 45%, transparent);
    background: color-mix(in srgb, var(--primary) 6%, var(--card));
  }

  .item-type {
    font-size: 11px;
    color: var(--primary);
    font-weight: 600;
    margin-bottom: 4px;
  }

  .promote-btn {
    margin-top: 6px;
    border: 1px dashed var(--warning-foreground);
    border-radius: calc(var(--radius) * 0.6);
    background: var(--warning);
    color: var(--warning-foreground);
    padding: 3px 8px;
    cursor: pointer;
    font-size: 11px;
  }

  .item-text {
    font-family: var(--font-code);
    font-size: 12px;
    word-break: break-all;
    margin-bottom: 4px;
  }

  .item-page {
    font-size: 11px;
    color: var(--muted-foreground);
    text-align: right;
  }
</style>

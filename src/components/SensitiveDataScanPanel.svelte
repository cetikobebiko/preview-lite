<script>
  import FeatureNotice from "./FeatureNotice.svelte";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";

  let { scanState, onScan = () => {}, onNavigate = () => {} } = $props();

  const featureContext = getRuntimeFeatureContext();
  const scanPresentation = getFeaturePresentation("sensitive_data_scan", featureContext);

  let patterns = $state([
    { label: "TC Kimlik Numarası", regex: "\\b[1-9][0-9]{10}\\b", active: true },
    { label: "Kredi Kartı", regex: "\\b(?:\\d[ -]*?){13,16}\\b", active: true },
    { label: "E-Posta", regex: "\\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\.[A-Za-z]{2,}\\b", active: true },
    { label: "IBAN", regex: "\\bTR\\d{2}\\s?(?:\\d{4}\\s?){5}\\d{2}\\b", active: true },
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
  <h3>Hassas veri tespiti</h3>
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
            </div>
          {/each}
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .sensitive-data-scan-panel {
    padding: 16px;
    background: var(--bg-sidebar, #121212);
    color: var(--text-color, #e0e0e0);
    height: 100%;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--border-color, #333);
    width: 300px;
    overflow-y: auto;
  }

  h3 {
    margin: 0 0 8px 0;
    font-size: 16px;
    color: white;
  }

  .warnings-box {
    font-size: 11px;
    color: #a0a0a0;
    background: rgba(255, 255, 255, 0.04);
    padding: 10px;
    border-radius: 6px;
    margin-bottom: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    border: 1px solid rgba(255, 255, 255, 0.08);
  }

  .warnings-box p {
    margin: 0;
    line-height: 1.3;
  }

  .desc {
    font-size: 12px;
    color: #888;
    margin-bottom: 16px;
  }

  .warning-text {
    color: #ef4444;
    font-size: 12px;
    margin: 2px 0;
  }

  .scan-warnings {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    padding: 8px;
    margin-bottom: 12px;
  }

  .patterns {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 16px;
    background: rgba(255, 255, 255, 0.05);
    padding: 12px;
    border-radius: 6px;
  }

  .pattern-item {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    cursor: pointer;
  }

  .scan-btn {
    background: #3b82f6;
    color: white;
    border: none;
    padding: 10px;
    border-radius: 6px;
    cursor: pointer;
    font-weight: 500;
    margin-bottom: 16px;
    transition: background 0.2s;
  }

  .scan-btn:hover:not(:disabled) {
    background: #2563eb;
  }

  .scan-btn:disabled {
    background: #444;
    cursor: not-allowed;
  }

  .results {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }

  h4 {
    margin: 0 0 12px 0;
    font-size: 14px;
    border-bottom: 1px solid #333;
    padding-bottom: 8px;
  }

  .item-list {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .result-item {
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid #333;
    border-radius: 4px;
    padding: 8px;
    cursor: pointer;
    transition: background 0.2s;
  }

  .result-item:hover {
    background: rgba(59, 130, 246, 0.1);
    border-color: rgba(59, 130, 246, 0.3);
  }

  .item-type {
    font-size: 11px;
    color: #3b82f6;
    text-transform: uppercase;
    font-weight: 600;
    margin-bottom: 4px;
  }

  .item-text {
    font-family: monospace;
    font-size: 13px;
    word-break: break-all;
    margin-bottom: 4px;
  }

  .item-page {
    font-size: 11px;
    color: #888;
    text-align: right;
  }
</style>

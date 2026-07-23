<script>
  import { createEventDispatcher } from "svelte";

  export let isOpen = false;
  export let fileName = "";
  export let totalPages = 1;
  export let currentPage = 1;
  export let readingTimeSeconds = 0;
  export let scale = 1.0;

  const dispatch = createEventDispatcher();

  function close() {
    dispatch("close");
  }

  function formatTime(totalSecs) {
    const mins = Math.floor(totalSecs / 60);
    const secs = totalSecs % 60;
    if (mins > 0) {
      return `${mins} dk ${secs} sn`;
    }
    return `${secs} sn`;
  }

  $: progressPercent = Math.round((currentPage / Math.max(1, totalPages)) * 100);
  $: avgTimePerPage = totalPages > 0 ? Math.round(readingTimeSeconds / Math.max(1, currentPage)) : 0;
</script>

{#if isOpen}
  <div class="analytics-overlay" on:click={close} role="presentation" tabindex="-1">
    <div class="analytics-dialog" on:click|stopPropagation role="presentation">
      <div class="analytics-header">
        <div style="display: flex; align-items: center; gap: 8px;">
          <span style="font-size: 20px;">📊</span>
          <div>
            <h3 class="analytics-title">Oturum ve Okuma İstatistikleri</h3>
            <p class="analytics-subtitle">{fileName}</p>
          </div>
        </div>
        <button class="close-btn" on:click={close}>✕</button>
      </div>

      <div class="analytics-body">
        <!-- Progress Bar -->
        <div class="stat-card">
          <div class="stat-header">
            <span>Okuma İlerlemesi</span>
            <span class="stat-value">{progressPercent}%</span>
          </div>
          <div class="progress-bar-bg">
            <div class="progress-bar-fill" style="width: {progressPercent}%;"></div>
          </div>
          <span class="stat-subtext">Sayfa {currentPage} / {totalPages}</span>
        </div>

        <!-- Grid Cards -->
        <div class="stat-grid">
          <div class="stat-card">
            <span class="stat-label">⏱️ Toplam Okuma Süresi</span>
            <span class="stat-big-value">{formatTime(readingTimeSeconds)}</span>
          </div>

          <div class="stat-card">
            <span class="stat-label">📄 Sayfa Başına Ortalama Süre</span>
            <span class="stat-big-value">{formatTime(avgTimePerPage)}</span>
          </div>

          <div class="stat-card">
            <span class="stat-label">🔍 Hatırlanan Ölçek (Zoom)</span>
            <span class="stat-big-value">%{Math.round(scale * 100)}</span>
          </div>

          <div class="stat-card">
            <span class="stat-label">📍 Konum Hafızası</span>
            <span class="stat-big-value">Kayıtlı</span>
          </div>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .analytics-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: var(--bg-primary);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 180;
    animation: fadeIn 0.15s ease-out;
  }

  .analytics-dialog {
    width: 520px;
    max-width: 90vw;
    background: var(--bg-surface);
    border: 1px solid var(--border-raw);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .analytics-header {
    padding: 14px 18px;
    border-bottom: 1px solid var(--border-raw);
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: var(--bg-panel);
  }

  .analytics-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .analytics-subtitle {
    font-size: 11px;
    color: var(--text-muted);
  }

  .close-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    font-size: 16px;
    cursor: pointer;
  }

  .analytics-body {
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .stat-card {
    background: var(--bg-panel);
    border: 1px solid var(--border-raw);
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .stat-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .stat-value {
    color: var(--accent-industrial);
  }

  .progress-bar-bg {
    width: 100%;
    height: 8px;
    background: var(--bg-primary);
    overflow: hidden;
  }

  .progress-bar-fill {
    height: 100%;
    background: var(--accent-industrial);
    transition: width 0.3s ease;
  }

  .stat-subtext {
    font-size: 11px;
    color: var(--text-muted);
  }

  .stat-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .stat-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-muted);
  }

  .stat-big-value {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    margin-top: 2px;
  }
</style>

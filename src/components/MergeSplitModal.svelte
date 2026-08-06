<script>
  import { createEventDispatcher } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { mergeSessionPdfs, splitSessionPdf } from "../pdfState.js";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";

  export let currentFilePath = "";
  export let maxPages = 0;
  export let sessionId = "";
  export let contentGeneration = 0;

  const dispatch = createEventDispatcher();
  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("page_manipulation", featureContext);
  
  let mode = "merge"; // 'merge' or 'split'
  let isProcessing = false;
  
  // Merge state
  let filesToMerge = currentFilePath ? [currentFilePath] : [];
  
  // Split state
  let splitRangesText = "1-1"; 

  async function runPageAction(callback) {
    try {
      return await guardFeatureAction("page_manipulation", featureContext, callback);
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        alert(error.presentation?.warning || error.message);
        return;
      }
      throw error;
    }
  }

  async function handleAddFiles() {
    return await runPageAction(performAddFiles);
  }

  async function performAddFiles() {
    try {
      const selected = await open({
        multiple: true,
        filters: [{ name: 'PDF', extensions: ['pdf'] }]
      });
      if (selected) {
        let newFiles = Array.isArray(selected) ? selected : [selected];
        filesToMerge = [...filesToMerge, ...newFiles];
      }
    } catch(e) {
      console.error(e);
    }
  }

  function removeFile(index) {
    filesToMerge.splice(index, 1);
    filesToMerge = [...filesToMerge];
  }

  async function executeMerge() {
    return await runPageAction(performMerge);
  }

  async function performMerge() {
    if (filesToMerge.length < 2) {
      alert("En az iki dosya seçmelisiniz.");
      return;
    }
    isProcessing = true;
    try {
      const savePath = await save({
        defaultPath: "birlestirilmis.pdf",
        filters: [{ name: 'PDF', extensions: ['pdf'] }]
      });
      if (savePath) {
        if (!sessionId) throw new Error("Aktif document session bulunamadı.");
        const extraFiles = filesToMerge.filter(f => f !== currentFilePath);
        await mergeSessionPdfs(sessionId, contentGeneration, extraFiles, savePath);
        alert("Dosyalar başarıyla birleştirildi!");
        dispatch("close");
      }
    } catch(e) {
      alert("Birleştirme hatası: " + e);
    }
    isProcessing = false;
  }

  async function executeSplit() {
    return await runPageAction(performSplit);
  }

  async function performSplit() {
    if (!splitRangesText.trim()) return;
    
    // Basit ayrıştırma: "1-3, 5-7" -> [[1,3], [5,7]]
    let ranges = [];
    try {
      const parts = splitRangesText.split(",");
      for (const p of parts) {
        const bounds = p.trim().split("-");
        let start = parseInt(bounds[0]) - 1; // 0-based index
        let end = bounds.length > 1 ? parseInt(bounds[1]) - 1 : start;
        if (isNaN(start) || isNaN(end) || start < 0 || end >= maxPages) {
          throw new Error("Geçersiz aralık: " + p);
        }
        ranges.push([start, end]);
      }
    } catch(e) {
      alert("Hata: Sayfa aralığı formatı hatalı veya sınırların dışında. Örn: 1-5, 8, 11-13");
      return;
    }

    isProcessing = true;
    try {
      const saveDir = await open({
        directory: true,
        multiple: false
      });
      
      if (saveDir) {
        let outDir = Array.isArray(saveDir) ? saveDir[0] : saveDir;
        if (!sessionId) throw new Error("Aktif document session bulunamadı.");
        await splitSessionPdf(sessionId, contentGeneration, ranges, outDir);
        alert("PDF başarıyla bölündü ve klasöre kaydedildi!");
        dispatch("close");
      }
    } catch(e) {
      alert("Bölme hatası: " + e);
    }
    isProcessing = false;
  }
</script>

<div class="modal-overlay" transition:fade={{ duration: 150 }} on:click={() => dispatch("close")}>
  <div class="modal-content glass-panel" transition:fly={{ y: 20, duration: 250 }} on:click|stopPropagation>
      <div class="modal-header">
          <h3>Belge Araçları</h3>
          <div class="mode-tabs">
            <button class:active={mode === 'merge'} on:click={() => mode='merge'}>Birleştir</button>
            <button class:active={mode === 'split'} on:click={() => mode='split'}>Böl</button>
          </div>
      </div>
      
      <div class="modal-body">
        <FeatureNotice featureId="page_manipulation" />
        {#if mode === 'merge'}
          <div class="merge-section">
            <p class="hint">Birleştirilecek PDF dosyalarını seçin ve sıralayın:</p>
            <div class="file-list">
              {#each filesToMerge as file, i}
                <div class="file-item">
                  <span class="file-number">{i+1}.</span>
                  <span class="file-name">{file.split('/').pop().split('\\').pop()}</span>
                  <button class="remove-btn" on:click={() => removeFile(i)}>✕</button>
                </div>
              {/each}
            </div>
            <button class="tech-button add-file-btn" on:click={handleAddFiles} disabled={!featurePresentation.runnable}>+ Dosya Ekle</button>
          </div>
        {:else}
          <div class="split-section">
            <p class="hint">Geçerli belgenin toplam sayfa sayısı: {maxPages}</p>
            <div class="form-group">
                <label>Bölünecek Aralıklar (Örn: 1-3, 5, 8-10)</label>
                <input type="text" bind:value={splitRangesText} placeholder="1-5, 8, 11-13" class="tech-input" />
            </div>
            <p class="hint">Yukarıdaki her bir aralık için seçtiğiniz klasörde ayrı bir PDF oluşturulacaktır.</p>
          </div>
        {/if}
      </div>
      
      <div class="modal-footer">
          <button class="tech-button" on:click={() => dispatch("close")} disabled={isProcessing}>İptal</button>
          {#if mode === 'merge'}
            <button class="tech-button primary" on:click={executeMerge} disabled={isProcessing || !featurePresentation.runnable}>
              {isProcessing ? 'İşleniyor...' : 'Birleştir ve Kaydet'}
            </button>
          {:else}
            <button class="tech-button primary" on:click={executeSplit} disabled={isProcessing || !featurePresentation.runnable}>
              {isProcessing ? 'İşleniyor...' : 'Böl ve Kaydet'}
            </button>
          {/if}
      </div>
  </div>
</div>

<style>
  .modal-overlay {
      position: fixed;
      top: 0; left: 0; width: 100vw; height: 100vh;
      background: rgba(10, 10, 15, 0.7);
      backdrop-filter: blur(4px);
      display: flex; align-items: center; justify-content: center;
      z-index: 10000;
  }
  .modal-content {
      width: 480px; max-width: 90vw;
      border-radius: 12px;
      display: flex; flex-direction: column;
      background: var(--surface);
      border: 1px solid var(--border-color);
  }
  .modal-header {
      padding: 16px 24px;
      border-bottom: 1px solid var(--border-color);
      display: flex; justify-content: space-between; align-items: center;
  }
  .modal-header h3 { margin: 0; font-size: 1.2rem; }
  
  .mode-tabs {
    display: flex;
    background: rgba(0,0,0,0.2);
    border-radius: 8px;
    padding: 4px;
  }
  
  .mode-tabs button {
    background: transparent;
    border: none;
    color: var(--text-secondary);
    padding: 4px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 0.9rem;
  }
  
  .mode-tabs button.active {
    background: var(--primary);
    color: white;
  }
  
  .modal-body {
      padding: 24px;
  }
  .form-group {
      margin-bottom: 16px;
      display: flex; flex-direction: column; gap: 8px;
  }
  .tech-input {
      background: rgba(0,0,0,0.2);
      border: 1px solid rgba(255,255,255,0.1);
      color: var(--text-primary);
      padding: 10px 12px;
      border-radius: 8px;
      font-size: 0.95rem;
  }
  .hint {
      font-size: 0.85rem; color: var(--text-secondary); margin-bottom: 12px;
  }
  
  .file-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 16px;
    max-height: 200px;
    overflow-y: auto;
  }
  
  .file-item {
    display: flex;
    align-items: center;
    background: rgba(255,255,255,0.05);
    padding: 8px 12px;
    border-radius: 6px;
    gap: 8px;
  }
  
  .file-name {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  
  .remove-btn {
    background: transparent; border: none;
    color: #ef4444; cursor: pointer;
    font-size: 14px;
  }
  
  .add-file-btn {
    width: 100%;
    border-style: dashed;
    background: transparent;
  }
  
  .add-file-btn:hover {
    background: rgba(255,255,255,0.05);
  }
  
  .modal-footer {
      padding: 16px 24px;
      border-top: 1px solid var(--border-color);
      display: flex; justify-content: flex-end; gap: 12px;
  }
</style>

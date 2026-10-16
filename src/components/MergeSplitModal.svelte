<script>
  import { untrack } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { mergeSessionPdfs, splitSessionPdf } from "../pdfState.js";
  import { ask, open, save } from "@tauri-apps/plugin-dialog";
  import {
    outputConflictMessage,
    publishWithOverwriteConfirmation,
  } from "../services/app/outputConflict.js";
  import { exportErrorMessage } from "../services/app/exportErrorMessage.js";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { parsePageRanges, toPhysicalRanges } from "../utils/pageRanges.js";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";

  // pageOrder is the display order (viewer.pageOrder); typed numbers are display positions.
  let { currentFilePath = "", maxPages = 0, pageOrder = null, sessionId = "", contentGeneration = 0, onclose, onsuccess } = $props();

  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("page_manipulation", featureContext);
  
  let mode = $state("merge"); // 'merge' or 'split'
  let isProcessing = $state(false);
  
  // Merge state: seeded once from the file open when the modal mounts.
  let filesToMerge = $state(untrack(() => (currentFilePath ? [currentFilePath] : [])));
  
  // Split state
  let splitRangesText = $state("1-1");

  let errorMsg = $state("");

  function selectMode(nextMode) {
    mode = nextMode;
    errorMsg = "";
  }

  async function runPageAction(callback) {
    try {
      return await guardFeatureAction("page_manipulation", featureContext, callback);
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        errorMsg = error.presentation?.warning || error.message;
        return;
      }
      throw error;
    }
  }

  async function handleAddFiles() {
    return await runPageAction(performAddFiles);
  }

  async function performAddFiles() {
    errorMsg = "";
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
      errorMsg = "Dosya seçilemedi: " + exportErrorMessage(e);
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
    errorMsg = "";
    if (filesToMerge.length < 2) {
      errorMsg = "En az iki dosya seçmelisiniz.";
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
        onsuccess?.("Dosyalar başarıyla birleştirildi!");
        onclose?.();
      }
    } catch(e) {
      errorMsg = "Birleştirme hatası: " + exportErrorMessage(e);
    }
    isProcessing = false;
  }

  async function executeSplit() {
    return await runPageAction(performSplit);
  }

  async function performSplit() {
    errorMsg = "";
    const parsed = parsePageRanges(splitRangesText, maxPages, { mode: "ranges" });
    // The backend splits physical PDF page ranges.
    const physical = parsed.ok ? toPhysicalRanges(parsed.ranges, pageOrder) : parsed;
    if (!physical.ok) {
      errorMsg = physical.error;
      return;
    }
    const ranges = physical.ranges;

    isProcessing = true;
    try {
      const saveDir = await open({
        directory: true,
        multiple: false
      });
      
      if (saveDir) {
        let outDir = Array.isArray(saveDir) ? saveDir[0] : saveDir;
        if (!sessionId) throw new Error("Aktif document session bulunamadı.");
        const result = await publishWithOverwriteConfirmation(
          (overwrite) => splitSessionPdf(sessionId, contentGeneration, ranges, outDir, overwrite),
          (names) =>
            ask(outputConflictMessage(names), {
              title: "Dosyalar Zaten Var",
              kind: "warning",
              okLabel: "Üzerine Yaz",
              cancelLabel: "İptal",
            }),
        );
        if (result.status === "declined") {
          errorMsg = "Bölme iptal edildi; mevcut dosyalara dokunulmadı.";
        } else if (result.status === "completed") {
          onsuccess?.("PDF başarıyla bölündü ve klasöre kaydedildi!");
          onclose?.();
        }
      }
    } catch(e) {
      errorMsg = "Bölme hatası: " + exportErrorMessage(e);
    }
    isProcessing = false;
  }
</script>

<div class="modal-overlay" role="presentation" transition:fade={{ duration: 150 }} onclick={(event) => event.target === event.currentTarget && onclose?.()}>
  <div class="modal-content glass-panel" role="dialog" aria-modal="true" aria-labelledby="merge-split-modal-title" transition:fly={{ y: 20, duration: 250 }}>
      <div class="modal-header">
          <h3 id="merge-split-modal-title">Belge Araçları</h3>
          <div class="mode-tabs">
            <button class:active={mode === 'merge'} onclick={() => selectMode('merge')}>Birleştir</button>
            <button class:active={mode === 'split'} onclick={() => selectMode('split')}>Böl</button>
          </div>
      </div>
      
      <div class="modal-body">
        <FeatureNotice featureId="page_manipulation" />
        {#if errorMsg}
          <div class="error-msg" role="alert">{errorMsg}</div>
        {/if}
        {#if mode === 'merge'}
          <div class="merge-section">
            <p class="hint">Birleştirilecek PDF dosyalarını seçin ve sıralayın:</p>
            <div class="file-list">
              {#each filesToMerge as file, i}
                <div class="file-item">
                  <span class="file-number">{i+1}.</span>
                  <span class="file-name">{file.split('/').pop().split('\\').pop()}</span>
                  <button class="remove-btn" onclick={() => removeFile(i)}>✕</button>
                </div>
              {/each}
            </div>
            <button class="tech-button add-file-btn" onclick={handleAddFiles} disabled={!featurePresentation.runnable}>+ Dosya Ekle</button>
          </div>
        {:else}
          <div class="split-section">
            <p class="hint">Geçerli belgenin toplam sayfa sayısı: {maxPages}</p>
            <div class="form-group">
                <label for="split-ranges-input">Bölünecek Aralıklar (Örn: 1-3, 5, 8-10)</label>
                <input id="split-ranges-input" type="text" bind:value={splitRangesText} placeholder="1-5, 8, 11-13" class="tech-input" />
            </div>
            <p class="hint">Yukarıdaki her bir aralık için seçtiğiniz klasörde belge adı ve sayfa aralığıyla adlandırılan ayrı bir PDF oluşturulacaktır (ör. rapor_sayfa_1-3.pdf).</p>
          </div>
        {/if}
      </div>
      
      <div class="modal-footer">
          <button class="tech-button" onclick={() => onclose?.()} disabled={isProcessing}>İptal</button>
          {#if mode === 'merge'}
            <button class="tech-button primary" onclick={executeMerge} disabled={isProcessing || !featurePresentation.runnable}>
              {isProcessing ? 'İşleniyor...' : 'Birleştir ve Kaydet'}
            </button>
          {:else}
            <button class="tech-button primary" onclick={executeSplit} disabled={isProcessing || !featurePresentation.runnable}>
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
  
  .error-msg {
      background: rgba(255, 59, 48, 0.1); border: 1px solid rgba(255, 59, 48, 0.3);
      color: #ff3b30; padding: 10px; border-radius: 6px; margin-bottom: 16px;
      font-size: 0.9rem;
  }

  .modal-footer {
      padding: 16px 24px;
      border-top: 1px solid var(--border-color);
      display: flex; justify-content: flex-end; gap: 12px;
  }
</style>

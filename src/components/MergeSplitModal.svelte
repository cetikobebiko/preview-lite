<script>
  import { untrack } from "svelte";
  import XIcon from "@lucide/svelte/icons/x";
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { mergeSessionPdfs, splitSessionPdf } from "../pdfState.js";
  import { ask, open, save } from "@tauri-apps/plugin-dialog";
  import {
    outputConflictMessage,
    publishWithOverwriteConfirmation,
  } from "../services/app/outputConflict.js";
  import { exportErrorMessage } from "../services/app/exportErrorMessage.js";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { parsePageRanges, toPhysicalRanges } from "../utils/pageRanges.js";
  import { bookmarkRangesText, everyNPagesRangesText } from "../services/splitRanges.js";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";

  // pageOrder is the display order (viewer.pageOrder); typed numbers are display positions.
  // outline: top-level bookmarks (viewer.outlineList) for splitting at bookmarks.
  let { currentFilePath = "", maxPages = 0, pageOrder = null, outline = [], sessionId = "", contentGeneration = 0, onclose, onsuccess } = $props();

  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("page_manipulation", featureContext);
  
  let mode = $state("merge"); // 'merge' or 'split'
  let isProcessing = $state(false);
  
  // Merge state: seeded once from the file open when the modal mounts.
  let filesToMerge = $state(untrack(() => (currentFilePath ? [currentFilePath] : [])));
  
  // Split state
  let splitRangesText = $state("1-1");
  let splitEvery = $state(1);
  const bookmarkRanges = $derived(bookmarkRangesText(outline, maxPages, pageOrder));

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

<AppDialog title="Belge Araçları" dismissible={!isProcessing} onclose={() => onclose?.()}>
  <div class="grid gap-3">
    <div class="grid grid-cols-2 gap-0.5 rounded-md bg-black/5 p-0.5 dark:bg-white/10" role="group" aria-label="İşlem">
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={mode === 'merge'} onclick={() => selectMode('merge')}>Birleştir</button>
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={mode === 'split'} onclick={() => selectMode('split')}>Böl</button>
    </div>

    <FeatureNotice featureId="page_manipulation" />
    {#if errorMsg}
      <div class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{errorMsg}</div>
    {/if}

    {#if mode === 'merge'}
      <p class="text-xs text-muted-foreground">Birleştirilecek PDF dosyalarını seçin ve sıralayın:</p>
      <ol class="grid gap-1">
        {#each filesToMerge as file, i}
          <li class="flex items-center gap-2 rounded-md border bg-card px-2.5 py-1.5">
            <span class="text-xs text-muted-foreground tabular-nums">{i+1}.</span>
            <span class="min-w-0 flex-1 truncate">{file.split('/').pop().split('\\').pop()}</span>
            <button type="button" class="inline-flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground" aria-label="Listeden çıkar" onclick={() => removeFile(i)}><XIcon class="size-3.5" /></button>
          </li>
        {/each}
      </ol>
      <Button variant="outline" size="sm" class="justify-self-start" onclick={handleAddFiles} disabled={!featurePresentation.runnable}>+ Dosya Ekle</Button>
    {:else}
      <p class="text-xs text-muted-foreground">Geçerli belgenin toplam sayfa sayısı: {maxPages}</p>
      <div class="grid gap-1.5">
        <Label for="split-ranges-input" class="text-xs text-muted-foreground">Bölünecek Aralıklar (Örn: 1-3, 5, 8-10)</Label>
        <Input id="split-ranges-input" type="text" bind:value={splitRangesText} placeholder="1-5, 8, 11-13" />
      </div>
      <div class="flex flex-wrap items-center gap-2 text-xs">
        <span class="text-muted-foreground">Aralıkları doldur:</span>
        <Input class="h-8 w-16" type="number" min="1" max={maxPages} aria-label="Parça başına sayfa" bind:value={splitEvery} />
        <Button variant="outline" size="sm" onclick={() => (splitRangesText = everyNPagesRangesText(maxPages, Number(splitEvery)) || splitRangesText)}>Her N sayfada</Button>
        <Button variant="outline" size="sm" onclick={() => (splitRangesText = bookmarkRanges)} disabled={!bookmarkRanges} title={bookmarkRanges ? "" : "Belgede sayfaya giden üst düzey yer imi yok"}>Üst düzey yer imlerine göre</Button>
      </div>
      <p class="text-xs text-muted-foreground">Yukarıdaki her bir aralık için seçtiğiniz klasörde belge adı ve sayfa aralığıyla adlandırılan ayrı bir PDF oluşturulacaktır (ör. rapor_sayfa_1-3.pdf).</p>
    {/if}
  </div>

  {#snippet footer()}
    <Button variant="outline" size="sm" onclick={() => onclose?.()} disabled={isProcessing}>İptal</Button>
    {#if mode === 'merge'}
      <Button size="sm" onclick={executeMerge} disabled={isProcessing || !featurePresentation.runnable}>
        {isProcessing ? 'İşleniyor...' : 'Birleştir ve Kaydet'}
      </Button>
    {:else}
      <Button size="sm" onclick={executeSplit} disabled={isProcessing || !featurePresentation.runnable}>
        {isProcessing ? 'İşleniyor...' : 'Böl ve Kaydet'}
      </Button>
    {/if}
  {/snippet}
</AppDialog>

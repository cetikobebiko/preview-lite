<script>
  import { untrack } from "svelte";
  import XIcon from "@lucide/svelte/icons/x";
  import ArrowUpIcon from "@lucide/svelte/icons/arrow-up";
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { mergeSessionPdfs, splitSessionPdf } from "../pdfState.js";
  import { createPdfFromImages } from "../services/backend/document.js";
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
  let { currentFilePath = "", maxPages = 0, pageOrder = null, outline = [], sessionId = "", contentGeneration = 0, initialMode = "merge", onclose, onsuccess } = $props();

  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("page_manipulation", featureContext);
  
  let mode = $state(untrack(() => initialMode)); // 'merge', 'split' or 'images'
  let isProcessing = $state(false);
  
  // Merge state: seeded once from the file open when the modal mounts.
  let filesToMerge = $state(untrack(() => (currentFilePath ? [currentFilePath] : [])));
  
  // Split state
  let splitRangesText = $state("1-1");
  let splitEvery = $state(1);
  const bookmarkRanges = $derived(bookmarkRangesText(outline, maxPages, pageOrder));

  // Images to PDF: one page per image, in list order.
  const IMAGE_EXTENSIONS = ["jpg", "jpeg", "png", "webp", "bmp", "gif", "tif", "tiff"];
  let imagesToConvert = $state([]);
  let imagePageSize = $state("a4");
  let imageMargin = $state(36);

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

  async function addImages() {
    errorMsg = "";
    const selected = await open({ multiple: true, filters: [{ name: "Görsel", extensions: IMAGE_EXTENSIONS }] });
    if (selected) imagesToConvert = [...imagesToConvert, ...(Array.isArray(selected) ? selected : [selected])];
  }

  function moveImageUp(index) {
    if (index <= 0) return;
    const next = [...imagesToConvert];
    [next[index - 1], next[index]] = [next[index], next[index - 1]];
    imagesToConvert = next;
  }

  async function executeImagesToPdf() {
    errorMsg = "";
    if (imagesToConvert.length === 0) {
      errorMsg = "En az bir görsel seçin.";
      return;
    }
    const savePath = await save({ defaultPath: "gorseller.pdf", filters: [{ name: "PDF", extensions: ["pdf"] }] });
    if (!savePath) return;
    isProcessing = true;
    try {
      const margin = imagePageSize === "image" ? 0 : Math.min(144, Math.max(0, Number(imageMargin) || 0));
      await createPdfFromImages($state.snapshot(imagesToConvert), savePath, { pageSize: imagePageSize, margin });
      onsuccess?.(`${imagesToConvert.length} görselden PDF oluşturuldu.`);
      onclose?.();
    } catch (e) {
      errorMsg = "PDF oluşturulamadı: " + exportErrorMessage(e);
    } finally {
      isProcessing = false;
    }
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
    <div class="grid grid-cols-3 gap-0.5 rounded-md bg-black/5 p-0.5 dark:bg-white/10" role="group" aria-label="İşlem">
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={mode === 'merge'} onclick={() => selectMode('merge')}>Birleştir</button>
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={mode === 'split'} onclick={() => selectMode('split')}>Böl</button>
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={mode === 'images'} onclick={() => selectMode('images')}>Görsellerden PDF</button>
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
    {:else if mode === 'images'}
      <p class="text-xs text-muted-foreground">Her görsel bir sayfa olur. JPEG'ler yeniden sıkıştırılmadan eklenir.</p>
      <ol class="grid max-h-56 gap-1 overflow-y-auto">
        {#each imagesToConvert as file, i (i)}
          <li class="flex items-center gap-2 rounded-md border bg-card px-2.5 py-1.5">
            <span class="text-xs text-muted-foreground tabular-nums">{i + 1}.</span>
            <span class="min-w-0 flex-1 truncate">{file.split(/[\\/]/).pop()}</span>
            <button type="button" class="inline-flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-40" aria-label="Yukarı taşı" disabled={i === 0} onclick={() => moveImageUp(i)}><ArrowUpIcon class="size-3.5" /></button>
            <button type="button" class="inline-flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground" aria-label="Listeden çıkar" onclick={() => (imagesToConvert = imagesToConvert.filter((_, index) => index !== i))}><XIcon class="size-3.5" /></button>
          </li>
        {/each}
      </ol>
      <Button variant="outline" size="sm" class="justify-self-start" onclick={addImages} disabled={isProcessing}>+ Görsel Ekle</Button>
      <div class="grid grid-cols-2 gap-2">
        <div class="grid gap-1">
          <Label for="images-page-size" class="text-xs text-muted-foreground">Sayfa boyutu</Label>
          <select id="images-page-size" bind:value={imagePageSize} class="h-9 rounded-md border border-input bg-background px-2.5 text-sm">
            <option value="a4">A4 (görsele göre yatay/dikey)</option>
            <option value="letter">US Letter</option>
            <option value="image">Görsel boyutu (96 dpi)</option>
          </select>
        </div>
        <div class="grid gap-1">
          <Label for="images-margin" class="text-xs text-muted-foreground">Kenar boşluğu (pt)</Label>
          <Input id="images-margin" type="number" min="0" max="144" bind:value={imageMargin} disabled={imagePageSize === 'image'} />
        </div>
      </div>
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
    {:else if mode === 'images'}
      <Button size="sm" onclick={executeImagesToPdf} disabled={isProcessing || imagesToConvert.length === 0}>
        {isProcessing ? 'İşleniyor...' : 'PDF Oluştur ve Kaydet'}
      </Button>
    {:else}
      <Button size="sm" onclick={executeSplit} disabled={isProcessing || !featurePresentation.runnable}>
        {isProcessing ? 'İşleniyor...' : 'Böl ve Kaydet'}
      </Button>
    {/if}
  {/snippet}
</AppDialog>

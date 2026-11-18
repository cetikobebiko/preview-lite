<script>
  import AppDialog from "./AppDialog.svelte";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { Button } from "$lib/components/ui/button";
  import { save } from "@tauri-apps/plugin-dialog";
  import { startSearchablePdfExport } from "../pdfState.js";
  import { getOcrLanguages } from "../ocrDocumentState.js";
  import { getSessionFormInfo } from "../services/backend/session.js";
  import { parsePageRanges, toPhysicalPages } from "../utils/pageRanges.js";
  import {
    TESSERACT_INSTALL_HINT,
    preferredOcrLanguage,
    searchablePdfDetails,
    searchablePdfErrorMessage,
    searchablePdfSummary,
    selectableOcrLanguages,
  } from "../services/searchablePdf.js";

  let {
    sessionId = "",
    contentGeneration = 0,
    fileName = "",
    pageCount = 0,
    pageOrder = null,
    onsaved,
    onopen = null,
    onclose,
  } = $props();

  const selectClass = "h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50";

  let languages = $state([]);
  let languagesLoading = $state(true);
  let languageError = $state("");
  let lang = $state("");
  let scope = $state("all");
  let pageRange = $state("");
  let signatureCount = $state(0);
  let running = $state(false);
  let error = $state("");
  let report = $state(null);
  let savedTarget = $state("");
  let notice = $state("");
  /** Çalışan job'un handle'ı ve başlatıldığı (oturum, generation). */
  let activeJob = null;

  const canRun = $derived(Boolean(sessionId) && !running && !languagesLoading && Boolean(lang) && !languageError);

  $effect(() => {
    let disposed = false;
    languagesLoading = true;
    getOcrLanguages()
      .then((list) => {
        if (disposed) return;
        languages = selectableOcrLanguages(list);
        lang = preferredOcrLanguage(languages);
        languageError = languages.length === 0 ? `Tesseract için kurulu dil verisi yok. ${TESSERACT_INSTALL_HINT}` : "";
      })
      .catch(() => {
        if (!disposed) languageError = `Tesseract bulunamadı. ${TESSERACT_INSTALL_HINT}`;
      })
      .finally(() => {
        if (!disposed) languagesLoading = false;
      });
    return () => {
      disposed = true;
    };
  });

  // İmza uyarısı için; okunamazsa uyarı gösterilmez, backend raporu yine söyler.
  $effect(() => {
    const requestSession = sessionId;
    const requestGeneration = contentGeneration;
    signatureCount = 0;
    if (!requestSession) return;
    let disposed = false;
    Promise.resolve()
      .then(() => getSessionFormInfo(requestSession, requestGeneration))
      .then((info) => {
        if (!disposed) signatureCount = info?.signature_count ?? 0;
      })
      .catch(() => {});
    return () => {
      disposed = true;
    };
  });

  // Belge iş sürerken değişirse job iptal edilir; eski belgenin sonucu gösterilmez.
  $effect(() => {
    const currentSession = sessionId;
    const currentGeneration = contentGeneration;
    if (activeJob && (activeJob.sessionId !== currentSession || activeJob.generation !== currentGeneration)) {
      activeJob.stale = true;
      void Promise.resolve(activeJob.handle.cancel()).catch(() => {});
    }
  });

  function selectedPages() {
    if (scope === "all") return { ok: true, pages: null };
    const selection = parsePageRanges(pageRange, pageCount, { mode: "pages" });
    if (!selection.ok) return selection;
    return { ok: true, pages: toPhysicalPages(selection.pages, pageOrder) };
  }

  async function run() {
    error = "";
    notice = "";
    report = null;
    savedTarget = "";
    const selection = selectedPages();
    if (!selection.ok) {
      error = selection.error;
      return;
    }
    const stem = (fileName || "belge").replace(/\.pdf$/i, "");
    const target = await save({
      title: "Aranabilir kopyayı kaydet",
      defaultPath: `${stem}_aranabilir.pdf`,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!target) return;
    running = true;
    const job = { sessionId, generation: contentGeneration, stale: false, handle: null };
    try {
      job.handle = startSearchablePdfExport(sessionId, contentGeneration, target, lang, selection.pages);
      activeJob = job;
      const result = await job.handle.promise;
      if (job.stale) {
        notice = "Belge iş sürerken değişti; sonuç gösterilmiyor.";
        return;
      }
      report = result;
      savedTarget = target;
      onsaved?.(searchablePdfSummary(result));
    } catch (e) {
      if (job.stale) {
        notice = "Belge iş sürerken değiştiği için işlem iptal edildi; hiçbir dosya yazılmadı.";
      } else {
        error = searchablePdfErrorMessage(e);
      }
    } finally {
      if (activeJob === job) activeJob = null;
      running = false;
    }
  }

  function cancel() {
    if (activeJob) void Promise.resolve(activeJob.handle.cancel()).catch(() => {});
  }

  function close() {
    if (!running) onclose?.();
  }

  function openCopy() {
    const target = savedTarget;
    onclose?.();
    onopen?.(target);
  }
</script>

<AppDialog title="Aranabilir PDF Olarak Kaydet" description="Taranmış sayfalar Tesseract ile tanınır ve kelimeler görünmez metin olarak yerleştirilir; sayfalar görsel olarak değişmez. Sonuç ayrı bir dosyaya kaydedilir, açık belge değişmez." size="md" dismissible={!running} onclose={close}>
  <div class="grid gap-3">
    <div class="grid gap-1.5">
      <Label for="searchable-lang" class="text-[11px] text-muted-foreground">OCR dili</Label>
      <select id="searchable-lang" bind:value={lang} class={selectClass} disabled={running || languagesLoading || languages.length === 0}>
        {#if languagesLoading}<option value="">Diller yükleniyor…</option>{/if}
        {#each languages as language (language)}
          <option value={language}>{language}</option>
        {/each}
      </select>
      {#if !languagesLoading && languages.length > 0 && !languages.includes("tur")}
        <p class="text-[11px] text-muted-foreground">Türkçe (tur) dil verisi kurulu değil; Türkçe karakterler yanlış tanınabilir.</p>
      {/if}
    </div>

    <fieldset class="grid gap-1.5 text-xs" disabled={running}>
      <legend class="mb-1 text-[11px] text-muted-foreground">Sayfalar</legend>
      <label class="flex items-center gap-2"><input type="radio" name="searchable-scope" value="all" bind:group={scope} /> Bütün taranmış sayfalar</label>
      <label class="flex items-center gap-2"><input type="radio" name="searchable-scope" value="range" bind:group={scope} /> Sayfa aralığı</label>
      {#if scope === "range"}
        <Input id="searchable-range" placeholder="ör. 1-3, 7" bind:value={pageRange} aria-label="Sayfa aralığı" />
      {/if}
      <p class="text-[11px] text-muted-foreground">Zaten metni olan sayfalar her durumda atlanır; ikinci bir metin katmanı eklenmez.</p>
    </fieldset>

    {#if signatureCount > 0}
      <p class="rounded-md bg-amber-500/10 px-3 py-2 text-xs" role="note">Bu belge dijital imza içeriyor. Kopya baştan yazılır ve imzalar kopyada geçersiz olur; açık belgedeki imzalar etkilenmez.</p>
    {/if}
    <p class="text-[11px] text-muted-foreground">Deneysel: doğruluk taramanın kalitesine bağlıdır. Parola korumalı belgelerin aranabilir kopyası kaydedilemez.</p>

    {#if running}
      <div class="grid gap-1.5" role="status">
        <p class="text-xs">Sayfalar tanınıyor… Bu işlem sayfa başına birkaç saniye sürebilir.</p>
        <div class="h-1.5 w-full overflow-hidden rounded bg-black/10 dark:bg-white/10"><div class="searchable-progress h-full w-1/3 rounded bg-primary"></div></div>
      </div>
    {/if}
    {#if report}
      <div class="grid gap-1 rounded-md bg-black/5 px-3 py-2 text-xs dark:bg-white/10" role="status">
        <p class="font-medium">{searchablePdfSummary(report)}</p>
        {#each searchablePdfDetails(report) as line, index (index)}
          <p class="text-muted-foreground">{line}</p>
        {/each}
      </div>
    {/if}
    {#if notice}
      <p class="rounded-md bg-black/5 px-3 py-2 text-xs dark:bg-white/10" role="status">{notice}</p>
    {/if}
    {#if languageError}
      <p class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{languageError}</p>
    {/if}
    {#if error}
      <p class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{error}</p>
    {/if}
  </div>
  {#snippet footer()}
    {#if running}
      <Button variant="outline" size="sm" onclick={cancel}>İptal</Button>
    {:else}
      <Button variant="outline" size="sm" onclick={close}>Kapat</Button>
      {#if report && savedTarget && onopen}
        <Button variant="outline" size="sm" onclick={openCopy}>Kopyayı aç</Button>
      {/if}
    {/if}
    <Button size="sm" onclick={run} disabled={!canRun}>{running ? "Kaydediliyor…" : "Kopyayı kaydet…"}</Button>
  {/snippet}
</AppDialog>

<style>
  .searchable-progress {
    animation: searchable-indeterminate 1.2s ease-in-out infinite;
  }
  @keyframes searchable-indeterminate {
    from { transform: translateX(-100%); }
    to { transform: translateX(300%); }
  }
  @media (prefers-reduced-motion: reduce) {
    .searchable-progress { animation: none; width: 100%; opacity: 0.5; }
  }
</style>

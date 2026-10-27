<script>
  import { open } from "@tauri-apps/plugin-dialog";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";
  import { startDocumentCompare } from "../pdfState.js";
  import { COMPARE_KIND_LABELS, currentSide, otherSide } from "../utils/documentCompare.js";

  let { compareState, sessionId = "", contentGeneration = 0, onNavigate = () => {} } = $props();

  const presentation = getFeaturePresentation("document_compare", getRuntimeFeatureContext());
  const result = $derived(compareState.resultFor(sessionId, contentGeneration));
  const fileName = $derived(compareState.otherPath.split(/[\\/]/).pop() ?? "");
  let pickError = $state("");

  async function chooseFile() {
    pickError = "";
    try {
      const selected = await open({ multiple: false, filters: [{ name: "PDF", extensions: ["pdf"] }] });
      if (typeof selected === "string") compareState.otherPath = selected;
    } catch (error) {
      pickError = `Dosya seçilemedi: ${error?.message ?? error}`;
    }
  }

  function compare() {
    void compareState.run(startDocumentCompare, sessionId, contentGeneration);
  }

  function count(kind) {
    return result?.changes.filter((change) => change.kind === kind).length ?? 0;
  }
</script>

<div class="flex h-full flex-col gap-3 p-3 text-[13px]">
  <FeatureNotice featureId="document_compare" compact />

  <div class="flex flex-col gap-2 rounded-md border bg-card p-3">
    <div class="flex items-center gap-2">
      <button type="button" class="rounded-md border px-2 py-1 text-xs hover:bg-muted" onclick={chooseFile} disabled={compareState.isComparing}>
        PDF seç…
      </button>
      <span class="min-w-0 flex-1 truncate text-xs text-muted-foreground" title={compareState.otherPath}>
        {fileName || "Karşılaştırılacak dosya seçilmedi"}
      </span>
    </div>
    <fieldset class="flex flex-col gap-1 text-xs" disabled={compareState.isComparing}>
      <legend class="mb-1 text-muted-foreground">Açık belge</legend>
      <label class="flex items-center gap-2">
        <input type="radio" name="compare-role" checked={!compareState.currentIsOld} onchange={() => (compareState.currentIsOld = false)} />
        Yeni sürüm (seçilen dosya eski)
      </label>
      <label class="flex items-center gap-2">
        <input type="radio" name="compare-role" checked={compareState.currentIsOld} onchange={() => (compareState.currentIsOld = true)} />
        Eski sürüm (seçilen dosya yeni)
      </label>
    </fieldset>
    {#if compareState.isComparing}
      <button type="button" class="rounded-md border px-2 py-1.5 text-xs font-medium hover:bg-muted" onclick={() => compareState.cancel()}>
        Karşılaştırılıyor… İptal et
      </button>
    {:else}
      <button
        type="button"
        class="rounded-md bg-primary px-2 py-1.5 text-xs font-medium text-primary-foreground disabled:opacity-50"
        onclick={compare}
        disabled={!compareState.otherPath || !sessionId || !presentation.runnable}
      >Karşılaştır</button>
    {/if}
  </div>

  {#if pickError || compareState.error}
    <p class="rounded-md border border-destructive/30 bg-destructive/10 p-2 text-xs text-destructive">{pickError || compareState.error}</p>
  {/if}

  {#if result}
    <div class="text-xs">
      {#if result.totalChanges === 0}
        <p class="font-medium">Metin farkı bulunamadı.</p>
      {:else}
        <p class="font-medium">{result.totalChanges} değişiklik: {count("replaced")} değişti, {count("inserted")} eklendi, {count("deleted")} silindi{result.totalChanges > result.changes.length ? " (gösterilenler)" : ""}.</p>
      {/if}
      {#if result.oldWords === 0 || result.newWords === 0}
        <p class="mt-1 text-warning-foreground">{result.oldWords === 0 ? "Eski" : "Yeni"} sürümde metin bulunamadı; belge taranmış olabilir.</p>
      {/if}
      {#if result.totalChanges > result.changes.length}
        <p class="mt-1 text-warning-foreground">İlk {result.changes.length} değişiklik gösteriliyor.</p>
      {/if}
    </div>

    <ol class="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto">
      {#each result.changes as change, index (index)}
        {@const mine = currentSide(change, result.currentIsOld)}
        {@const theirs = otherSide(change, result.currentIsOld)}
        <li>
          <button
            type="button"
            class="compare-change w-full rounded-md border p-2 text-left hover:bg-muted"
            data-kind={change.kind}
            onclick={() => onNavigate(mine.pageIndex)}
          >
            <span class="flex justify-between text-[11px] text-muted-foreground">
              <span class="kind font-medium">{COMPARE_KIND_LABELS[change.kind]}</span>
              <span>Sayfa {mine.pageIndex + 1}{theirs.pageIndex !== mine.pageIndex ? ` · seçilen dosyada ${theirs.pageIndex + 1}` : ""}</span>
            </span>
            {#if change.old.text}<span class="block break-words line-through decoration-red-500/70">{change.old.text}</span>{/if}
            {#if change.new.text}<span class="block break-words text-green-700 dark:text-green-400">{change.new.text}</span>{/if}
          </button>
        </li>
      {/each}
    </ol>
  {/if}
</div>

<style>
  .compare-change[data-kind="inserted"] .kind { color: rgb(22 163 74); }
  .compare-change[data-kind="deleted"] .kind { color: rgb(220 38 38); }
  .compare-change[data-kind="replaced"] .kind { color: rgb(37 99 235); }
</style>

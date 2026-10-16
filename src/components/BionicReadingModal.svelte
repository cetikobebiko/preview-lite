<script>
  import AppDialog from "./AppDialog.svelte";
  import { BIONIC_EMPTY_MESSAGE, toBionicLines } from "../utils/bionicText.js";

  let { isOpen = false, text = "", source = null, onclose } = $props();

  const bionicLines = $derived(toBionicLines(text));
  // Bionic Reading covers a single page; say which one and how its text was obtained.
  const sourceLabel = $derived(
    source && Number.isInteger(source.pageNumber)
      ? `Yalnız sayfa ${source.pageNumber} · ${source.method === "ocr" ? "OCR ile tanınan metin" : "PDF metin katmanı"}`
      : "",
  );

  function close() {
    onclose?.();
  }
</script>

{#if isOpen}
  <AppDialog title="Biyonik Okuma Katmanı (Bionic Reading)" description="İlk harfleri vurgulayarak hızlı okuma ve odaklanmayı artırır" size="lg" onclose={close}>
    {#if sourceLabel}<p class="mb-3 text-xs text-muted-foreground" data-testid="bionic-source">{sourceLabel}</p>{/if}
    <div class="bionic-text-content text-[15px] leading-7" data-testid="bionic-text">
          {#if bionicLines.length === 0}{BIONIC_EMPTY_MESSAGE}{:else}{#each bionicLines as words, lineIndex}{#if lineIndex > 0}<br />{/if}{#each words as word, wordIndex}{#if wordIndex > 0}{" "}{/if}<b>{word.bold}</b>{word.rest}{/each}{/each}{/if}
        </div>
  </AppDialog>
{/if}

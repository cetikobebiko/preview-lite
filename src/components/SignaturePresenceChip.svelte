<script>
  // Signature presence indicator (Prompt 11 G2). It reads the viewer worker's existing
  // form info (PDFium count, which includes empty signature fields): it says that the
  // document has signature fields, not that they are signed or valid.
  import { getFormInfo } from "../pdfState.js";

  let { sessionId = "", contentGeneration = 0, visible = false, onOpen = () => {}, lookup = getFormInfo } = $props();

  let hasSignatureFields = $state(false);
  let sequence = 0;

  $effect(() => {
    const session = sessionId;
    const generation = contentGeneration;
    const request = ++sequence;
    hasSignatureFields = false;
    if (!visible || !session) return;
    Promise.resolve()
      .then(() => lookup(session, generation))
      .then((info) => {
        if (request === sequence) hasSignatureFields = (info?.signature_count ?? 0) > 0;
      })
      .catch(() => {
        // Form info is optional here (form_editing may be off); the menu still opens the panel.
      });
  });
</script>

{#if visible && hasSignatureFields}
  <button
    type="button"
    class="shrink-0 rounded-md border px-2 py-0.5 text-[11px] text-muted-foreground hover:bg-muted hover:text-foreground"
    title="Belgede imza alanı var; imzaları incelemek için tıklayın"
    data-testid="signature-presence-chip"
    onclick={onOpen}
  >İmza alanı · İncele</button>
{/if}

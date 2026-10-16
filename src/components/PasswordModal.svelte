<script>
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";
  import { cn } from "$lib/utils";

  let { isOpen = false, fileName = "", errorMsg = "", onsubmit, oncancel } = $props();

  let password = $state("");
  let inputRef = $state();

  $effect(() => {
    if (isOpen && inputRef) {
      setTimeout(() => inputRef?.focus(), 50);
    }
  });

  function handleSubmit() {
    if (!password) return;
    onsubmit?.(password);
    // Güvenlik: Parolayı Svelte state'inden hemen sil
    password = "";
  }

  function handleCancel() {
    password = "";
    oncancel?.();
  }

  function handleKeydown(e) {
    if (e.key === "Enter") {
      handleSubmit();
    } else if (e.key === "Escape") {
      handleCancel();
    }
  }
</script>

{#if isOpen}
  <!-- Not dismissible from outside: the attempt is answered by unlocking or
       cancelling, and Escape in the field cancels as before. -->
  <AppDialog title="Parola gerekli" size="sm" dismissible={false}>
    <div class="grid gap-3">
      <p class="text-muted-foreground">
        <strong class="text-foreground">{fileName}</strong> dosyası parola ile korunuyor. İçeriği görüntülemek için lütfen parolayı girin.
      </p>
      <input
        bind:this={inputRef}
        type="password"
        bind:value={password}
        onkeydown={handleKeydown}
        placeholder="Parolayı girin..."
        aria-label="Parola"
        aria-invalid={errorMsg ? "true" : undefined}
        class={cn("h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50", errorMsg && "border-destructive")}
      />
      {#if errorMsg}
        <div class="text-xs text-destructive" role="alert">{errorMsg}</div>
      {/if}
    </div>
    {#snippet footer()}
      <Button variant="outline" size="sm" onclick={handleCancel}>İptal</Button>
      <Button size="sm" onclick={handleSubmit} disabled={!password}>Kilidi Aç</Button>
    {/snippet}
  </AppDialog>
{/if}

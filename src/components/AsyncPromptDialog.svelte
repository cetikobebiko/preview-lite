<script>
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";

  let { coordinator } = $props();

  let value = $state("");
  let inputRef = $state();

  $effect(() => {
    const request = coordinator.active;
    if (!request) return;
    value = request.initialValue ?? "";
    queueMicrotask(() => inputRef?.focus());
  });

  function submit(event) {
    event.preventDefault();
    coordinator.submit(value);
  }
</script>

{#if coordinator.active}
  <AppDialog title={coordinator.active.title} size="sm" outsideClose={false} onclose={() => coordinator.cancel()}>
    <form id="async-prompt-form" class="grid gap-1.5" onsubmit={submit}>
      <Label for="async-prompt-input" class="text-xs text-muted-foreground">{coordinator.active.label}</Label>
      <Input
        id="async-prompt-input"
        bind:ref={inputRef}
        bind:value
        placeholder={coordinator.active.placeholder}
      />
    </form>
    {#snippet footer()}
      <Button variant="outline" size="sm" onclick={() => coordinator.cancel()}>İptal</Button>
      <Button type="submit" size="sm" form="async-prompt-form">Tamam</Button>
    {/snippet}
  </AppDialog>
{/if}

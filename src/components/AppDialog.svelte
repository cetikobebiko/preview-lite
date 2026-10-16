<script>
  import XIcon from "@lucide/svelte/icons/x";
  import * as Dialog from "$lib/components/ui/dialog";
  import { cn } from "$lib/utils";

  // Shared modal shell over the shadcn dialog: portalled, focus-trapped and
  // labelled by its title. Escape, an outside click and the close button all
  // go through onclose; the parent stays the owner of whether it is open.
  // `dismissible={false}` is for flows that must be answered explicitly;
  // `outsideClose={false}` keeps Escape but ignores clicks outside.
  let {
    open = true,
    title,
    description = "",
    onclose,
    size = "md",
    dismissible = true,
    outsideClose = dismissible,
    class: className = "",
    bodyClass = "",
    children,
    footer,
  } = $props();

  const widths = { sm: "sm:max-w-sm", md: "sm:max-w-lg", lg: "sm:max-w-2xl", xl: "sm:max-w-4xl" };
</script>

<Dialog.Root bind:open={() => open, (next) => { if (!next) onclose?.(); }}>
  <Dialog.Content
    class={cn("flex max-h-[85vh] flex-col gap-0 overflow-hidden p-0", widths[size], className)}
    showCloseButton={false}
    onEscapeKeydown={(event) => { if (!dismissible) event.preventDefault(); }}
    onInteractOutside={(event) => { if (!dismissible || !outsideClose) event.preventDefault(); }}
  >
    <Dialog.Header class="shrink-0 gap-1 px-5 pt-5 pb-3 pr-12 text-left">
      <Dialog.Title class="text-[15px] font-semibold">{title}</Dialog.Title>
      {#if description}<Dialog.Description class="text-[13px]">{description}</Dialog.Description>{/if}
    </Dialog.Header>
    {#if dismissible}
      <button
        type="button"
        class="absolute top-4 right-4 inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
        aria-label="Kapat"
        onclick={() => onclose?.()}
      ><XIcon class="size-4" /></button>
    {/if}
    <div class={cn("min-h-0 flex-1 overflow-y-auto px-5 pb-5 text-[13px]", bodyClass)}>
      {@render children?.()}
    </div>
    {#if footer}
      <div class="flex shrink-0 justify-end gap-2 border-t bg-muted/40 px-5 py-3">
        {@render footer()}
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>

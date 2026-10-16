<script>
  import * as Command from "$lib/components/ui/command";
  import * as Dialog from "$lib/components/ui/dialog";
  import { filterCommands, selectCommand } from "../commandRegistry.js";
  import { FeatureUnavailableError } from "../featureActionGuard.js";
  import { getRuntimeFeatureContext } from "../featureRegistry.js";
  import { badgeLabel } from "../services/app/toolbarMenu.js";

  let {
    isOpen = $bindable(false),
    context = null,
    onselect = () => {},
    onclose = () => {},
  } = $props();

  const runtimeFeatureContext = getRuntimeFeatureContext();

  let searchQuery = $state("");
  let selectedId = $state("");
  let unavailableMessage = $state("");

  let featureContext = $derived(context || runtimeFeatureContext);
  // Filtering stays in the registry (Turkish casing, feature visibility), so
  // bits-ui only handles focus and keyboard selection.
  let filteredCommands = $derived(filterCommands(searchQuery, featureContext));

  $effect.pre(() => {
    if (isOpen) {
      searchQuery = "";
      unavailableMessage = "";
    }
  });

  $effect.pre(() => {
    const selectable = filteredCommands.filter((command) => command.presentation.runnable);
    if (!selectable.some((command) => command.id === selectedId)) {
      selectedId = selectable[0]?.id ?? "";
    }
  });

  async function executeCommand(command) {
    unavailableMessage = "";
    try {
      await selectCommand(command, featureContext, (id) => {
        if (typeof onselect === "function") {
          onselect({ detail: id });
        }
      });
      close();
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        unavailableMessage = command?.presentation?.warning || error.message;
        return;
      }
      throw error;
    }
  }

  function close() {
    isOpen = false;
    if (typeof onclose === "function") {
      onclose();
    }
  }
</script>

<Dialog.Root bind:open={() => isOpen, (open) => { if (!open) close(); }}>
  <Dialog.Content class="top-[16%] translate-y-0 gap-0 overflow-hidden p-0 sm:max-w-xl" showCloseButton={false}>
    <Dialog.Title class="sr-only">Komut paleti</Dialog.Title>
    <Command.Root shouldFilter={false} loop bind:value={selectedId} label="Komutlar" class="rounded-none! p-0">
      <div class="border-b p-2">
        <Command.Input bind:value={searchQuery} placeholder="Komut veya işlem arayın... (Cmd + K)" class="cmd-input" />
      </div>
      <Command.List class="max-h-[min(420px,60vh)] p-1.5">
        {#each filteredCommands as cmd (cmd.id)}
          <!-- Unavailable commands are skipped by keyboard selection and show
               their warning inline; selectCommand still guards every run. -->
          <Command.Item
            value={cmd.id}
            class="cmd-item items-start gap-3 rounded-md px-2.5 py-2"
            disabled={!cmd.presentation.runnable}
            aria-describedby={cmd.presentation.warning ? `command-warning-${cmd.id}` : undefined}
            onSelect={() => executeCommand(cmd)}
          >
            <span class="mt-px w-5 shrink-0 text-center text-base leading-5" aria-hidden="true">{cmd.icon}</span>
            <div class="flex min-w-0 flex-1 flex-col">
              <span class="truncate text-[13px] text-foreground">{cmd.title}</span>
              <span class="text-[11px] text-muted-foreground">
                {cmd.category}
                {#if cmd.presentation.badge}
                  · {badgeLabel(cmd.presentation.badge)}
                {/if}
              </span>
              {#if cmd.presentation.warning}
                <span class="mt-0.5 text-[11px] text-warning-foreground" id={`command-warning-${cmd.id}`}>
                  {cmd.presentation.warning}
                </span>
              {/if}
            </div>
            {#if cmd.shortcut}
              <Command.Shortcut>{cmd.shortcut}</Command.Shortcut>
            {/if}
          </Command.Item>
        {:else}
          <div class="py-8 text-center text-[13px] text-muted-foreground">Komut bulunamadı</div>
        {/each}
      </Command.List>
    </Command.Root>

    {#if unavailableMessage}
      <div class="cmd-unavailable-message border-t bg-warning px-3 py-2 text-xs text-warning-foreground" role="status">
        {unavailableMessage}
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>

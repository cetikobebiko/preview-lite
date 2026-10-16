<script>
  import MousePointerIcon from "@lucide/svelte/icons/mouse-pointer-2";
  import HighlighterIcon from "@lucide/svelte/icons/highlighter";
  import UnderlineIcon from "@lucide/svelte/icons/underline";
  import TypeIcon from "@lucide/svelte/icons/type";
  import Settings2Icon from "@lucide/svelte/icons/settings-2";
  import * as Popover from "$lib/components/ui/popover";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import * as Tooltip from "$lib/components/ui/tooltip";
  import { Slider } from "$lib/components/ui/slider";
  import { buttonVariants } from "$lib/components/ui/button";
  import { cn } from "$lib/utils";
  import { badgeLabel } from "../services/app/toolbarMenu.js";

  let { annotation, ui, featureUi, colorPalette } = $props();

  const annotationState = $derived(featureUi("annotation"));
  const tools = [
    { value: "pointer", label: "Seçim", icon: MousePointerIcon, gated: false },
    { value: "highlight", label: "Vurgula", icon: HighlighterIcon, gated: true },
    { value: "underline", label: "Altını çiz", icon: UnderlineIcon, gated: true },
    { value: "text", label: "Metin notu", icon: TypeIcon, gated: true },
  ];
</script>

<Tooltip.Provider delayDuration={600}>
  <div class="markup-toolbar flex h-10 shrink-0 items-center gap-2 border-b bg-toolbar px-3" role="toolbar" aria-label="İşaretleme araçları">
    <ToggleGroup.Root
      type="single"
      size="sm"
      spacing={1}
      bind:value={() => annotation.activeTool, (tool) => { if (tool) annotation.activeTool = tool; }}
    >
      {#each tools.filter((tool) => !tool.gated || annotationState.visible) as tool (tool.value)}
        {@const disabled = tool.gated && !annotationState.runnable}
        <Tooltip.Root>
          <Tooltip.Trigger>
            {#snippet child({ props })}
              <ToggleGroup.Item {...props} value={tool.value} aria-label={tool.label} {disabled} class="text-foreground/75 data-[state=on]:bg-accent data-[state=on]:text-foreground">
                <tool.icon />
              </ToggleGroup.Item>
            {/snippet}
          </Tooltip.Trigger>
          <Tooltip.Content side="bottom" sideOffset={6}>{tool.gated && annotationState.warning ? annotationState.warning : tool.label}</Tooltip.Content>
        </Tooltip.Root>
      {/each}
    </ToggleGroup.Root>

    {#if annotationState.visible}
      <div class="h-5 w-px bg-border" aria-hidden="true"></div>

      <label
        class={cn("relative size-6 cursor-pointer rounded-full border border-black/10 shadow-xs dark:border-white/15", !annotationState.runnable && "pointer-events-none opacity-50")}
        style:background-color={annotation.activeColor}
        title="Açıklama rengi"
      >
        <input type="color" class="sr-only" aria-label="Açıklama rengi" bind:value={annotation.activeColor} disabled={!annotationState.runnable} />
      </label>

      <Popover.Root bind:open={() => ui.settingsPopover && annotationState.runnable, (open) => (ui.settingsPopover = open)}>
        <Popover.Trigger
          class={cn(buttonVariants({ variant: "ghost", size: "icon-sm" }), "text-foreground/75 hover:text-foreground")}
          aria-label="Çizim ayarları"
          disabled={!annotationState.runnable}
        >
          <Settings2Icon />
        </Popover.Trigger>
        <Popover.Content align="start" class="w-56 space-y-3">
          <div class="space-y-1.5">
            <p class="text-xs font-medium text-muted-foreground">Kalem rengi</p>
            <div class="flex gap-2">
              {#each colorPalette as color (color)}
                <button
                  type="button"
                  class={cn("size-6 rounded-full border border-black/10 dark:border-white/15", annotation.strokeColor === color && "ring-2 ring-primary ring-offset-2 ring-offset-popover")}
                  style:background-color={color}
                  aria-label="Renk {color}"
                  aria-pressed={annotation.strokeColor === color}
                  onclick={() => (annotation.strokeColor = color)}
                ></button>
              {/each}
            </div>
          </div>
          <div class="space-y-1.5">
            <p class="flex justify-between text-xs font-medium text-muted-foreground">
              Kalınlık <span class="tabular-nums">{annotation.strokeSize} px</span>
            </p>
            <Slider type="single" min={1} max={16} step={1} bind:value={annotation.strokeSize} aria-label="Kalem kalınlığı" />
          </div>
        </Popover.Content>
      </Popover.Root>

      {#if annotationState.badge}
        <span class="rounded-sm bg-muted px-1.5 text-[10px] text-muted-foreground" title={annotationState.warning}>{badgeLabel(annotationState.badge)}</span>
      {/if}
    {/if}
  </div>
</Tooltip.Provider>

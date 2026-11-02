<script>
  import PanelLeftIcon from "@lucide/svelte/icons/panel-left";
  import PenToolIcon from "@lucide/svelte/icons/pen-tool";
  import SearchIcon from "@lucide/svelte/icons/search";
  import ShareIcon from "@lucide/svelte/icons/share";
  import ImageDownIcon from "@lucide/svelte/icons/image-down";
  import FolderOpenIcon from "@lucide/svelte/icons/folder-open";
  import EllipsisIcon from "@lucide/svelte/icons/ellipsis";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
  import * as Tooltip from "$lib/components/ui/tooltip";
  import { Button, buttonVariants } from "$lib/components/ui/button";
  import { cn } from "$lib/utils";
  import ToolbarButton from "./ToolbarButton.svelte";
  import ToolbarNavigation from "./ToolbarNavigation.svelte";
  import { buildToolbarMenu } from "../services/app/toolbarMenu.js";
  import { TRAFFIC_LIGHT_INSET_PX, usesOverlayTitleBar } from "../services/platform/titleBar.js";

  let {
    isPdf, viewer, ui, annotation, fileName, activePdfDisplayPath, featureUi, isDirty,
    onFeatureAction, onSelectFile, onGoBack, onGoForward, onPreviousPage, onNextPage, onPageInputChange,
    onRotateLeft, onRotateRight, onFlipH, onFlipV, onZoomOut, onResetZoom, onZoomIn,
    onToggleSinglePage, onToggleBookMode, onToggleDarkMode, onOpenBionic, onRunOcr,
    onSpeak, onStopSpeaking, onSave, onExportImage,
  } = $props();

  const overlayTitleBar = usesOverlayTitleBar();
  const hasDocument = $derived(isPdf || Boolean(viewer.activeImageSourcePath));
  const annotationVisible = $derived(featureUi("annotation").visible);
  // The current page's PDF label, shown only when it says something the position does not.
  const currentPageLabel = $derived.by(() => {
    const physical = viewer.pageOrder[viewer.currentPageIndex] ?? viewer.currentPageIndex;
    const label = viewer.pageLabels?.[physical] ?? "";
    return label && label !== String(viewer.currentPageIndex + 1) ? label : "";
  });
  const menuSections = $derived(buildToolbarMenu({
    isPdf, hasDocument, viewer, ui, featureUi, onFeatureAction,
    actions: {
      onSelectFile, onGoBack, onGoForward, onToggleSinglePage, onToggleBookMode, onToggleDarkMode,
      onOpenBionic, onRunOcr, onSpeak, onStopSpeaking,
    },
  }));

  function toggleMarkup() {
    ui.togglePanel("markup");
    // A hidden markup row must not leave a drawing tool armed on the page.
    if (!ui.panels.markup) annotation.activeTool = "pointer";
  }
</script>

<Tooltip.Provider delayDuration={600}>
  <header
    class="app-toolbar flex h-[52px] shrink-0 items-center gap-1 border-b bg-toolbar pr-2 pl-2"
    style:padding-left={overlayTitleBar ? `${TRAFFIC_LIGHT_INSET_PX}px` : undefined}
    data-tauri-drag-region
  >
    {#if isPdf}
      <ToolbarButton label="Kenar çubuğu" hint="Kenar çubuğunu göster/gizle (⌘B)" pressed={ui.panels.sidebar} onclick={() => ui.togglePanel("sidebar")}><PanelLeftIcon /></ToolbarButton>
    {/if}

    <div class="ml-1 flex min-w-0 flex-1 flex-col justify-center leading-tight" data-tauri-drag-region>
      <span class="truncate text-[13px] font-semibold" title={isPdf ? activePdfDisplayPath : viewer.activeImageSourcePath} data-tauri-drag-region>
        {hasDocument ? fileName : "Antigravity"}
      </span>
      {#if isPdf && viewer.pageOrder.length > 0}
        <span class="flex items-center gap-1 text-[11px] text-muted-foreground" data-tauri-drag-region>
          Sayfa
          <input
            type="number"
            aria-label="Sayfa numarası"
            class="h-4 rounded-sm bg-transparent text-center tabular-nums outline-none [appearance:textfield] hover:bg-muted focus:bg-background focus:ring-1 focus:ring-ring [&::-webkit-inner-spin-button]:appearance-none"
            style:width="calc({String(viewer.pageOrder.length).length}ch + 8px)"
            value={viewer.currentPageIndex + 1}
            min="1"
            max={viewer.pageOrder.length}
            onchange={onPageInputChange}
          />
          / {viewer.pageOrder.length}{#if currentPageLabel}<span class="truncate" title="Sayfa etiketi">({currentPageLabel})</span>{/if}{isDirty ? " — Düzenlendi" : ""}
        </span>
      {:else if hasDocument && !isPdf}
        <span class="text-[11px] text-muted-foreground" data-tauri-drag-region>{viewer.fileExt?.toUpperCase()} görseli</span>
      {/if}
    </div>

    {#if hasDocument}
      <ToolbarNavigation {isPdf} {viewer} {onPreviousPage} {onNextPage} {onRotateLeft} {onRotateRight} {onFlipH} {onFlipV} {onZoomOut} {onResetZoom} {onZoomIn} />
      <div class="mx-1.5 h-5 w-px bg-border" aria-hidden="true"></div>
    {/if}

    {#if annotationVisible && isDirty}
      <Button variant="secondary" size="sm" class="h-7 px-3 text-xs" title="Çalışma kopyasını atomik olarak kaydet" onclick={onSave}>Kaydet</Button>
    {/if}
    {#if hasDocument && annotationVisible}
      <ToolbarButton label="İşaretleme" hint="İşaretleme araç çubuğu" pressed={Boolean(ui.panels.markup)} onclick={toggleMarkup}><PenToolIcon /></ToolbarButton>
    {/if}
    {#if isPdf && featureUi("search").visible}
      <ToolbarButton label="Ara" hint={featureUi("search").warning || "Belgede ara (⌘F)"} pressed={ui.panels.search} disabled={!featureUi("search").runnable} onclick={() => onFeatureAction("search", () => ui.togglePanel("search"))}><SearchIcon /></ToolbarButton>
    {/if}
    {#if hasDocument}
      <ToolbarButton label="Dışa aktar" pressed={ui.isModalOpen("export")} onclick={() => ui.openModal("export")}><ShareIcon /></ToolbarButton>
    {/if}
    {#if hasDocument && !isPdf}
      <ToolbarButton label="Görseli dönüştür" onclick={onExportImage}><ImageDownIcon /></ToolbarButton>
    {/if}
    {#if !hasDocument}
      <ToolbarButton label="Aç" hint="Dosya aç (⌘O)" onclick={onSelectFile}><FolderOpenIcon /></ToolbarButton>
    {/if}

    {#if menuSections.length > 0}
      <DropdownMenu.Root>
        <DropdownMenu.Trigger
          class={cn(buttonVariants({ variant: "ghost", size: "icon-sm" }), "text-foreground/75 hover:text-foreground")}
          aria-label="Diğer"
        >
          <EllipsisIcon />
        </DropdownMenu.Trigger>
        <DropdownMenu.Content align="end" class="w-64">
          {#each menuSections as section, index (section.id)}
            {#if index > 0}<DropdownMenu.Separator />{/if}
            <DropdownMenu.Group>
              {#if section.label}<DropdownMenu.Label class="text-xs text-muted-foreground">{section.label}</DropdownMenu.Label>{/if}
              {#each section.items as entry (entry.id)}
                {#if "checked" in entry}
                  <DropdownMenu.CheckboxItem
                    bind:checked={() => entry.checked, () => {}}
                    closeOnSelect
                    disabled={entry.disabled}
                    title={entry.hint || undefined}
                    onSelect={entry.onSelect}
                  >
                    <span class="flex-1">{entry.label}</span>
                    {#if entry.badge}<span class="rounded-sm bg-muted px-1.5 text-[10px] text-muted-foreground">{entry.badge}</span>{/if}
                  </DropdownMenu.CheckboxItem>
                {:else}
                  <DropdownMenu.Item disabled={entry.disabled} title={entry.hint || undefined} onSelect={entry.onSelect}>
                    <span class="flex-1">{entry.label}</span>
                    {#if entry.badge}<span class="rounded-sm bg-muted px-1.5 text-[10px] text-muted-foreground">{entry.badge}</span>{/if}
                    {#if entry.shortcut}<DropdownMenu.Shortcut>{entry.shortcut}</DropdownMenu.Shortcut>{/if}
                  </DropdownMenu.Item>
                {/if}
              {/each}
            </DropdownMenu.Group>
          {/each}
        </DropdownMenu.Content>
      </DropdownMenu.Root>
    {/if}
  </header>
</Tooltip.Provider>

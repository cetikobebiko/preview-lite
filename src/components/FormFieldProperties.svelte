<script>
  import Trash2Icon from "@lucide/svelte/icons/trash-2";
  import { Input } from "$lib/components/ui/input";
  import { Checkbox } from "$lib/components/ui/checkbox";
  import * as Popover from "$lib/components/ui/popover";
  import { Button, buttonVariants } from "$lib/components/ui/button";
  import { Textarea } from "$lib/components/ui/textarea";
  import { cn } from "$lib/utils";

  // Seçili form alanının özellikleri (form-authoring-plan.md, Faz B). Araç çubuğunda
  // bağlamsal olarak görünür; metin girişleri odak kaybında ya da Enter'da, onay kutuları
  // hemen tek bir `set_props` işlemi olarak gönderilir. İpucu ve en fazla karakter okuma
  // modelinde olmadığından boş başlar; yalnız kullanıcı değiştirirse gönderilir. Ad girişi
  // tam adın son parçasını (/T) gösterir; yeniden adlandırma yalnız o parçayı değiştirir.
  let { design } = $props();

  const selection = $derived(design.selection);
  const widget = $derived(selection?.widget ?? null);
  const isText = $derived(widget?.kind === "Text");
  const isChoice = $derived(widget?.kind === "ComboBox" || widget?.kind === "ListBox");
  const isRadio = $derived(widget?.kind === "RadioButton");

  let name = $state("");
  let tooltip = $state("");
  let tooltipTouched = $state(false);
  let maxLen = $state("");
  let optionsText = $state("");
  let optionsOpen = $state(false);

  $effect(() => {
    const current = widget;
    name = current ? partialName(current.field_name) : "";
    tooltip = "";
    tooltipTouched = false;
    maxLen = "";
    optionsText = current?.options?.map((option) => option.label).join("\n") ?? "";
  });

  function partialName(fullName) {
    const dot = fullName.lastIndexOf(".");
    return dot >= 0 ? fullName.slice(dot + 1) : fullName;
  }

  function commitName() {
    const next = name.trim();
    if (!widget || !next || next === partialName(widget.field_name)) {
      name = widget ? partialName(widget.field_name) : "";
      return;
    }
    void design.updateSelected({ name: next });
  }

  function commitTooltip() {
    if (!widget || !tooltipTouched) return;
    tooltipTouched = false;
    void design.updateSelected({ tooltip });
  }

  function commitMaxLen() {
    if (!widget || maxLen === "") return;
    const value = Number(maxLen);
    if (!Number.isInteger(value) || value < 0) return;
    void design.updateSelected({ maxLen: value });
  }

  function commitOptions() {
    const options = optionsText
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
    if (!widget || options.length === 0) return;
    optionsOpen = false;
    void design.updateSelected({ options });
  }

  function onEnter(event, commit) {
    if (event.key === "Enter") {
      event.preventDefault();
      commit();
    }
  }
</script>

{#if widget}
  <div class="flex min-w-0 items-center gap-2 text-xs" role="group" aria-label="Form alanı özellikleri">
    <Input
      class="h-7 w-36 text-xs"
      aria-label="Alan adı"
      title={isRadio ? "Radio grubunun adı" : "Alan adı"}
      bind:value={name}
      disabled={design.busy}
      onblur={commitName}
      onkeydown={(event) => onEnter(event, commitName)}
    />
    <Input
      class="h-7 w-32 text-xs"
      aria-label="İpucu"
      placeholder="İpucu"
      bind:value={tooltip}
      disabled={design.busy}
      oninput={() => (tooltipTouched = true)}
      onblur={commitTooltip}
      onkeydown={(event) => onEnter(event, commitTooltip)}
    />
    <label class="flex items-center gap-1.5 whitespace-nowrap">
      <Checkbox
        checked={widget.required}
        disabled={design.busy}
        aria-label="Zorunlu"
        onCheckedChange={(checked) => void design.updateSelected({ required: checked === true })}
      />
      Zorunlu
    </label>
    <label class="flex items-center gap-1.5 whitespace-nowrap">
      <Checkbox
        checked={widget.read_only}
        disabled={design.busy}
        aria-label="Salt okunur"
        onCheckedChange={(checked) => void design.updateSelected({ readOnly: checked === true })}
      />
      Salt okunur
    </label>
    {#if isText}
      <label class="flex items-center gap-1.5 whitespace-nowrap">
        <Checkbox
          checked={widget.multiline}
          disabled={design.busy}
          aria-label="Çok satırlı"
          onCheckedChange={(checked) => void design.updateSelected({ multiline: checked === true })}
        />
        Çok satırlı
      </label>
      <Input
        class="h-7 w-24 text-xs"
        type="number"
        min="0"
        aria-label="En fazla karakter"
        placeholder="En fazla kr."
        title="En fazla karakter (0 sınırı kaldırır)"
        bind:value={maxLen}
        disabled={design.busy}
        onblur={commitMaxLen}
        onkeydown={(event) => onEnter(event, commitMaxLen)}
      />
    {/if}
    {#if isChoice}
      <Popover.Root bind:open={optionsOpen}>
        <Popover.Trigger
          class={cn(buttonVariants({ variant: "outline", size: "sm" }), "h-7 text-xs")}
          disabled={design.busy}
        >
          Seçenekler
        </Popover.Trigger>
        <Popover.Content align="start" class="w-64 space-y-2">
          <p class="text-xs text-muted-foreground">Her satıra bir seçenek.</p>
          <Textarea aria-label="Seçenekler" rows={6} bind:value={optionsText} />
          <Button size="sm" class="w-full" onclick={commitOptions}>Kaydet</Button>
        </Popover.Content>
      </Popover.Root>
    {/if}
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label="Alanı sil"
      title="Alanı sil (Delete)"
      disabled={design.busy}
      onclick={() => void design.deleteSelected()}
    >
      <Trash2Icon />
    </Button>
    {#if design.busy}
      <span class="text-muted-foreground">Kaydediliyor…</span>
    {:else if design.error}
      <span class="truncate text-destructive" title={design.error}>{design.error}</span>
    {/if}
  </div>
{:else if design.error}
  <span class="truncate text-xs text-destructive" title={design.error}>{design.error}</span>
{/if}

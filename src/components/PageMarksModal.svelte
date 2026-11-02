<script>
  import AppDialog from "./AppDialog.svelte";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { Button } from "$lib/components/ui/button";
  import { applyPageMarks } from "../pdfState.js";
  import { isSignatureLossDeclined } from "../services/signatureLossGuard.js";
  import { parsePageRanges } from "../utils/pageRanges.js";

  let { sessionId = "", contentGeneration = 0, pageCount = 0, pageOrder = [], editable = true, onsaved, onclose } = $props();

  const segmentClass = "rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs";
  const MODES = [
    { value: "header_footer", label: "Üstbilgi/Altbilgi" },
    { value: "watermark", label: "Filigran" },
    { value: "background", label: "Arka plan" },
    { value: "remove", label: "Kaldır" },
  ];
  const SLOTS = ["Sol üst", "Orta üst", "Sağ üst", "Sol alt", "Orta alt", "Sağ alt"];
  const TOKENS = [
    { label: "Sayfa 1 / n", token: "<<Sayfa 1 / n>>" },
    { label: "Sayfa no", token: "<<1>>" },
    { label: "Sayfa sayısı", token: "<<n>>" },
    { label: "Tarih", token: "<<dd.mm.yyyy>>" },
    { label: "Bates", token: "<<Bates Number#6#1##>>" },
  ];
  const KIND_LABELS = { header_footer: "Üstbilgi/altbilgi", watermark: "Filigran", background: "Arka plan" };

  let mode = $state("header_footer");
  let allPages = $state(true);
  let rangeText = $state("1");
  let saving = $state(false);
  let error = $state("");

  let texts = $state(["", "", "", "", "<<Sayfa 1 / n>>", ""]);
  let focusedSlot = $state(4);
  let headerFontSize = $state(9);
  let headerColor = $state("#000000");
  let margins = $state({ top: 36, bottom: 36, left: 72, right: 72 });
  let startNumber = $state(1);
  let replaceHeader = $state(true);

  let watermarkText = $state("GİZLİ");
  let watermarkFontSize = $state(0);
  let watermarkColor = $state("#cc0000");
  let watermarkOpacity = $state(30);
  let watermarkRotation = $state(45);
  let watermarkBehind = $state(false);
  let replaceWatermark = $state(true);

  let backgroundColor = $state("#fff8e1");
  let backgroundOpacity = $state(100);
  let replaceBackground = $state(true);

  let removeKinds = $state({ header_footer: true, watermark: false, background: false });

  const pages = $derived.by(() => {
    if (allPages) return { ok: true, pages: [] };
    const parsed = parsePageRanges(rangeText, pageCount, { mode: "pages" });
    if (!parsed.ok) return parsed;
    return { ok: true, pages: parsed.pages.map((display) => pageOrder[display] ?? display) };
  });

  function today() {
    const now = new Date();
    return { year: now.getFullYear(), month: now.getMonth() + 1, day: now.getDate() };
  }

  function insertToken(token) {
    texts[focusedSlot] = `${texts[focusedSlot]}${token}`;
  }

  const op = $derived.by(() => {
    if (mode === "header_footer") {
      return {
        op: "header_footer",
        spec: {
          texts: texts.map((text) => text),
          fontSize: Number(headerFontSize),
          color: headerColor,
          margins: [Number(margins.top), Number(margins.bottom), Number(margins.left), Number(margins.right)],
          startNumber: Number(startNumber),
          date: today(),
        },
        replace: replaceHeader,
      };
    }
    if (mode === "watermark") {
      return {
        op: "watermark",
        spec: {
          text: watermarkText,
          fontSize: Number(watermarkFontSize),
          color: watermarkColor,
          opacity: Number(watermarkOpacity) / 100,
          rotation: Number(watermarkRotation),
          offset: [0, 0],
          behind: watermarkBehind,
        },
        replace: replaceWatermark,
      };
    }
    if (mode === "background") {
      return {
        op: "background",
        spec: { color: backgroundColor, opacity: Number(backgroundOpacity) / 100 },
        replace: replaceBackground,
      };
    }
    return { op: "remove", kinds: Object.keys(removeKinds).filter((kind) => removeKinds[kind]) };
  });

  const formError = $derived.by(() => {
    if (!pages.ok) return pages.error;
    if (mode === "header_footer" && texts.every((text) => !text.trim())) return "En az bir kutuya metin girin.";
    if (mode === "header_footer" && !(Number(headerFontSize) >= 1 && Number(headerFontSize) <= 200)) return "Yazı boyutu 1–200 pt olmalı.";
    if (mode === "watermark" && !watermarkText.trim()) return "Filigran metnini girin.";
    if (mode === "watermark" && !(Number(watermarkOpacity) >= 1 && Number(watermarkOpacity) <= 100)) return "Opaklık %1–100 olmalı.";
    if (mode === "background" && !(Number(backgroundOpacity) >= 1 && Number(backgroundOpacity) <= 100)) return "Opaklık %1–100 olmalı.";
    if (mode === "remove" && op.kinds.length === 0) return "Kaldırılacak türü seçin.";
    return null;
  });
  const canApply = $derived(editable && !saving && !formError && Boolean(sessionId));

  async function apply() {
    if (!canApply) return;
    saving = true;
    error = "";
    try {
      const snapshot = await applyPageMarks(sessionId, contentGeneration, op, pages.pages);
      onsaved?.(snapshot, mode);
      onclose?.();
    } catch (e) {
      if (!isSignatureLossDeclined(e)) error = e?.message ?? String(e);
    } finally {
      saving = false;
    }
  }

  function close() {
    if (!saving) onclose?.();
  }
</script>

<AppDialog title="Sayfa İşaretleri" description="Üstbilgi, altbilgi, Bates numarası, filigran ve arka plan; belgeye artımlı olarak yazılır." size="lg" onclose={close}>
  <div class="grid gap-4">
    {#if !editable}
      <p class="rounded-md bg-warning px-3 py-2 text-xs text-warning-foreground">Belge salt okunur açıldı; sayfa işaretleri eklenemez.</p>
    {/if}
    <div class="grid grid-cols-4 gap-0.5 rounded-md bg-black/5 p-0.5 dark:bg-white/10" role="group" aria-label="İşaret türü">
      {#each MODES as item (item.value)}
        <button type="button" class={segmentClass} aria-pressed={mode === item.value} onclick={() => { mode = item.value; error = ""; }}>{item.label}</button>
      {/each}
    </div>

    {#if mode === "header_footer"}
      <div class="grid grid-cols-3 gap-2">
        {#each SLOTS as slot, index (slot)}
          <div class="grid gap-1">
            <Label for="mark-slot-{index}" class="text-[11px] text-muted-foreground">{slot}</Label>
            <Input id="mark-slot-{index}" type="text" maxlength="1024" bind:value={texts[index]} onfocus={() => { focusedSlot = index; }} disabled={saving} />
          </div>
        {/each}
      </div>
      <div class="flex flex-wrap items-center gap-1.5">
        <span class="text-[11px] text-muted-foreground">Ekle ({SLOTS[focusedSlot].toLowerCase()}):</span>
        {#each TOKENS as item (item.token)}
          <Button variant="outline" size="sm" onclick={() => insertToken(item.token)} disabled={saving}>{item.label}</Button>
        {/each}
      </div>
      <div class="grid grid-cols-4 gap-2">
        <div class="grid gap-1"><Label for="mark-hf-size" class="text-[11px] text-muted-foreground">Yazı boyutu (pt)</Label><Input id="mark-hf-size" type="number" min="1" max="200" bind:value={headerFontSize} /></div>
        <div class="grid gap-1"><Label for="mark-hf-color" class="text-[11px] text-muted-foreground">Renk</Label><Input id="mark-hf-color" type="color" bind:value={headerColor} /></div>
        <div class="grid gap-1"><Label for="mark-hf-start" class="text-[11px] text-muted-foreground">İlk sayfa no</Label><Input id="mark-hf-start" type="number" min="0" bind:value={startNumber} /></div>
        <label class="flex items-end gap-2 pb-2 text-xs"><input type="checkbox" bind:checked={replaceHeader} /> Var olanı değiştir</label>
        <div class="grid gap-1"><Label for="mark-m-top" class="text-[11px] text-muted-foreground">Üst boşluk</Label><Input id="mark-m-top" type="number" min="0" bind:value={margins.top} /></div>
        <div class="grid gap-1"><Label for="mark-m-bottom" class="text-[11px] text-muted-foreground">Alt boşluk</Label><Input id="mark-m-bottom" type="number" min="0" bind:value={margins.bottom} /></div>
        <div class="grid gap-1"><Label for="mark-m-left" class="text-[11px] text-muted-foreground">Sol boşluk</Label><Input id="mark-m-left" type="number" min="0" bind:value={margins.left} /></div>
        <div class="grid gap-1"><Label for="mark-m-right" class="text-[11px] text-muted-foreground">Sağ boşluk</Label><Input id="mark-m-right" type="number" min="0" bind:value={margins.right} /></div>
      </div>
    {:else if mode === "watermark"}
      <div class="grid gap-1">
        <Label for="mark-wm-text" class="text-[11px] text-muted-foreground">Metin</Label>
        <textarea id="mark-wm-text" rows="2" maxlength="1024" bind:value={watermarkText} class="rounded-md border border-input bg-background px-2.5 py-1.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50"></textarea>
      </div>
      <div class="grid grid-cols-4 gap-2">
        <div class="grid gap-1"><Label for="mark-wm-size" class="text-[11px] text-muted-foreground">Boyut (0 = sığdır)</Label><Input id="mark-wm-size" type="number" min="0" max="500" bind:value={watermarkFontSize} /></div>
        <div class="grid gap-1"><Label for="mark-wm-color" class="text-[11px] text-muted-foreground">Renk</Label><Input id="mark-wm-color" type="color" bind:value={watermarkColor} /></div>
        <div class="grid gap-1"><Label for="mark-wm-opacity" class="text-[11px] text-muted-foreground">Opaklık (%)</Label><Input id="mark-wm-opacity" type="number" min="1" max="100" bind:value={watermarkOpacity} /></div>
        <div class="grid gap-1"><Label for="mark-wm-rotation" class="text-[11px] text-muted-foreground">Açı (°)</Label><Input id="mark-wm-rotation" type="number" min="-180" max="180" bind:value={watermarkRotation} /></div>
      </div>
      <div class="flex gap-4 text-xs">
        <label class="flex items-center gap-2"><input type="checkbox" bind:checked={watermarkBehind} /> İçeriğin arkasına yerleştir</label>
        <label class="flex items-center gap-2"><input type="checkbox" bind:checked={replaceWatermark} /> Var olanı değiştir</label>
      </div>
    {:else if mode === "background"}
      <div class="grid grid-cols-3 gap-2">
        <div class="grid gap-1"><Label for="mark-bg-color" class="text-[11px] text-muted-foreground">Renk</Label><Input id="mark-bg-color" type="color" bind:value={backgroundColor} /></div>
        <div class="grid gap-1"><Label for="mark-bg-opacity" class="text-[11px] text-muted-foreground">Opaklık (%)</Label><Input id="mark-bg-opacity" type="number" min="1" max="100" bind:value={backgroundOpacity} /></div>
        <label class="flex items-end gap-2 pb-2 text-xs"><input type="checkbox" bind:checked={replaceBackground} /> Var olanı değiştir</label>
      </div>
    {:else}
      <p class="text-xs text-muted-foreground">Yalnız bu uygulamanın eklediği işaretler kaldırılır.</p>
      <div class="flex gap-4 text-xs">
        {#each Object.keys(removeKinds) as kind (kind)}
          <label class="flex items-center gap-2"><input type="checkbox" bind:checked={removeKinds[kind]} /> {KIND_LABELS[kind]}</label>
        {/each}
      </div>
    {/if}

    <div class="grid gap-1.5">
      <span class="text-[11px] text-muted-foreground">Sayfalar</span>
      <div class="flex items-center gap-3 text-xs">
        <label class="flex items-center gap-2"><input type="radio" name="mark-pages" checked={allPages} onchange={() => { allPages = true; }} /> Tümü ({pageCount})</label>
        <label class="flex items-center gap-2"><input type="radio" name="mark-pages" checked={!allPages} onchange={() => { allPages = false; }} /> Aralık</label>
        <Input class="h-8 max-w-48" type="text" bind:value={rangeText} placeholder="1-3, 5" disabled={allPages} aria-label="Sayfa aralığı" />
      </div>
    </div>

    {#if formError}
      <p class="text-xs text-destructive">{formError}</p>
    {/if}
    {#if error}
      <p class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{error}</p>
    {/if}
  </div>
  {#snippet footer()}
    <Button variant="outline" size="sm" onclick={close} disabled={saving}>Kapat</Button>
    <Button size="sm" onclick={apply} disabled={!canApply}>{saving ? "Uygulanıyor…" : mode === "remove" ? "Kaldır" : "Uygula"}</Button>
  {/snippet}
</AppDialog>

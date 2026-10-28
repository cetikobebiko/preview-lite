<script>
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";
  import { setOutline } from "../pdfState.js";
  import { getSessionPdfOutline } from "../services/backend/session.js";
  import { isSignatureLossDeclined } from "../services/signatureLossGuard.js";
  import {
    findEmptyOutlineTitle,
    getOutlineNode,
    indentOutlineNode,
    insertOutlineNode,
    moveOutlineNode,
    newOutlineNode,
    outdentOutlineNode,
    outlineEditTreeToWire,
    outlineToEditTree,
    removeOutlineNode,
    updateOutlineNode,
  } from "../services/documentStructure.js";

  let {
    sessionId = "",
    contentGeneration = 0,
    currentPageIndex = 0,
    pageCount = 0,
    editable = true,
    onsaved,
    onclose,
  } = $props();

  let tree = $state([]);
  let loadedWire = $state("[]");
  let selected = $state(null);
  let loading = $state(true);
  let saving = $state(false);
  let error = $state("");

  $effect(() => {
    const requestSession = sessionId;
    const requestGeneration = contentGeneration;
    if (!requestSession) return;
    let cancelled = false;
    loading = true;
    error = "";
    getSessionPdfOutline(requestSession, requestGeneration)
      .then((outline) => {
        if (cancelled) return;
        tree = outlineToEditTree(Array.isArray(outline) ? outline : []);
        loadedWire = JSON.stringify(outlineEditTreeToWire(tree));
        selected = null;
      })
      .catch((e) => { if (!cancelled) error = `Yer imleri okunamadı: ${e?.message ?? e}`; })
      .finally(() => { if (!cancelled) loading = false; });
    return () => { cancelled = true; };
  });

  function flatten(nodes, path = [], depth = 0, out = []) {
    nodes.forEach((node, index) => {
      const nodePath = [...path, index];
      out.push({ node, path: nodePath, depth });
      flatten(node.children, nodePath, depth + 1, out);
    });
    return out;
  }

  const rows = $derived(flatten(tree));
  const selectedNode = $derived(selected ? getOutlineNode(tree, selected) : null);
  const changed = $derived(JSON.stringify(outlineEditTreeToWire(tree)) !== loadedWire);
  const emptyTitle = $derived(findEmptyOutlineTitle(tree));
  const disabled = $derived(!editable || saving || loading);
  const canSave = $derived(!disabled && changed && !emptyTitle);
  const samePath = (a, b) => a && b && a.length === b.length && a.every((value, index) => value === b[index]);

  function add(asChild) {
    const node = newOutlineNode(`Sayfa ${currentPageIndex + 1}`, currentPageIndex);
    if (!selected) {
      tree = insertOutlineNode(tree, [], tree.length, node);
      selected = [tree.length - 1];
      return;
    }
    if (asChild) {
      const count = getOutlineNode(tree, selected).children.length;
      tree = insertOutlineNode(tree, selected, count, node);
      selected = [...selected, count];
      return;
    }
    const parent = selected.slice(0, -1);
    const index = selected[selected.length - 1] + 1;
    tree = insertOutlineNode(tree, parent, index, node);
    selected = [...parent, index];
  }

  function remove() {
    if (!selected) return;
    tree = removeOutlineNode(tree, selected);
    selected = null;
  }

  function apply(result) {
    tree = result.tree;
    selected = result.path;
  }

  function rename(path, title) {
    tree = updateOutlineNode(tree, path, (node) => ({ ...node, title }));
  }

  function targetCurrentPage() {
    if (!selected) return;
    tree = updateOutlineNode(tree, selected, (node) => ({ ...node, pageIndex: currentPageIndex, displayPage: currentPageIndex }));
  }

  async function save() {
    if (!canSave) return;
    saving = true;
    error = "";
    try {
      const snapshot = await setOutline(sessionId, contentGeneration, outlineEditTreeToWire(tree));
      onsaved?.(snapshot);
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

<AppDialog title="Yer İmlerini Düzenle" description="Yer imlerini ekleyin, yeniden adlandırın ve taşıyın; yeni yer imleri geçerli sayfaya gider." size="lg" onclose={close}>
  <div class="grid gap-3">
    {#if !editable}
      <p class="rounded-md bg-warning px-3 py-2 text-xs text-warning-foreground">Belge salt okunur açıldı; yer imleri değiştirilemez.</p>
    {/if}
    <div class="flex flex-wrap gap-1.5" role="toolbar" aria-label="Yer imi işlemleri">
      <Button variant="outline" size="sm" onclick={() => add(false)} {disabled}>Ekle</Button>
      <Button variant="outline" size="sm" onclick={() => add(true)} disabled={disabled || !selected}>Alt yer imi</Button>
      <Button variant="outline" size="sm" onclick={() => apply(moveOutlineNode(tree, selected, -1))} disabled={disabled || !selected} aria-label="Yukarı taşı">↑</Button>
      <Button variant="outline" size="sm" onclick={() => apply(moveOutlineNode(tree, selected, 1))} disabled={disabled || !selected} aria-label="Aşağı taşı">↓</Button>
      <Button variant="outline" size="sm" onclick={() => apply(outdentOutlineNode(tree, selected))} disabled={disabled || !selected || selected.length < 2} aria-label="Bir düzey dışarı">←</Button>
      <Button variant="outline" size="sm" onclick={() => apply(indentOutlineNode(tree, selected))} disabled={disabled || !selected || selected[selected.length - 1] === 0} aria-label="Bir düzey içeri">→</Button>
      <Button variant="outline" size="sm" onclick={targetCurrentPage} disabled={disabled || !selected}>Hedef: sayfa {currentPageIndex + 1}</Button>
      <Button variant="outline" size="sm" onclick={remove} disabled={disabled || !selected}>Sil</Button>
    </div>
    {#if loading}
      <p class="text-xs text-muted-foreground">Yükleniyor…</p>
    {:else if rows.length === 0}
      <p class="text-xs text-muted-foreground">Belgede yer imi yok. "Ekle" geçerli sayfaya bir yer imi ekler.</p>
    {:else}
      <ul class="max-h-[50vh] overflow-y-auto rounded-md border" aria-label="Yer imleri">
        {#each rows as row (row.node.key)}
          <li class="flex items-center gap-2 border-b px-2 py-1 last:border-b-0 {samePath(row.path, selected) ? 'bg-accent' : ''}" style="padding-left: {0.5 + row.depth * 1.25}rem">
            <input
              class="h-7 min-w-0 flex-1 rounded border border-transparent bg-transparent px-1.5 text-sm outline-none focus:border-input focus:bg-background"
              value={row.node.title}
              aria-label="Yer imi başlığı"
              maxlength="1024"
              disabled={!editable || saving}
              onfocus={() => { selected = row.path; }}
              oninput={(event) => rename(row.path, event.currentTarget.value)}
            />
            <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">
              {row.node.pageIndex !== null ? `→ s. ${row.node.pageIndex + 1}` : `s. ${row.node.displayPage + 1}`}
            </span>
          </li>
        {/each}
      </ul>
    {/if}
    {#if emptyTitle}
      <p class="text-xs text-destructive">Yer imi başlığı boş olamaz.</p>
    {/if}
    {#if selectedNode && pageCount > 0 && selectedNode.pageIndex !== null && selectedNode.pageIndex >= pageCount}
      <p class="text-xs text-destructive">Hedef sayfa belgede yok.</p>
    {/if}
    {#if error}
      <p class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{error}</p>
    {/if}
  </div>
  {#snippet footer()}
    <Button variant="outline" size="sm" onclick={close} disabled={saving}>Kapat</Button>
    <Button size="sm" onclick={save} disabled={!canSave}>{saving ? "Kaydediliyor…" : "Kaydet"}</Button>
  {/snippet}
</AppDialog>

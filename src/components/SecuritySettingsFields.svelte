<script>
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { ENCRYPTION_METHODS, PRINT_PERMISSIONS, passwordStrength } from "../services/documentSecurity.js";

  // Shared by the single-document security dialog and the batch wizard; `fields`
  // comes from `emptySecurityFields()`.
  let {
    fields = $bindable(),
    disabled = false,
    idPrefix = "security",
    removeNote = "",
  } = $props();

  const selectClass = "h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50";
  const toggleClass = "rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs";
  const PERMISSION_LABELS = Object.freeze({
    modify: "Sayfa içeriğini değiştirme",
    copy: "Metin ve görselleri kopyalama",
    annotate: "Açıklama ekleme ve form doldurma",
    fillForms: "Form doldurma ve imzalama",
    assemble: "Sayfa ekleme, silme, döndürme",
  });

  $effect(() => {
    if (fields.method === "rc4_128") fields.encryptMetadata = true;
  });
</script>

<div class="grid grid-cols-2 gap-0.5 rounded-md bg-black/5 p-0.5 dark:bg-white/10" role="group" aria-label="Güvenlik işlemi">
  <button type="button" class={toggleClass} aria-pressed={fields.mode === "protect"} onclick={() => (fields.mode = "protect")} {disabled}>Parola ile koru</button>
  <button type="button" class={toggleClass} aria-pressed={fields.mode === "remove"} onclick={() => (fields.mode = "remove")} {disabled}>Güvenliği kaldır</button>
</div>

{#if fields.mode === "protect"}
  <div class="grid grid-cols-2 gap-2">
    <div class="grid gap-1">
      <Label for="{idPrefix}-user" class="text-[11px] text-muted-foreground">Açma parolası (isteğe bağlı)</Label>
      <Input id="{idPrefix}-user" type="password" autocomplete="new-password" bind:value={fields.userPassword} {disabled} />
      {#if fields.userPassword}<small class="text-[11px] text-muted-foreground">Güç: {passwordStrength(fields.userPassword)}</small>{/if}
    </div>
    <div class="grid gap-1">
      <Label for="{idPrefix}-user-confirm" class="text-[11px] text-muted-foreground">Açma parolası (tekrar)</Label>
      <Input id="{idPrefix}-user-confirm" type="password" autocomplete="new-password" bind:value={fields.userConfirm} {disabled} />
    </div>
    <div class="grid gap-1">
      <Label for="{idPrefix}-owner" class="text-[11px] text-muted-foreground">İzin parolası</Label>
      <Input id="{idPrefix}-owner" type="password" autocomplete="new-password" bind:value={fields.ownerPassword} {disabled} />
      {#if fields.ownerPassword}<small class="text-[11px] text-muted-foreground">Güç: {passwordStrength(fields.ownerPassword)}</small>{/if}
    </div>
    <div class="grid gap-1">
      <Label for="{idPrefix}-owner-confirm" class="text-[11px] text-muted-foreground">İzin parolası (tekrar)</Label>
      <Input id="{idPrefix}-owner-confirm" type="password" autocomplete="new-password" bind:value={fields.ownerConfirm} {disabled} />
    </div>
  </div>

  <div class="grid grid-cols-2 gap-2">
    <div class="grid gap-1">
      <Label for="{idPrefix}-method" class="text-[11px] text-muted-foreground">Şifreleme</Label>
      <select id="{idPrefix}-method" bind:value={fields.method} class={selectClass} {disabled}>
        {#each ENCRYPTION_METHODS as option (option.value)}
          <option value={option.value}>{option.label}</option>
        {/each}
      </select>
    </div>
    <div class="grid gap-1">
      <Label for="{idPrefix}-print" class="text-[11px] text-muted-foreground">Yazdırma</Label>
      <select id="{idPrefix}-print" bind:value={fields.permissions.print} class={selectClass} {disabled}>
        {#each PRINT_PERMISSIONS as option (option.value)}
          <option value={option.value}>{option.label}</option>
        {/each}
      </select>
    </div>
  </div>

  <fieldset class="grid grid-cols-2 gap-1.5 text-xs" {disabled}>
    <legend class="mb-1 text-[11px] text-muted-foreground">Parolasız açan kişiye izin verilenler</legend>
    {#each Object.entries(PERMISSION_LABELS) as [key, label] (key)}
      <label class="flex items-center gap-2"><input type="checkbox" bind:checked={fields.permissions[key]} /> {label}</label>
    {/each}
    <label class="col-span-2 flex items-center gap-2"><input type="checkbox" bind:checked={fields.encryptMetadata} disabled={fields.method === "rc4_128"} /> Meta veriyi de şifrele (kapalıysa arama motorları okuyabilir)</label>
  </fieldset>
{:else if removeNote}
  <p class="text-xs text-muted-foreground">{removeNote}</p>
{/if}

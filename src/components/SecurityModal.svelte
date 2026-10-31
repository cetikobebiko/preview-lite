<script>
  import AppDialog from "./AppDialog.svelte";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { Button } from "$lib/components/ui/button";
  import { save } from "@tauri-apps/plugin-dialog";
  import { saveSessionSecurityCopy } from "../services/backend/session.js";
  import {
    ALL_PERMISSIONS,
    ENCRYPTION_METHODS,
    PRINT_PERMISSIONS,
    passwordStrength,
    protectionSettingsError,
  } from "../services/documentSecurity.js";

  let { sessionId = "", contentGeneration = 0, fileName = "", onsaved, onclose } = $props();

  const selectClass = "h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50";
  const PERMISSION_LABELS = Object.freeze({
    modify: "Sayfa içeriğini değiştirme",
    copy: "Metin ve görselleri kopyalama",
    annotate: "Açıklama ekleme ve form doldurma",
    fillForms: "Form doldurma ve imzalama",
    assemble: "Sayfa ekleme, silme, döndürme",
  });

  let mode = $state("protect"); // "protect" or "remove"
  let userPassword = $state("");
  let userConfirm = $state("");
  let ownerPassword = $state("");
  let ownerConfirm = $state("");
  let method = $state("aes256");
  let permissions = $state({ ...ALL_PERMISSIONS });
  let encryptMetadata = $state(true);
  let saving = $state(false);
  let error = $state("");

  const settings = $derived({ userPassword, ownerPassword, method, permissions: { ...permissions }, encryptMetadata });
  const mismatch = $derived(
    userPassword !== userConfirm ? "Açma parolaları uyuşmuyor."
      : ownerPassword !== ownerConfirm ? "İzin parolaları uyuşmuyor." : null,
  );
  const validationError = $derived(mode === "protect" ? (protectionSettingsError(settings) ?? mismatch) : null);
  const canSave = $derived(Boolean(sessionId) && !saving && !validationError);

  $effect(() => {
    if (method === "rc4_128") encryptMetadata = true;
  });

  function defaultName() {
    const stem = (fileName || "belge").replace(/\.pdf$/i, "");
    return `${stem}${mode === "protect" ? "_korumali" : "_korumasiz"}.pdf`;
  }

  async function run() {
    error = "";
    const target = await save({
      title: mode === "protect" ? "Korunan kopyayı kaydet" : "Korumasız kopyayı kaydet",
      defaultPath: defaultName(),
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!target) return;
    saving = true;
    try {
      const op = mode === "protect" ? { op: "protect", settings: $state.snapshot(settings) } : { op: "remove" };
      await saveSessionSecurityCopy(sessionId, contentGeneration, op, target);
      onsaved?.(mode === "protect" ? "Parola ile korunan kopya kaydedildi." : "Korumasız kopya kaydedildi.");
      onclose?.();
    } catch (e) {
      error = e?.message ?? String(e);
    } finally {
      saving = false;
    }
  }

  function close() {
    if (!saving) onclose?.();
  }
</script>

<AppDialog title="Belge Güvenliği" description="Belgenin parola ile korunan ya da korumasız bir kopyasını kaydedin. Açık belge değişmez." size="lg" onclose={close}>
  <div class="grid gap-3">
    <div class="grid grid-cols-2 gap-0.5 rounded-md bg-black/5 p-0.5 dark:bg-white/10" role="group" aria-label="Güvenlik işlemi">
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={mode === "protect"} onclick={() => (mode = "protect")}>Parola ile koru</button>
      <button type="button" class="rounded-[5px] py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs" aria-pressed={mode === "remove"} onclick={() => (mode = "remove")}>Güvenliği kaldır</button>
    </div>

    {#if mode === "protect"}
      <div class="grid grid-cols-2 gap-2">
        <div class="grid gap-1">
          <Label for="security-user" class="text-[11px] text-muted-foreground">Açma parolası (isteğe bağlı)</Label>
          <Input id="security-user" type="password" autocomplete="new-password" bind:value={userPassword} disabled={saving} />
          {#if userPassword}<small class="text-[11px] text-muted-foreground">Güç: {passwordStrength(userPassword)}</small>{/if}
        </div>
        <div class="grid gap-1">
          <Label for="security-user-confirm" class="text-[11px] text-muted-foreground">Açma parolası (tekrar)</Label>
          <Input id="security-user-confirm" type="password" autocomplete="new-password" bind:value={userConfirm} disabled={saving} />
        </div>
        <div class="grid gap-1">
          <Label for="security-owner" class="text-[11px] text-muted-foreground">İzin parolası</Label>
          <Input id="security-owner" type="password" autocomplete="new-password" bind:value={ownerPassword} disabled={saving} />
          {#if ownerPassword}<small class="text-[11px] text-muted-foreground">Güç: {passwordStrength(ownerPassword)}</small>{/if}
        </div>
        <div class="grid gap-1">
          <Label for="security-owner-confirm" class="text-[11px] text-muted-foreground">İzin parolası (tekrar)</Label>
          <Input id="security-owner-confirm" type="password" autocomplete="new-password" bind:value={ownerConfirm} disabled={saving} />
        </div>
      </div>

      <div class="grid grid-cols-2 gap-2">
        <div class="grid gap-1">
          <Label for="security-method" class="text-[11px] text-muted-foreground">Şifreleme</Label>
          <select id="security-method" bind:value={method} class={selectClass} disabled={saving}>
            {#each ENCRYPTION_METHODS as option (option.value)}
              <option value={option.value}>{option.label}</option>
            {/each}
          </select>
        </div>
        <div class="grid gap-1">
          <Label for="security-print" class="text-[11px] text-muted-foreground">Yazdırma</Label>
          <select id="security-print" bind:value={permissions.print} class={selectClass} disabled={saving}>
            {#each PRINT_PERMISSIONS as option (option.value)}
              <option value={option.value}>{option.label}</option>
            {/each}
          </select>
        </div>
      </div>

      <fieldset class="grid grid-cols-2 gap-1.5 text-xs" disabled={saving}>
        <legend class="mb-1 text-[11px] text-muted-foreground">Parolasız açan kişiye izin verilenler</legend>
        {#each Object.entries(PERMISSION_LABELS) as [key, label] (key)}
          <label class="flex items-center gap-2"><input type="checkbox" bind:checked={permissions[key]} /> {label}</label>
        {/each}
        <label class="col-span-2 flex items-center gap-2"><input type="checkbox" bind:checked={encryptMetadata} disabled={method === "rc4_128"} /> Meta veriyi de şifrele (kapalıysa arama motorları okuyabilir)</label>
      </fieldset>

      <p class="text-[11px] text-muted-foreground">Kopya yeniden yazılır; belgede dijital imza varsa kopyada geçersiz olur. Parolayı unutursanız kopya açılamaz.</p>
    {:else}
      <p class="text-xs text-muted-foreground">Belgenin şifresiz bir kopyası kaydedilir. Kısıtlı izinlerle açılmış bir belgenin güvenliği yalnız izin (sahip) parolasıyla açıldıysa kaldırılabilir.</p>
    {/if}

    {#if validationError && (userPassword || ownerPassword)}
      <p class="text-xs text-destructive">{validationError}</p>
    {/if}
    {#if error}
      <p class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{error}</p>
    {/if}
  </div>
  {#snippet footer()}
    <Button variant="outline" size="sm" onclick={close} disabled={saving}>Kapat</Button>
    <Button size="sm" onclick={run} disabled={!canSave}>{saving ? "Kaydediliyor…" : "Kopyayı kaydet…"}</Button>
  {/snippet}
</AppDialog>

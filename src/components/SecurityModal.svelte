<script>
  import AppDialog from "./AppDialog.svelte";
  import SecuritySettingsFields from "./SecuritySettingsFields.svelte";
  import { Button } from "$lib/components/ui/button";
  import { save } from "@tauri-apps/plugin-dialog";
  import { saveSessionSecurityCopy } from "../services/backend/session.js";
  import { emptySecurityFields, securityFieldsError, securityOpFromFields } from "../services/documentSecurity.js";
  import { copySignatureNote, readSignatureCount, withSignatureLoss } from "../services/copySignatures.js";

  let { sessionId = "", contentGeneration = 0, fileName = "", onsaved, onclose } = $props();

  let fields = $state(emptySecurityFields());
  let saving = $state(false);
  let error = $state("");
  let signatureCount = $state(0);

  const validationError = $derived(securityFieldsError(fields));
  const canSave = $derived(Boolean(sessionId) && !saving && !validationError);

  // Ön bilgi; okunamazsa not gösterilmez, backend raporu kayıttan sonra yine söyler.
  $effect(() => {
    const requestSession = sessionId;
    const requestGeneration = contentGeneration;
    signatureCount = 0;
    let disposed = false;
    readSignatureCount(requestSession, requestGeneration).then((count) => {
      if (!disposed) signatureCount = count;
    });
    return () => {
      disposed = true;
    };
  });

  function defaultName() {
    const stem = (fileName || "belge").replace(/\.pdf$/i, "");
    return `${stem}${fields.mode === "protect" ? "_korumali" : "_korumasiz"}.pdf`;
  }

  async function run() {
    error = "";
    const protect = fields.mode === "protect";
    const target = await save({
      title: protect ? "Korunan kopyayı kaydet" : "Korumasız kopyayı kaydet",
      defaultPath: defaultName(),
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!target) return;
    saving = true;
    try {
      const report = await saveSessionSecurityCopy(sessionId, contentGeneration, securityOpFromFields($state.snapshot(fields)), target);
      onsaved?.(withSignatureLoss(protect ? "Parola ile korunan kopya kaydedildi." : "Korumasız kopya kaydedildi.", report, "security"));
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
    <SecuritySettingsFields
      bind:fields
      disabled={saving}
      removeNote="Belgenin şifresiz bir kopyası kaydedilir. Kısıtlı izinlerle açılmış bir belgenin güvenliği yalnız izin (sahip) parolasıyla açıldıysa kaldırılabilir."
    />
    {#if signatureCount > 0}
      <p class="rounded-md bg-amber-500/10 px-3 py-2 text-xs" role="note">{copySignatureNote("security")}</p>
    {/if}
    {#if fields.mode === "protect"}
      <p class="text-[11px] text-muted-foreground">Parolayı unutursanız kopya açılamaz.</p>
    {/if}

    {#if validationError && (fields.userPassword || fields.ownerPassword)}
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

<script>
  // Offline signature verification panel (Prompt 11 G2, pades-architecture-design.md §19).
  // Every dimension is shown on its own; there is deliberately no single "valid" mark:
  // the signer's trustworthiness is never evaluated in this profile.
  import { onDestroy, untrack } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";
  import { startSignatureInspectionAction } from "../services/actions.js";
  import { createSignatureVerificationState } from "../stores/signatureVerificationState.svelte.js";
  import { createSignatureVerificationController } from "../services/app/signatureVerificationController.js";

  let { sessionId = "", contentGeneration = 0, isPdf = true } = $props();

  const featureContext = getRuntimeFeatureContext();
  const presentation = getFeaturePresentation("signature_verification", featureContext);
  const verification = createSignatureVerificationState();
  const controller = createSignatureVerificationController({
    state: verification,
    startInspection: (session, generation) =>
      startSignatureInspectionAction(session, generation, featureContext),
    getContext: () => ({ sessionId, contentGeneration, isPdf }),
  });

  $effect(() => {
    sessionId;
    contentGeneration;
    isPdf;
    untrack(() => controller.reconcileContext());
  });

  onDestroy(() => controller.destroy());

  const report = $derived(verification.report);
  const stale = $derived(verification.isStaleFor(sessionId, contentGeneration));

  const INTEGRITY = {
    valid: "İmza kriptografik olarak bütün; imzalayanın güvenilirliği değerlendirilmedi",
    invalid: "Bütün değil: imzalı baytlar ya da imza değeri tutmuyor",
    indeterminate: "Belirlenemedi",
    unsupported_algorithm: "Desteklenmeyen ya da yasaklı algoritma; doğrulanmadı",
    malformed: "İmza verisi bozuk; doğrulanamadı",
  };
  const BYTE_RANGE = {
    valid: "Kurallara uygun",
    invalid: "Geçersiz",
    indeterminate: "Belirlenemedi",
  };
  const MODIFICATION = {
    no_later_revision: "İmzadan sonra değişiklik yok",
    later_revision_permitted: "Sonraki revizyonlar izinli",
    later_revision_not_permitted: "Sonraki revizyonlar izinsiz",
    later_revision_unclassified: "İmzadan sonra revizyon var (izin durumu sınıflandırılmadı)",
    byte_range_invalid: "ByteRange geçersiz olduğu için değerlendirilemedi",
  };
  const DOCUMENT_MODIFICATION = {
    no_modifications: "Son imzadan sonra değişiklik yok",
    permitted_modifications_only: "Yalnız izinli değişiklikler",
    unauthorized_modifications_detected: "İzinsiz değişiklik var",
    indeterminate: "Belirlenemedi",
  };
  const PADES = {
    none: "PAdES işareti yok",
    pades_b_b: "B-B (yapısal işaretlere göre)",
    pades_b_t: "B-T (yapısal işaretlere göre)",
    pades_b_l_t: "B-LT (yapısal işaretlere göre)",
    pades_b_l_t_a: "B-LTA (yapısal işaretlere göre)",
  };
  const PERMISSION = {
    no_changes_permitted: "P=1: değişikliğe izin yok",
    form_fill_and_signing_permitted: "P=2: form doldurma ve imzalama",
    form_fill_signing_and_annotations: "P=3: form, imzalama ve açıklamalar",
  };
  const POLICY = {
    allowed: "izinli",
    deprecated: "eskimiş",
    disallowed: "yasaklı",
    indeterminate: "belirsiz",
    not_evaluated: "değerlendirilmedi",
  };
  const CERTIFICATE = {
    missing: "İmzalayan sertifikası CMS içinde yok",
    ambiguous: "Birden çok aday sertifika var",
    not_evaluated: "Değerlendirilemedi",
  };

  const negative = (value) => value === "invalid" || value === "malformed";

  function signerText(signature) {
    if (!signature.signer) return CERTIFICATE[signature.signerCertificate] ?? "—";
    const { commonName, organization, country } = signature.signer;
    return [commonName, organization, country].filter(Boolean).join(" · ") || signature.signer.issuer;
  }
</script>

<div class="flex flex-col gap-3 p-3 text-sm" data-testid="signature-verification-panel">
  <FeatureNotice featureId="signature_verification" compact />

  <p class="text-xs text-muted-foreground">
    İmzaların yapısı ve kriptografik bütünlüğü çevrimdışı denetlenir. Sertifika zinciri, güven ve
    iptal durumu değerlendirilmez.
  </p>

  <div class="flex gap-2">
    <Button
      size="sm"
      onclick={() => controller.run()}
      disabled={verification.isRunning || !presentation.runnable || !isPdf || !sessionId}
    >
      {verification.isRunning ? "Denetleniyor…" : report ? "Yeniden denetle" : "İmzaları denetle"}
    </Button>
    {#if verification.isRunning}
      <Button size="sm" variant="outline" onclick={() => controller.cancel()}>İptal</Button>
    {/if}
  </div>

  {#if verification.error}
    <p class="text-xs text-destructive" role="alert">{verification.error}</p>
  {/if}

  {#if report}
    {#if stale}
      <p class="rounded-md border px-2 py-1 text-xs" role="status" data-testid="signature-report-stale">
        Belge bu rapordan sonra değişti; rapor eski. Yeniden denetleyin.
      </p>
    {/if}

    <section class="flex flex-col gap-1" aria-label="Belge özeti">
      <p>
        {report.totalSignatures} imza · {report.revisionCount} revizyon
        {#if report.emptyFieldCount > 0}· {report.emptyFieldCount} boş imza alanı{/if}
      </p>
      <p class="text-xs text-muted-foreground">
        Belge değişikliği: {DOCUMENT_MODIFICATION[report.documentModification]}
      </p>
      {#if report.certificationPermission}
        <p class="text-xs text-muted-foreground">Sertifika izni: {PERMISSION[report.certificationPermission]}</p>
      {/if}
      {#if report.inspectionStatus !== "complete"}
        <p class="text-xs text-destructive">
          {report.inspectionStatus === "indeterminate"
            ? "Belge açılamadı; imzalar denetlenmedi."
            : "Denetim kısmi: bazı yapısal kaynaklar okunamadı."}
        </p>
      {/if}
      {#each report.findings as finding (finding.code + (finding.context ?? ""))}
        <p class="text-xs text-muted-foreground">• {finding.message}</p>
      {/each}
    </section>

    {#each report.signatures as signature (signature.id)}
      <article class="flex flex-col gap-1 rounded-md border p-2" data-testid="signature-card">
        <header class="flex items-baseline justify-between gap-2">
          <h3 class="truncate font-medium">{signature.fieldName ?? "(adsız alan)"}</h3>
          <span class="shrink-0 text-xs text-muted-foreground">
            {signature.kind === "document_timestamp" ? "Belge zaman damgası" : `Revizyon ${(signature.revisionIndex ?? -1) + 1}`}
          </span>
        </header>
        <dl class="grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 text-xs">
          <dt class="text-muted-foreground">Kriptografik bütünlük</dt>
          <dd class:text-destructive={negative(signature.integrity)} data-testid="signature-integrity">
            {INTEGRITY[signature.integrity]}
          </dd>
          <dt class="text-muted-foreground">Bayt aralığı</dt>
          <dd class:text-destructive={negative(signature.byteRangeStatus)}>{BYTE_RANGE[signature.byteRangeStatus]}</dd>
          <dt class="text-muted-foreground">Sonraki değişiklikler</dt>
          <dd>{MODIFICATION[signature.modification]}</dd>
          <dt class="text-muted-foreground">Algoritma</dt>
          <dd>
            {signature.digestAlgorithm ?? "?"} ({POLICY[signature.digestPolicy] ?? "?"}) ·
            {signature.signatureAlgorithm ?? "?"} ({POLICY[signature.signaturePolicy] ?? "?"})
          </dd>
          <dt class="text-muted-foreground">İmzalayan (doğrulanmadı)</dt>
          <dd class="break-words">{signerText(signature)}</dd>
          <dt class="text-muted-foreground">Güven</dt>
          <dd>Değerlendirilmedi</dd>
          <dt class="text-muted-foreground">İptal durumu</dt>
          <dd>Değerlendirilmedi</dd>
          <dt class="text-muted-foreground">PAdES (algılanan)</dt>
          <dd>{PADES[signature.padesDetected]}</dd>
          {#if signature.timestamp}
            <dt class="text-muted-foreground">Zaman damgası</dt>
            <dd>Var, doğrulanmadı</dd>
          {/if}
          {#if signature.claimedTime.cms || signature.claimedTime.pdf}
            <dt class="text-muted-foreground">Beyan edilen zaman</dt>
            <dd>{signature.claimedTime.cms ?? signature.claimedTime.pdf} (imzalayanın iddiası)</dd>
          {/if}
          {#if signature.docmdp?.permission}
            <dt class="text-muted-foreground">DocMDP</dt>
            <dd>{PERMISSION[signature.docmdp.permission]}</dd>
          {/if}
          {#if signature.reason}
            <dt class="text-muted-foreground">Neden</dt>
            <dd class="break-words">{signature.reason}</dd>
          {/if}
        </dl>
        {#if signature.findings.length > 0}
          <ul class="mt-1 flex flex-col gap-0.5 text-xs text-muted-foreground">
            {#each signature.findings as finding, index (index)}
              <li>• {finding.message}</li>
            {/each}
          </ul>
        {/if}
      </article>
    {/each}
  {/if}
</div>

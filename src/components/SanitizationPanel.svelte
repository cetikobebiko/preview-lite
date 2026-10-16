<script>
  import AppDialog from "./AppDialog.svelte";
  import {
    SANITIZATION_MINIMUM_PROFILE_CONTROL_IDS,
    SANITIZATION_SUPPORTED_CONTROL_IDS,
  } from "../features/sanitization/sanitizationSchemas.js";

  let {
    show = false,
    state,
    runnable = false,
    onInspect = () => {},
    onPolicyChange = () => {},
    onAcknowledgementChange = () => {},
    onExecute = () => {},
    onCancel = () => {},
    onReset = () => {},
    onClose = () => {},
  } = $props();

  let autoInspectionStarted = false;
  const inspection = $derived(state?.inspection ?? null);
  const facts = $derived(inspection?.documentFacts ?? null);
  const controls = $derived(inspection?.controls ?? []);
  const unsupportedBlockers = $derived(state?.blockingFindings ?? []);
  const requiredAcknowledgements = $derived(state?.requiredAcknowledgements ?? []);
  const result = $derived(state?.executionResult ?? null);
  const executionError = $derived(state?.executionError ?? null);
  const editingDisabled = $derived(state?.executionStatus === "executing" || state?.saveDialogOpen);

  $effect(() => {
    if (!show) {
      autoInspectionStarted = false;
      return;
    }
    if (runnable && state?.inspectionStatus === "idle" && !autoInspectionStarted) {
      autoInspectionStarted = true;
      onInspect();
    }
  });

  const factRows = $derived(facts ? [
    ["Şifreleme", facts.encrypted],
    ["Dijital imza", facts.signed],
    ["Incremental revision", facts.incrementalRevisionsDetected],
    ["XFA", facts.xfaDetected],
    ["Optional content", facts.optionalContentDetected],
    ["RichMedia", facts.richMediaDetected],
  ] : []);

  function selectProfile(profile) {
    const selected = profile === "verified_limited_v1"
      ? [...SANITIZATION_MINIMUM_PROFILE_CONTROL_IDS]
      : [...state.selectedControls];
    onPolicyChange({ profile, controls: selected });
  }

  function toggleControl(controlId, checked) {
    const selected = new Set(state.selectedControls);
    if (checked) selected.add(controlId);
    else selected.delete(controlId);
    if (controlId === "object_metadata" && checked) {
      selected.add("xmp_metadata");
      selected.add("page_metadata");
    }
    if ((controlId === "xmp_metadata" || controlId === "page_metadata") && !checked) {
      selected.delete("object_metadata");
    }
    selected.add("incremental_revisions");
    onPolicyChange({ profile: "custom_supported_v1", controls: [...selected] });
  }

  const supportLabel = (support) => ({
    supported_for_inspection: "İnceleme destekleniyor",
    experimental: "Deneysel inceleme",
    unsupported: "İnceleme desteği yok",
    not_applicable: "Kapsam dışı",
  })[support] ?? "Bilinmeyen inceleme desteği";

  const removalLabel = (support) => ({
    supported: "Kaldırma ve doğrulama destekleniyor",
    supported_with_visual_change: "Görsel değişiklikle destekleniyor",
    supported_with_functional_change: "İşlevsel değişiklikle destekleniyor",
    candidate: "Production desteği yok",
    candidate_with_visual_change: "Production desteği yok",
    candidate_with_functional_change: "Production desteği yok",
    experimental: "Deneysel — seçilemez",
    unsupported: "Kaldırma desteği yok",
    unverified: "Kaldırma doğrulanmadı",
    not_applicable: "Kapsam dışı",
  })[support] ?? "Bilinmeyen kaldırma desteği";

  const acknowledgementText = (name) => ({
    signatureInvalidationAccepted: "Full rewrite mevcut dijital imzaları ve sertifikasyon durumunu geçersiz kılabilir.",
    visualChangeAccepted: "Seçili kontroller belgenin görünümünü değiştirebilir.",
    formFunctionLossAccepted: "Seçili kontroller form işlevlerini kaybettirebilir.",
    encryptionChangeAccepted: "Çıktının şifreleme semantiği değişebilir.",
    standardConformanceLossAccepted: "PDF/A, PDF/X veya PDF/UA uygunluğu korunmayabilir.",
  })[name] ?? name;

  function transformedStatus(controlId) {
    const item = result?.transform?.controls?.find((control) => control.controlId === controlId);
    return item?.transformStatus === "not_found" ? "Bulunmadı" : "Kaldırıldı ve doğrulandı";
  }
</script>

{#if show}
  <AppDialog title="PDF sanitization" description="Verified sanitization workflow" size="lg" onclose={onClose}>
    <div class="sanitization">
      <div class="scope-note">
        Bu ön inceleme yalnız desteklenen PDF yapılarını tarar. Belgedeki bütün olası aktif veya gizli içeriğin bulunduğunu garanti etmez.
      </div>
      <p class="standard-note">Full rewrite sonrasında PDF/A, PDF/X, PDF/UA, certification ve linearization iddialarının korunması doğrulanmaz.</p>
      {#if !runnable}<p class="disabled-note">Feature disabled — workflow 8C-7 aktivasyon kararına kadar çalıştırılamaz.</p>{/if}

      {#if state?.workflowStatus === "idle"}
        <div class="status">Inspection bekleniyor.</div>
      {:else if state?.inspectionStatus === "inspecting"}
        <div class="status">Belge immutable snapshot üzerinden inceleniyor…</div>
      {:else if state?.inspectionStatus === "failed"}
        <div class="error" role="alert">{state.inspectionError?.message ?? "Inspection tamamlanamadı."}</div>
      {:else if state?.inspectionStatus === "cancelled" && state?.executionStatus === "idle"}
        <div class="status">Inspection iptal edildi.</div>
      {:else if inspection && facts}
        <section class="review-section">
          <h3>Belge bilgileri</h3>
          <dl class="facts">
            <div><dt>Sayfa sayısı</dt><dd>{facts.pageCount}</dd></div>
            {#each factRows as fact (fact[0])}
              <div><dt>{fact[0]}</dt><dd class:present={fact[1]}>{fact[1] ? "Tespit edildi" : "Tespit edilmedi"}</dd></div>
            {/each}
          </dl>
        </section>

        {#if inspection.blockingFindings.length > 0}
          <section class="blockers" aria-label="Sanitization engelleri">
            <h3>Engeller ve zorunlu risk onayları</h3>
            {#each inspection.blockingFindings as blocker (blocker.code + blocker.controlId)}
              <p><strong>{blocker.controlId}</strong> — {blocker.safeMessage}</p>
            {/each}
            {#if unsupportedBlockers.length > 0}
              <p class="hard-block">Desteklenmeyen içerik acknowledgement ile aşılamaz; Save dialog açılmaz.</p>
            {/if}
          </section>
        {/if}

        <section class="review-section">
          <h3>Policy profili</h3>
          <label class="profile">
            <input type="radio" name="sanitization-profile" checked={state.selectedProfile === "verified_limited_v1"}
              disabled={editingDisabled} onchange={() => selectProfile("verified_limited_v1")} />
            <span><strong>Verified limited profile</strong><small>Tanımlı minimum profilin bütün mandatory kontrolleri.</small></span>
          </label>
          <label class="profile">
            <input type="radio" name="sanitization-profile" checked={state.selectedProfile === "custom_supported_v1"}
              disabled={editingDisabled} onchange={() => selectProfile("custom_supported_v1")} />
            <span><strong>Custom cleanup</strong><small>Yalnız seçili ve backend-supported kontroller.</small></span>
          </label>
        </section>

        <section class="review-section">
          <h3>Kontroller</h3>
          <div class="controls">
            {#each controls as control (control.controlId)}
              {@const supported = SANITIZATION_SUPPORTED_CONTROL_IDS.includes(control.controlId)}
              {@const mandatory = state.selectedProfile === "verified_limited_v1" && supported}
              <article class:found={control.present} class:blocking={!!control.blockingReason}>
                <label class="control-title">
                  <input type="checkbox"
                    checked={state.selectedControls.includes(control.controlId)}
                    disabled={!supported || mandatory || editingDisabled || control.controlId === "incremental_revisions"}
                    onchange={(event) => toggleControl(control.controlId, event.currentTarget.checked)} />
                  <strong>{control.controlId}</strong>
                  <span>{control.present ? `${control.findingCount} bulgu` : "Bulunmadı"}</span>
                </label>
                <p>{supportLabel(control.inspectionSupport)} · {removalLabel(control.futureRemovalSupport)}</p>
                <p>Beklenen görsel etki: {control.expectedVisualImpact}</p>
                <p>Beklenen işlevsel etki: {control.expectedFunctionalImpact}</p>
                {#if mandatory}<p class="mandatory">Minimum profil için mandatory</p>{/if}
                {#if control.blockingReason}<p class="blocking-reason">{control.blockingReason}</p>{/if}
              </article>
            {/each}
          </div>
        </section>

        {#if requiredAcknowledgements.length > 0}
          <section class="acknowledgements">
            <h3>Zorunlu açık onaylar</h3>
            {#each requiredAcknowledgements as name (name)}
              <label>
                <input type="checkbox" checked={state.acknowledgements[name]} disabled={editingDisabled}
                  onchange={(event) => onAcknowledgementChange(name, event.currentTarget.checked)} />
                <span>{acknowledgementText(name)}</span>
              </label>
            {/each}
          </section>
        {/if}

        {#if state?.saveDialogOpen}
          <div class="status">Kaydetme hedefi seçiliyor… Execution henüz başlamadı.</div>
        {:else if state?.executionStatus === "executing"}
          <div class="status">
            Belge hazırlanıyor; seçili öğelerin kaldırılması, bağımsız doğrulama ve doğrulanmış çıktının yayımlanması backend tarafından yürütülüyor.
            {#if state.executionCancelRequested}<p>İptal isteği gönderildi. Yayınlama commit sınırı aşılmışsa disk sonucu korunacaktır.</p>{/if}
          </div>
        {:else if state?.executionStatus === "cancelled"}
          <div class="status">Sanitization işlemi publication öncesinde iptal edildi.</div>
        {:else if state?.executionStatus === "failed" && executionError}
          <div class="error" role="alert">
            <strong>{executionError.message}</strong>
            {#if executionError.recoveryPath}<p>Kurtarma yolu: {executionError.recoveryPath}</p>{/if}
            {#if executionError.replacementDigest}<p>Replacement digest: {executionError.replacementDigest}</p>{/if}
            {#if executionError.commitState}<p>Commit state: {executionError.commitState}</p>{/if}
          </div>
        {:else if state?.executionStatus === "completed" && result}
          <section class="result" aria-label="Sanitization sonucu">
            <h3>{result.assurance === "verified_limited"
              ? "Doğrulanmış sınırlı sanitization tamamlandı."
              : "Seçili öğelerin doğrulanmış temizliği tamamlandı."}</h3>
            {#if result.assurance === "verified_limited"}
              <p>Bu sonuç yalnız raporda listelenen sanitization kontrollerini kapsar; PDF'nin bütün olası aktif veya gizli içeriklerinden arındırıldığını garanti etmez.</p>
            {:else}
              <p>Seçili öğeler kaldırıldı ve doğrulandı. Diğer aktif veya gizli PDF içerikleri kapsam dışında olabilir.</p>
            {/if}
            <dl>
              <div><dt>Çıktı</dt><dd>{result.publication.targetPath}</dd></div>
              <div><dt>Assurance</dt><dd>{result.assurance}</dd></div>
              <div><dt>Profil</dt><dd>{result.profile}</dd></div>
              <div><dt>Doğrulama</dt><dd>{result.verification.isVerified ? "Bağımsız doğrulama geçti" : "Doğrulama başarısız"}</dd></div>
              <div><dt>Yayımlandı</dt><dd>{result.publication.published ? "Evet" : "Hayır"}</dd></div>
              <div><dt>Commit state</dt><dd>{result.publication.commitState}</dd></div>
              <div><dt>Output digest</dt><dd class="digest">{result.publication.publishedDigest}</dd></div>
            </dl>
            {#if result.publication.commitState === "new_committed_durability_unconfirmed"}
              <p class="warning">Doğrulanmış çıktı hedefe yazıldı; üst dizin durability onayı tamamlanamadı.</p>
            {/if}
            <ul>
              {#each result.controls as controlId (controlId)}<li>{controlId}: {transformedStatus(controlId)}</li>{/each}
            </ul>
          </section>
        {/if}
      {/if}

    </div>
    {#snippet footer()}
        {#if state?.executionStatus === "completed"}
          <button type="button" onclick={onReset}>Yeni sanitization</button>
        {:else if state?.inspectionStatus === "inspecting" || state?.executionStatus === "executing"}
          <button type="button" onclick={onCancel} disabled={!runnable || state.executionCancelRequested}>İptal</button>
        {:else}
          <button class="secondary" type="button" onclick={onInspect} disabled={!runnable || state?.saveDialogOpen}>Yeniden incele</button>
          <button type="button" onclick={onExecute} disabled={!runnable || !state?.readyForExecution || state?.saveDialogOpen}>Kaydet ve çalıştır</button>
        {/if}
    {/snippet}
  </AppDialog>
{/if}

<style>
  .sanitization { font-size: 13px; }
  .control-title { display: flex; align-items: center; gap: 12px; }
  h3, p { margin: 0; } h3 { margin-bottom: 8px; font-size: 13px; font-weight: 600; }
  .scope-note, .standard-note, .disabled-note, .status, .error, .blockers, .result { margin-top: 10px; padding: 10px 12px; border-radius: var(--radius); background: var(--muted); line-height: 1.45; font-size: 12px; }
  .scope-note:first-child { margin-top: 0; }
  .standard-note { background: var(--warning); color: var(--warning-foreground); }
  .disabled-note, .error, .blockers { color: var(--destructive); background: color-mix(in srgb, var(--destructive) 10%, transparent); }
  .review-section, .acknowledgements { margin-top: 16px; }
  .facts { display: grid; grid-template-columns: repeat(2, 1fr); gap: 6px; margin: 0; }
  .facts div, .result dl div { padding: 8px; border-radius: calc(var(--radius) * 0.8); background: var(--muted); }
  dt { color: var(--muted-foreground); font-size: 11px; } dd { margin: 2px 0 0; overflow-wrap: anywhere; font-size: 12px; } dd.present { color: var(--warning-foreground); }
  .profile, .acknowledgements label { display: flex; gap: 10px; margin-top: 8px; padding: 10px; border: 1px solid var(--border); border-radius: var(--radius); }
  .profile input, .acknowledgements input { accent-color: var(--primary); }
  .profile span { display: grid; gap: 3px; } small { color: var(--muted-foreground); }
  .controls { display: grid; gap: 6px; } article { padding: 10px; border: 1px solid var(--border); border-radius: var(--radius); }
  article.found { border-color: color-mix(in srgb, var(--warning-foreground) 50%, transparent); } article.blocking { border-color: color-mix(in srgb, var(--destructive) 60%, transparent); }
  article p { margin-top: 4px; color: var(--muted-foreground); font-size: 11px; }
  .control-title span { margin-left: auto; color: var(--muted-foreground); font-size: 11px; }
  .blocking-reason, .hard-block { color: var(--destructive); } .mandatory { color: var(--primary); }
  .result dl { display: grid; grid-template-columns: 1fr 1fr; gap: 6px; margin: 10px 0; } .digest { font-family: var(--font-code); font-size: 11px; }
  .warning { color: var(--warning-foreground); }
  button { padding: 5px 12px; border: 1px solid transparent; border-radius: calc(var(--radius) * 0.8); font-size: 12px; font-weight: 500; color: var(--primary-foreground); background: var(--primary); cursor: pointer; }
  button.secondary { border-color: var(--border); color: var(--secondary-foreground); background: var(--secondary); }
  button:disabled { cursor: not-allowed; opacity: .5; }
</style>

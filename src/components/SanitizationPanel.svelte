<script>
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
  <div class="backdrop" role="presentation" onclick={(event) => event.target === event.currentTarget && onClose()}>
    <div class="panel" role="dialog" aria-modal="true" aria-label="PDF sanitization workflow">
      <header>
        <div>
          <p class="eyebrow">VERIFIED SANITIZATION WORKFLOW</p>
          <h2>PDF sanitization</h2>
        </div>
        <button class="ghost" type="button" onclick={onClose}>Kapat</button>
      </header>

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

      <footer>
        {#if state?.executionStatus === "completed"}
          <button type="button" onclick={onReset}>Yeni sanitization</button>
        {:else if state?.inspectionStatus === "inspecting" || state?.executionStatus === "executing"}
          <button type="button" onclick={onCancel} disabled={!runnable || state.executionCancelRequested}>İptal</button>
        {:else}
          <button class="secondary" type="button" onclick={onInspect} disabled={!runnable || state?.saveDialogOpen}>Yeniden incele</button>
          <button type="button" onclick={onExecute} disabled={!runnable || !state?.readyForExecution || state?.saveDialogOpen}>Kaydet ve çalıştır</button>
        {/if}
      </footer>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 1000; display: grid; place-items: center; padding: 24px; background: rgba(0,0,0,.72); }
  .panel { width: min(760px, 96vw); max-height: 92vh; overflow: auto; padding: 20px; border: 1px solid #34373d; border-radius: 12px; color: #e5e7eb; background: #17191d; box-shadow: 0 24px 60px rgba(0,0,0,.5); }
  header, footer, .control-title { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  h2, h3, p { margin: 0; } h2 { font-size: 1.2rem; } h3 { margin-bottom: 10px; font-size: .95rem; }
  .eyebrow { color: #60a5fa; font-size: .68rem; letter-spacing: .12em; }
  .scope-note, .standard-note, .disabled-note, .status, .error, .blockers, .result { margin-top: 12px; padding: 12px; border-radius: 8px; background: #23262c; line-height: 1.45; font-size: .84rem; }
  .standard-note { border-left: 3px solid #f59e0b; } .disabled-note, .error, .blockers { border: 1px solid #7f1d1d; background: #2b191b; }
  .review-section, .acknowledgements { margin-top: 18px; }
  .facts { display: grid; grid-template-columns: repeat(2, 1fr); gap: 7px; margin: 0; }
  .facts div, .result dl div { padding: 9px; border-radius: 7px; background: #23262c; }
  dt { color: #9ca3af; font-size: .72rem; } dd { margin: 3px 0 0; overflow-wrap: anywhere; font-size: .82rem; } dd.present { color: #fbbf24; }
  .profile, .acknowledgements label { display: flex; gap: 10px; margin-top: 8px; padding: 10px; border: 1px solid #34373d; border-radius: 8px; }
  .profile span { display: grid; gap: 3px; } small { color: #aab1bd; }
  .controls { display: grid; gap: 8px; } article { padding: 11px; border: 1px solid #34373d; border-radius: 8px; }
  article.found { border-color: #9a6700; } article.blocking { border-color: #b91c1c; }
  article p { margin-top: 5px; color: #b7bdc8; font-size: .75rem; } .control-title { justify-content: flex-start; }
  .control-title span { margin-left: auto; color: #9ca3af; font-size: .72rem; }
  .blocking-reason, .hard-block { color: #fca5a5; } .mandatory { color: #93c5fd; }
  .result dl { display: grid; grid-template-columns: 1fr 1fr; gap: 7px; margin: 12px 0; } .digest { font-family: monospace; font-size: .72rem; }
  .warning { color: #fde68a; } footer { margin-top: 18px; padding-top: 14px; border-top: 1px solid #34373d; justify-content: flex-end; }
  button { padding: 8px 12px; border: 1px solid #4b5563; border-radius: 7px; color: white; background: #2563eb; }
  button.secondary, button.ghost { background: transparent; } button:disabled { cursor: not-allowed; opacity: .45; }
</style>

<script>
  let {
    state,
    onPlan = () => {},
    onExecute = () => {},
    onCancelPlan = () => {},
    onCancelExecution = () => {},
    onToggleSelection = () => {},
    onRemoveRegion = () => {},
    onClearRegions = () => {},
    onNavigate = () => {},
    onPolicyChange = () => {},
    onAcknowledgementChange = () => {},
    onNewRedaction = () => {},
  } = $props();

  const acknowledgementFields = [
    ["rasterizedPagesRequired", "rasterizedPagesAccepted", "Redaksiyon içeren sayfaların rasterize edilmesini kabul ediyorum."],
    ["textSearchLossRequired", "textSearchLossAccepted", "Metin seçimi ve arama yeteneklerinin kaybını kabul ediyorum."],
    ["accessibilityLossRequired", "accessibilityLossAccepted", "Erişilebilirlik etiketlerinin kaybını kabul ediyorum."],
    ["annotationLossRequired", "annotationLossAccepted", "Etkilenen sayfalardaki annotation kaybını kabul ediyorum."],
    ["formLossRequired", "formLossAccepted", "Etkilenen sayfalardaki form kaybını kabul ediyorum."],
    ["signatureInvalidationRequired", "signatureInvalidationAccepted", "İmza geçersizleşmesi riskini kabul ediyorum."],
    ["encryptionChangeRequired", "encryptionChangeAccepted", "Şifreleme değişikliği riskini kabul ediyorum."],
  ];
  const lossFields = [
    ["textSelectionLost", "Metin seçimi"],
    ["searchabilityLost", "Aranabilirlik"],
    ["accessibilityTagsLost", "Erişilebilirlik etiketleri"],
    ["linksLost", "Bağlantılar"],
    ["annotationsLost", "Annotation’lar"],
    ["formsLost", "Formlar"],
    ["vectorQualityLost", "Vektör kalitesi"],
    ["colorSpaceFidelityRisk", "Renk uzayı doğruluğu riski"],
    ["spotColorsLost", "Spot renkler"],
    ["layersLost", "Katmanlar"],
    ["structureAssociationLost", "Yapı ilişkilendirmeleri"],
    ["pageActionsLost", "Sayfa eylemleri"],
  ];

  let displayPlan = $derived(state?.plan ?? state?.reviewPlan ?? null);
  let affectedPages = $derived(displayPlan?.pagePlans?.filter((page) => page.strategy === "rasterize_page").length ?? 0);
  let unaffectedPages = $derived(displayPlan?.pagePlans?.filter((page) => page.strategy === "import_unaffected_page").length ?? 0);
  let requiredAcknowledgements = $derived(acknowledgementFields.filter(([required]) => displayPlan?.acknowledgementRequirements?.[required]));
  let activeLosses = $derived(lossFields.filter(([field]) => displayPlan?.losses?.[field]));
  let report = $derived(state?.result?.report ?? null);
  let publication = $derived(report?.publicationReport ?? null);
  let durabilityWarning = $derived(publication?.commitState === "new_committed_durability_unconfirmed");
</script>

<aside class="secure-redaction-panel" aria-label="Güvenli redaksiyon plan incelemesi">
  <header>
    <div>
      <h3>Güvenli redaksiyon</h3>
      <p class="status">Durum: {state?.status ?? "idle"}</p>
    </div>
  </header>

  <div class="candidate-warning">
    <strong>Bekleyen adaylar PDF içeriğini kaldırmaz.</strong>
    <span>Kesikli bölgeler yalnız inceleme seçimidir; güvenli sonuç değildir.</span>
  </div>

  {#if state?.status === "completed" && report}
    <section class="result success">
      <h4>Tamamlandı</h4>
      <p>{durabilityWarning ? "Çıktı hedefe yazıldı ve doğrulandı; üst dizin durability onayı tamamlanamadı." : "Doğrulanmış redaksiyon çıktısı kaydedildi."}</p>
      {#if state.result.cancelArrivedLate}
        <p class="warning">İptal isteği yayınlama başladıktan sonra ulaştı; doğrulanmış çıktı yayımlandı.</p>
      {/if}
      <dl>
        <dt>Çıktı</dt><dd>{publication?.publishedPath}</dd>
        <dt>Doğrulama</dt><dd>{report.verificationReport?.isVerified ? "Başarılı" : "Başarısız"}</dd>
        <dt>Etkilenen sayfa</dt><dd>{report.transformReport?.affectedPages?.length ?? 0}</dd>
        <dt>Raster DPI</dt><dd>{report.transformReport?.rasterDpi}</dd>
        <dt>Commit durumu</dt><dd>{publication?.commitState}</dd>
      </dl>
      <button type="button" onclick={onNewRedaction}>Yeni redaksiyon</button>
    </section>
  {:else}
    <section>
      <div class="row split">
        <h4>Bekleyen bölgeler ({state?.pendingRegions?.length ?? 0})</h4>
        <button type="button" class:active={state?.selectionMode} disabled={state?.executing} onclick={() => onToggleSelection(!state?.selectionMode)}>
          {state?.selectionMode ? "Seçimi kapat" : "Bölge ekle"}
        </button>
      </div>
      {#if !state?.pendingRegions?.length}
        <p class="muted">PDF sayfasında “Bölge ekle” ile dikdörtgen bir aday çizin.</p>
      {:else}
        <ul class="regions">
          {#each state.pendingRegions as region (region.id)}
            <li>
              <button type="button" class="link" onclick={() => onNavigate(region.pageIndex)}>Sayfa {region.pageIndex + 1}</button>
              <span>{region.source}</span>
              <button type="button" aria-label={`Adayı sil ${region.id}`} disabled={state.executing} onclick={() => onRemoveRegion(region.id)}>Sil</button>
            </li>
          {/each}
        </ul>
        <button type="button" disabled={state.executing} onclick={onClearRegions}>Tüm adayları temizle</button>
      {/if}
    </section>

    <section class="policy">
      <h4>Güvenli profil</h4>
      <p><code>secure_rasterized_pages_v1</code> · siyah, opak, solid görünüm</p>
      <label>
        Raster DPI
        <input type="number" min="72" max="600" value={state?.policy?.rasterDpi} disabled={state?.executing} onchange={(event) => onPolicyChange({ rasterDpi: Number(event.currentTarget.value) })} />
      </label>
      <label>
        Güvenlik payı (pt)
        <input type="number" min="0" max="36" step="0.25" value={state?.policy?.safetyMarginPoints} disabled={state?.executing} onchange={(event) => onPolicyChange({ safetyMarginPoints: Number(event.currentTarget.value) })} />
      </label>
      <p class="muted">İnteraktif içerik, imza ve şifreli belgeler conservative “block” politikasıyla ele alınır.</p>
    </section>

    <section class="actions">
      {#if state?.planning}
        <p class="progress">Plan hazırlanıyor…</p>
        <button type="button" onclick={onCancelPlan}>Planlamayı iptal et</button>
      {:else}
        <button type="button" class="primary" disabled={!state?.pendingRegions?.length || state?.executing || state?.saveDialogOpen} onclick={onPlan}>
          {displayPlan ? "Yeniden planla" : "Plan hazırla"}
        </button>
      {/if}
      {#if state?.planningError}
        <p class="error">{state.planningError.message}</p>
      {/if}
    </section>

    {#if displayPlan}
      <section class="plan-review">
        <h4>Plan inceleniyor</h4>
        {#if !state?.plan}
          <p class="warning">Redaksiyon girdileri veya kayıp kabulleri değişti. Bu inceleme özeti stale durumdadır; execution için yeniden planlayın.</p>
        {/if}
        <dl>
          <dt>Çalıştırılabilir</dt><dd>{displayPlan.executable ? "Evet" : "Hayır"}</dd>
          <dt>Execution için hazır</dt><dd>{state?.plan && displayPlan.readyForExecution ? "Evet" : "Hayır"}</dd>
          <dt>Etkilenen sayfa</dt><dd>{affectedPages}</dd>
          <dt>Etkilenmeyen sayfa</dt><dd>{unaffectedPages}</dd>
        </dl>

        {#if displayPlan.blockers?.length}
          <div class="blockers">
            <h5>Engelleyiciler</h5>
            {#each displayPlan.blockers as blocker (blocker.code)}<p>• {blocker.message}</p>{/each}
            <p class="muted">Bu engeller risk kabulüyle aşılamaz.</p>
          </div>
        {/if}
        {#if displayPlan.warnings?.length}
          <div class="warnings">
            <h5>Uyarılar</h5>
            {#each displayPlan.warnings as warning (warning.code)}<p>• {warning.message}</p>{/each}
          </div>
        {/if}
        {#if activeLosses.length}
          <div>
            <h5>Özellik kayıpları</h5>
            <ul>{#each activeLosses as [, label]}<li>{label}</li>{/each}</ul>
          </div>
        {/if}

        <p class="raster-wording">Redaksiyon içeren sayfalar rasterize edilir ve image-only hale gelebilir. Metin/arama, erişilebilirlik ve vektör özellikleri kaybolabilir. Çıktı bağımsız verifier’dan geçmeden yayımlanmaz.</p>

        {#if requiredAcknowledgements.length}
          <fieldset>
            <legend>Gerekli kayıp kabulleri</legend>
            {#each requiredAcknowledgements as [, accepted, label]}
              <label class="ack">
                <input type="checkbox" checked={state.acknowledgements[accepted]} disabled={state.executing} onchange={(event) => onAcknowledgementChange(accepted, event.currentTarget.checked)} />
                <span>{label}</span>
              </label>
            {/each}
          </fieldset>
        {/if}

        {#if state?.executing}
          <div class="execution-progress">
            <p>Güvenli redaksiyon uygulanıyor; ardından bağımsız doğrulama ve doğrulanmış çıktı yayını yürütülüyor…</p>
            <button type="button" disabled={state.cancelRequested} onclick={onCancelExecution}>{state.cancelRequested ? "İptal istendi" : "İptal et"}</button>
          </div>
        {:else}
          <button type="button" class="primary" disabled={!state?.readyForExecution || state?.saveDialogOpen} onclick={onExecute}>
            {state?.saveDialogOpen ? "Kaydetme hedefi seçiliyor…" : "Doğrulanmış çıktı kaydet"}
          </button>
        {/if}
      </section>
    {/if}

    {#if state?.executionError}
      <section class="result failure">
        <h4>İşlem tamamlanamadı</h4>
        <p>{state.executionError.message}</p>
        {#if state.executionError.recoveryPath}<p><strong>Kurtarma dosyası:</strong> {state.executionError.recoveryPath}</p>{/if}
        {#if state.executionError.commitState}<p><strong>Disk durumu:</strong> {state.executionError.commitState}</p>{/if}
      </section>
    {/if}
  {/if}
</aside>

<style>
  .secure-redaction-panel { width: 340px; height: 100%; overflow-y: auto; padding: 16px; box-sizing: border-box; background: var(--bg-sidebar, #121212); color: var(--text-primary, #eee); border-left: 1px solid var(--border-raw, #333); font-size: 12px; }
  header, .row, .split { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  h3, h4, h5, p { margin: 0; } h3 { font-size: 16px; } h4 { font-size: 13px; } h5 { margin: 8px 0 4px; }
  section { padding: 12px 0; border-bottom: 1px solid var(--border-raw, #333); display: grid; gap: 9px; }
  button, input { font: inherit; } button { padding: 6px 9px; border: 1px solid #555; color: inherit; background: #262626; cursor: pointer; } button:disabled { opacity: .5; cursor: not-allowed; }
  button.primary, button.active { border-color: #d97706; background: #78350f; } button.link { border: 0; padding: 0; background: none; color: #f59e0b; }
  .candidate-warning, .raster-wording { display: grid; gap: 4px; padding: 10px; margin-top: 10px; background: rgba(245, 158, 11, .12); border: 1px dashed #f59e0b; }
  .candidate-warning strong { color: #fbbf24; } .muted, .status { color: var(--text-muted, #999); }
  .regions { list-style: none; padding: 0; margin: 0; display: grid; gap: 6px; } .regions li { display: grid; grid-template-columns: 1fr 1fr auto; align-items: center; gap: 6px; }
  .policy label { display: grid; grid-template-columns: 1fr 100px; align-items: center; gap: 8px; } input[type="number"] { width: 100%; box-sizing: border-box; background: #191919; color: inherit; border: 1px solid #555; padding: 5px; }
  dl { display: grid; grid-template-columns: 1fr 1fr; gap: 4px 8px; margin: 0; } dt { color: var(--text-muted, #999); } dd { margin: 0; overflow-wrap: anywhere; }
  .blockers, .failure { padding: 9px; background: rgba(239, 68, 68, .12); border: 1px solid #7f1d1d; } .warnings, .warning { color: #fbbf24; }
  fieldset { margin: 0; border: 1px solid #555; display: grid; gap: 8px; } .ack { display: grid; grid-template-columns: auto 1fr; gap: 7px; align-items: start; }
  .progress, .execution-progress { color: #fbbf24; } .success { border: 1px solid #166534; padding: 10px; background: rgba(34, 197, 94, .1); } .error { color: #f87171; }
</style>

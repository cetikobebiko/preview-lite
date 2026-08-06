# Prompt 5.1 / Aşama 2 Doğrulama Durumu

Bu belge önceki, doğrulanmamış “bütün yollar tamamlandı / bütün testler geçti”
iddialarının yerine repository’nin 2026-07-28 durumunu kaydeder.

## Mimari sonuç

- Aktif PDF frontend IPC kimliği session-native’tir. Render yüzeyi
  `sessionId/contentGeneration`; read/job yüzeyi
  `sessionId/expectedContentGeneration` kullanır.
- `renderSession`, `PdfPage`, thumbnail/organizer, Sidebar, OcrLayer,
  AnnotationLayer, PrepressPanel, search coordinator ve annotation burn-in
  stale guard’ı aktif PDF kimliği olarak path taşımaz.
- Session snapshot source/working path yayınlamaz. `displayPath` ve `fileName`
  yalnız presentation/save-target bilgisidir.
- PDF share `share_file_macos` snapshot job’ıdır. Bağımsız image/external-file
  share `share_external_file_macos(path)` ile ayrıdır.
- PDF OCR session snapshot’ından çalışır. Bağımsız image OCR unsupported’dır.
- Signature verification snapshot job’ıdır. LTV ve gerçek PAdES bu çalışmada
  implemente edilmemiştir; LTV mock-disabled guard’ı ve
  `FEATURE_UNAVAILABLE` sonucu korunur.
- Metadata read helper’ı PDFium metadata API’sini kullanır. Metadata ve alt-text
  writes readonly/disabled UI ile sunulur ve Rust’ta typed unsupported hata
  döndürür.
- Session öncesi open/password input’ları, harici merge/image/batch operandları
  ve output target path’leri korunmuştur.

## Doğrulama kapsamı

Hedefli testler exact IPC payload’larını, legacy export yokluğunu, stale
generation/result reddini, snapshot cleanup’ı, share process timeout’unu ve OCR
temp-file cleanup’ını kapsar. Nihai sayılar yalnız çalıştırılan komutların gerçek
çıktısıyla görev tesliminde raporlanmalıdır; bu dosyada gelecekte eskimeye açık
“tüm testler geçti” toplamı tutulmaz.

## Snapshot job lifecycle doğrulaması

Snapshot-job command’ları canonical frontend UUID, session ownership, captured
generation ve generic `SessionJobOutcome<T>` kullanır. Hedefli testler
cancel-before-registration’ın operation closure’ını çalıştırmadığını; duplicate
ve cross-session kimliklerin typed admission hatası verdiğini; gerçek failure,
cooperative cancellation ve completed-after-cancel sonuçlarının ayrıldığını;
cleanup retry/deferred bilgisinin operation sonucuyla birlikte korunduğunu;
normal session close’un canlı snapshot job’ı iptal etmediğini ve
completion/cancel/reaper/close yarışlarının bounded tamamlandığını doğrular.

## Bilinen sınırlar

- Signature verification/PAdES/LTV kriptografik implementasyon değildir.
- PDF/X ve bazı ileri özellikler mevcut feature-status sınırlamalarını korur.
- Apple Vision OCR ve Share Sheet’in gerçek GUI davranışı bu birim testlerinde
  açılmaz; process, timeout, session snapshot ve cleanup sınırları test edilir.
## Candidate/transition lifecycle verification

Behavioral coverage lives in `src/services/documentOpenCoordinator.test.js` and
the `document_session::tests` Rust module. It covers first-open and image-to-PDF
null previous identity, latest-only commit-fence queuing, response-loss
reconciliation, post-commit render failure, transactional PDF-to-image,
password-dialog supersession, Save preservation after activation failure,
CandidateReady allowlisting, activation/transition expiry, credential retention,
corrupt/encrypted/password classification, and active-session authority.

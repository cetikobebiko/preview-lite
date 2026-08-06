# Engineering Baseline

Bu belge, 25 Temmuz 2026 tarihinde Prompt 0 kapsamında alınan değişiklik öncesi
kontrol noktasıdır. Amaç uygulamanın çalıştığını varsaymak değil, mevcut repository
durumunu ve doğrulama sınırlarını tekrar üretilebilir biçimde kaydetmektir.

## Kapsam ve karar özeti

- Kaynak kod, config veya dependency sürümü değiştirilmedi.
- Baseline komutları asıl çalışma ağacında değil, izinleri koruyan tam bir snapshot
  üzerinde ele alındı.
- Güvenilir dosya sistemi izolasyonu bulunamadığı için `npm install` çalıştırılmadı.
- Snapshot'ta `node_modules` bulunmadığı için Vitest ve Vite çalıştırılamadı.
- Rust format kontrolü çalıştı ve başarısız oldu. Diğer Cargo kontrolleri,
  izole `CARGO_HOME` içinden crates.io çözümlenemediği için derleme aşamasına
  ulaşamadı.
- Çalıştırılmayan veya derleme aşamasına ulaşmayan hiçbir kontrol başarılı olarak
  sınıflandırılmadı.
- Fuzz hedefi yalnızca envantere alındı; Prompt 0 komut listesinde olmadığı ve
  `cargo-fuzz` doğrulanmadığı için çalıştırılmadı.

## Repository kontrol noktası

| Alan | Değer |
|---|---|
| Kontrol noktası zamanı | `2026-07-25T16:04:02+0300` |
| Commit | `8a05265c121faab0aba05a76a21be4abc6191b8e` |
| Branch | `main` |
| Upstream | `origin/main`, başlangıçta `+0/-0` |
| Working tree | Başlangıçta dirty |
| Staged | 2 dosya |
| Unstaged | 82 dosya |
| Untracked proje dosyası | 18 dosya |
| `git diff --stat` | 82 dosya, 24.101 ekleme, 2.413 silme |
| `git diff --cached --stat` | 2 dosya, 4 silme |

Başlangıçtaki staged değişiklikler `public/cmaps/README.md` ve
`public/standard_fonts/README.md` silmeleridir. `package.json`,
`package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`,
`src/App.svelte`, `src-tauri/src/lib.rs`, yapılandırmalar, yeni Rust/Svelte
modülleri ve testler dahil geniş bir kullanıcı değişikliği kümesi zaten mevcuttu.
Bu değişiklikler Prompt 0 tarafından üretilmiş sayılmamalı ve hiçbirine reset,
restore, checkout, stash, clean veya otomatik format uygulanmadı.

## Doğrulanan teknoloji yığını

| Katman | Repository'deki durum |
|---|---|
| Frontend | Svelte `5.56.8`, Vite `5`, Vitest `4.1.10` |
| UI | Tailwind CSS `4.3.3`, shadcn-svelte `1.4.2` |
| Desktop | Tauri 2 |
| Backend | Rust, `pdfium-render 0.8` |
| Paket yöneticisi | npm |
| Lockfile | `package-lock.json` (lockfile v3) ve `src-tauri/Cargo.lock` mevcut |

`package.json` içinde `packageManager` sabitlemesi yoktur ve dependency
specifier'larının bir bölümü caret range kullanır. Mevcut `package-lock.json`
273 paket kaydı içerir; bu nedenle kurulum lockfile ile sınırlandırılmış olsa da
Node/npm sürümü ayrıca sabitlenmemiştir.

Repository Svelte 5'tedir; buna karşılık bazı mevcut bileşenlerde legacy
`export let` ve `$:` sözdizimi de bulunur. Build çalışmadığı için bunun Svelte
5/Vite/Vitest ile uyumluluğu doğrulanamadı. Bu görevde runes veya TypeScript
migrasyonu yapılmadı.

### Mimari sapma

Proje kuralı PDF işlemlerinde tek motor olarak PDFium'u öngörür. Buna karşın
`src-tauri/Cargo.toml` içinde `lopdf 0.34` vardır ve
`src-tauri/src/lib.rs`, `a11y.rs`, `redaction.rs` ve `prepress.rs` tarafından
PDF yazma/inceleme yollarında kullanılır. Bu, sonraki geliştirme öncesi çözülmesi
gereken mevcut bir mimari risktir; Prompt 0 kapsamında değiştirilmedi.

Kaynak taramasında aktif `pdfjs-dist`, PDF.js worker, TextLayer/AnnotationLayer,
`PDFDocumentProxy` veya `PDFPageProxy` kullanımı bulunmadı. `public/cmaps` ve
`public/standard_fonts` README dosyaları başlangıçta staged deletion,
`src/searchWorker.js` ise başlangıçta silinmiş durumdadır.

## Ortam

| Alan | Değer |
|---|---|
| İşletim sistemi | macOS 26.1, build `25B78` |
| Mimari | `arm64` |
| Aktif shell | `/bin/zsh` |
| Node | `v26.5.0` |
| npm | `11.17.0` |
| rustc | `1.97.1`, host `aarch64-apple-darwin` |
| Cargo | `1.97.1` |
| rustup | Komut bulunamadı; aktif rustup toolchain doğrulanamadı |
| npm registry | `https://registry.npmjs.org/` |
| npm lifecycle varsayılanı | `ignore-scripts=false` |
| `PDFIUM_PATH` | Ayarlı değil |
| Tesseract | `/opt/homebrew/bin/tesseract`, `5.5.2` |

Secret, token, signing key veya özel ortam değişkeni değeri kaydedilmedi.
Yalnızca görevle ilgili değişkenlerin ayarlı olup olmadığı kontrol edildi.

## Native runtime ve PDFium

| Yol | Tür / mimari | Boyut | SHA-256 |
|---|---|---:|---|
| `src-tauri/libpdfium.dylib` | Mach-O 64-bit dylib, arm64 | 7.151.568 bayt | `c2f68f87106da4f0cb6d6a90a7aad9644b3c1cb92a3463dbdc8f39449aec3869` |
| `src-tauri/lib/libpdfium.dylib` | Mach-O 64-bit dylib, arm64 | 7.151.568 bayt | `c2f68f87106da4f0cb6d6a90a7aad9644b3c1cb92a3463dbdc8f39449aec3869` |

`src-tauri/VERSION`: `MAJOR=152`, `MINOR=0`, `BUILD=7961`, `PATCH=0`.
Bundled iki dylib yalnızca macOS arm64 için kanıt sağlar; Windows, Linux veya
macOS x86_64 runtime uygunluğu kaynak koddan çıkarılamaz.

## Snapshot

Snapshot `/private/tmp/antigravity-baseline.TcSTSk/repo` altında `rsync -a` ile
oluşturuldu. Tracked dosyalar, ilgili untracked proje dosyaları, test fixture'ları,
binary asset'ler, PDFium kütüphaneleri, ikonlar ve diğer build girdileri dahil
206 dosya kopyalandı. Symlink, mode ve izin semantiğini koruyan arşiv kopyalama
kullanıldı; kapsamda symlink veya executable-bit taşıyan dosya bulunmadı.

Hariç tutulanlar:

- `.git`
- `node_modules`
- `dist`
- `src-tauri/target`
- `.DS_Store`
- `.env`, `.env.*`, `.npmrc`
- Cargo credential dosyaları
- `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.mobileprovision`

Hariç tutma öncesi secret/credential adayı taraması sonuç vermedi. Build/cache
çıktıları snapshot'a alınmadı: asıl repository'de `node_modules` yaklaşık
188 MB, `dist` yaklaşık 332 KB ve `src-tauri/target` yaklaşık 7 GB idi.

Kaynak ile snapshot'ın dosya içeriği manifest hash'i başlangıçta ve komutlardan
sonra aynıydı:

`469eb34ddc9fcd475ae1360bb8715fccff6b9d9adedee118f1a9085d01215a0e`

Checksum tabanlı `rsync -acn --itemize-changes` karşılaştırması da boş döndü.
Lockfile hash'leri komutlardan önce ve sonra değişmedi:

- `package-lock.json`:
  `39d3458edd777964061b9c72d713f118087b1ce1d19d4c8fdc7858ae6ef90e92`
- `src-tauri/Cargo.lock`:
  `1d14b15c4d78a861a3e6e07d2516e0818b88b268f1f6a188bbb2ec48431db665`

### Komutların oluşturduğu generated dosyalar

Snapshot repository ağacı içinde komutların oluşturduğu source, config, lock,
fixture veya başka proje dosyası: **none**.

Özellikle yeni/değişmiş `package-lock.json`, `Cargo.lock`, `node_modules`,
`dist` veya `src-tauri/target` yoktur. Snapshot sonrası checksum karşılaştırması
da proje ağacında değişiklik göstermedi.

Snapshot repository'sinin dışında, yalnız geçici çalışma kökü altında şu cache
artefaktları oluştu:

- `npm-cache/_logs/2026-07-25T13_08_30_270Z-debug-0.log`
- `npm-cache/_logs/2026-07-25T13_08_38_334Z-debug-0.log`
- `npm-cache/_update-notifier-last-checked`
- `cargo-home/.package-cache`
- `cargo-home/registry/CACHEDIR.TAG`
- `node-compile-cache/...` altında 398 cache blob'u
- `cargo-target/`: dosya yok

Bu geçici cache'lerin hiçbiri asıl repository'ye kopyalanmadı veya PR kapsamına
alınmadı.

`.git` kullanımı için package scriptleri, `build.rs` ve kaynaklar tarandı; build
ve test davranışını açıkça `.git` varlığına bağlayan bir yol bulunmadı. Komutlar
tamamlanamadığı için dolaylı bir bağımlılık bütünüyle dışlanamaz. Bu nedenle
snapshot'ta `.git` bulunmaması bir tekrar üretilebilirlik sınırlamasıdır ve asıl
repository üzerinde komut çalıştırılarak telafi edilmedi.

## İzolasyon kararı

`npm install` öncesinde gerçek izolasyon mekanizmaları denetlendi:

- Codex çalışma ortamı ağı sınırlar, ancak process'in snapshot dışındaki kullanıcı
  dosyalarını okumasını teknik olarak engellemez.
- `/usr/bin/sandbox-exec` bulunmasına rağmen kontrollü probe
  `sandbox_apply: Operation not permitted` ile exit code `71` döndürdü.
- Docker CLI bulundu, fakat daemon/socket kullanılamadı
  (`/Users/umut/.docker/run/docker.sock` yok).
- Podman ve Bubblewrap bulunmadı.

Sonuç: lifecycle scriptlerinin HOME, credential alanları veya snapshot dışındaki
kullanıcı dosyalarına erişimini teknik olarak engelleyen çalışır bir dosya sistemi
sandbox'ı yoktu. Kullanıcının açık koşulu gereği `npm install` çalıştırılmadı.
Geçici HOME/config/cache veya `--ignore-scripts` ikame edilmedi. Ağ erişimi için
escalation istenmedi; çünkü dosya sistemi izolasyonu sağlanmadan indirilen paket
ve build scriptlerini çalıştırmak kabul edilen sınırı ihlal edecekti.

## Baseline komut sonuçları

Komutlar asıl repository yerine snapshot'ta, izole geçici HOME/cache dizinleriyle
ve belirtilen sırada ele alındı. Zamanlar Europe/Istanbul (`+03:00`) bölgesindedir.
Log başlangıç/bitişleri dosya birth/modified zamanlarıyla, kısa komutlar için süre
de shell `time` çıktısıyla kaydedildi.

| Komut | CWD | Başlangıç – bitiş | Süre | Exit | Sonuç |
|---|---|---|---:|---:|---|
| `npm install` | snapshot kökü | `2026-07-25 16:08` izolasyon kararı | — | — | **could not run / isolation unavailable** |
| `npm run test` | snapshot kökü | `16:08:30` – `16:08:30` | 0,47 sn | 127 | **failed:** `vitest: command not found` |
| `npm run build` | snapshot kökü | `16:08:38` – `16:08:38` | 0,14 sn | 127 | **failed:** `vite: command not found` |
| `cargo fmt --all -- --check` | snapshot `src-tauri` | `16:08:55` – `16:08:58` | 3,40 sn | 1 | **failed:** 18 Rust dosyasında format diff'i |
| `cargo check` | snapshot `src-tauri` | `16:09:42` – `16:10:52` | yaklaşık 70 sn | 130 | **could not complete — crates.io/DNS unavailable; manually interrupted** |
| `cargo test` | snapshot `src-tauri` | `16:11:12` – `16:11:24` | yaklaşık 12 sn | 130 | **could not complete — crates.io/DNS unavailable; manually interrupted** |
| `cargo clippy --all-targets --all-features -- -D warnings` | snapshot `src-tauri` | `16:11:32` – `16:11:45` | yaklaşık 13 sn | 130 | **could not complete — crates.io/DNS unavailable; manually interrupted** |

### Sonuç ayrıntıları

`npm run test`:

```text
> preview-alternative@0.1.0 test
> vitest run
sh: vitest: command not found
```

`npm run build`:

```text
> preview-alternative@0.1.0 build
> vite build
sh: vite: command not found
```

Bu iki hata kaynak kod testi/build sonucu değildir; güvenli kurulum yapılamadığı
ve snapshot'a `node_modules` kopyalanmadığı için executable bulunamamıştır.

`cargo fmt --check` kaynak dosyalarını değiştirmeden aşağıdaki 18 dosyada diff
raporladı:

```text
src-tauri/src/a11y.rs
src-tauri/src/ai.rs
src-tauri/src/annotations.rs
src-tauri/src/batch.rs
src-tauri/src/error.rs
src-tauri/src/export.rs
src-tauri/src/lib.rs
src-tauri/src/manipulation.rs
src-tauri/src/ocr.rs
src-tauri/src/pdf_engine/mod.rs
src-tauri/src/pdf_engine/subprocess_manager.rs
src-tauri/src/plugins.rs
src-tauri/src/prepress.rs
src-tauri/src/redaction.rs
src-tauri/src/signature.rs
src-tauri/src/worker/ipc.rs
src-tauri/src/worker/pdfium.rs
src-tauri/tests/crash_tests.rs
```

`cargo check`, `cargo test` ve `cargo clippy` izole boş `CARGO_HOME` ile crates.io
indexini çözümleyemedi:

```text
warning: spurious network error: [6] Couldn't resolve host name
(Could not resolve host: index.crates.io)
```

Tekrarlanan DNS denemeleri sonrası processler manuel olarak kesildi. Dolayısıyla
Rust kodu compile edilmedi; test ve Clippy sonucu üretilmedi. Tam ham loglar PR'a
eklenmedi. Bu üç sonucu build/test failure olarak yorumlamak doğru değildir:
dependency çözümlemesi tamamlanamadığı için kontroller sonuç üretemedi. Önceden
belirlenmiş sabit bir timeout uygulanmadı; tekrar eden DNS retry'larının ilerleme
sağlamadığı gözlendikten sonra processler manuel olarak kesildi.

Ham loglar bu revizyon sırasında hâlâ
`/private/tmp/antigravity-baseline.TcSTSk/logs/` altında mevcuttur. Bu konum
geçicidir, repository veya PR artefaktı değildir ve işletim sistemi tarafından
temizlenebilir. Bu nedenle ham loglar **kalıcı olarak korunmuş değildir**; kalıcı
kanıt bu belgede özetlenen komut, zaman, exit code ve hata parçalarıdır.

## Test envanteri ve kapsam sınırı

- 9 Vitest dosyasında 47 `it(...)` vakası bulundu.
- Bileşen testlerinin önemli bölümü Tauri `invoke` çağrılarını mock eder; bu,
  Rust/PDFium runtime entegrasyonunu doğrulamaz.
- 4 Rust test fonksiyonu bulundu: AI cosine benzerliği, plugin işlem limiti,
  eksik PDF hata yolu ve crash yaşam döngüsü.
- `src-tauri/fuzz` altında `pdfium_lifecycle` fuzz hedefi, corpus ve dictionary
  vardır. Fuzz hedefi çalıştırılmadı.
- Vitest executable bulunamadığı ve Cargo dependency çözümlemesi tamamlanamadığı
  için bu testlerin tamamı **unrun** durumundadır.

Özellik bazındaki test eşlemesi `feature-matrix.md` içindedir.

## CI ve platform notları

`.github/workflows/release.yml` tag tabanlı Linux/macOS/Windows matrisi,
`npm install`, frontend testleri, Cargo testleri ve Tauri release adımları içerir.
`.github/workflows/build-windows.yml` Windows build/artifact üretir, ancak ayrı
frontend veya Rust test adımı içermez. Bu görevde hiçbir publish, release, deploy
veya signing işlemi çalıştırılmadı.

Tauri capabilities dialog open/save, shell open ve geniş filesystem read/write
izinleri tanımlar. Builder dialog, fs, shell ve log pluginlerini kaydeder.
Updater dependency'si mevcut olsa da updater plugin kaydı kaynakta görülmedi.

## Asıl repository görev sonu kontrol noktası

Final salt-okunur Git kontrolü `2026-07-25T17:54:16+0300` tarihinde asıl
repository'de alındı. Snapshot hash'i bu kontrolün yerine kullanılmadı.

| Alan | Başlangıç | Final | Fark |
|---|---:|---:|---|
| Commit | `8a05265c121faab0aba05a76a21be4abc6191b8e` | aynı | yok |
| Branch/upstream | `main`, `origin/main`, `+0/-0` | aynı | yok |
| Staged dosya | 2 | 2 | yok |
| Unstaged dosya | 82 | 82 | yok |
| Untracked dosya | 18 | 21 | yalnız 3 Prompt 0 belgesi |

Final staged yollar başlangıçla aynıdır:

- `public/cmaps/README.md`
- `public/standard_fonts/README.md`

Final unstaged yollar başlangıçla aynıdır ve Appendix A'da eksiksiz listelenir.
Final untracked yollar başlangıçtaki 18 yola ek olarak şunları içerir:

- `docs/engineering/baseline.md`
- `docs/engineering/feature-matrix.md`
- `docs/engineering/ipc-inventory.md`

Prompt 0 tarafından oluşturulan/değiştirilen dosyaların tam listesi bu üç
belgedir. `rsync -acn --itemize-changes` ile asıl working tree ve başlangıç
snapshot'ı karşılaştırıldığında, hariç tutulan `.git`/cache/build/credential
kapsamı dışında yalnız `docs/`, `docs/engineering/` dizinleri ve bu üç dosya
görüldü. Başlangıçta mevcut hiçbir tracked veya untracked kullanıcı dosyasında
Prompt 0 kaynaklı içerik farkı bulunmadı. Beklenmeyen değişiklik: **yok**.

Final `git status --porcelain=v2 --branch --untracked-files=all` çıktısı Appendix
C'de verbatim olarak korunur. Final dosya yolları bu raw çıktıdan da
denetlenebilir.

## Sonraki aşamayı bloke eden konular

1. Güvenilir process/filesystem sandbox'ı olmadığı için dependency kurulumu
   yapılamıyor.
2. Snapshot içinde frontend dependency'leri yok; test ve build çalıştırılamıyor.
3. Boş `CARGO_HOME` ve kısıtlı ağ nedeniyle Rust dependency'leri çözümlenemiyor.
4. `cargo fmt --check` 18 dosyada başarısız.
5. Frontend–handler payload uyuşmazlıkları normal UI yollarını kırıyor; ayrıntılar
   `ipc-inventory.md` içindedir.
6. `lopdf` kullanımı tek-PDFium mimari kuralıyla çelişiyor.
7. Yalnızca macOS arm64 PDFium binary'si doğrulandı; çapraz platform paketleme
   kanıtlanmadı.

Bu maddelerin hiçbiri Prompt 0 kapsamında düzeltilmedi. Her kaynak veya dependency
değişikliği ayrı onay gerektirir.

## Kaynak koddan kesin çıkarılamayanlar

- Uygulamanın gerçek PDF örnekleriyle runtime davranışı ve performansı.
- Svelte 5/Tailwind 4/Vite/Vitest kombinasyonunun build uyumluluğu.
- Tauri'nin başında `_` bulunan Rust parametrelerini IPC anahtarına dönüştürme
  davranışı; build olmadan ilgili macro çıktısı doğrulanamadı.
- PDFium dyliblerinin Windows/Linux/macOS x86_64 karşılıklarının bulunabilirliği.
- Ollama, sistem sertifika deposu, yazıcı, OCR ve platform servislerinin gerçek
  makinedeki uçtan uca davranışı.
- Güvenlik/standart uyumu iddia eden redaction, PAdES, LTV, PDF/UA ve PDF/X
  yollarının bağımsız doğrulama araçlarındaki sonucu.
- `.git` yokluğuna dolaylı olarak bağlı olabilecek üçüncü taraf build scripti
  davranışı.

## Appendix A — Başlangıç working tree dosya yolları

Bu listeler `2026-07-25T16:04:02+0300` başlangıç kontrol noktasına aittir.

### Başlangıç staged — 2

```text
public/cmaps/README.md
public/standard_fonts/README.md
```

### Başlangıç unstaged — 82

```text
.github/workflows/release.yml
diff.txt
my_diff.txt
package-lock.json
package.json
src-tauri/Cargo.lock
src-tauri/Cargo.toml
src-tauri/capabilities/default.json
src-tauri/fuzz/Cargo.toml
src-tauri/fuzz/corpus/minimal.pdf
src-tauri/fuzz/dict/pdf.dict
src-tauri/fuzz/fuzz_targets/pdfium_lifecycle.rs
src-tauri/gen/schemas/acl-manifests.json
src-tauri/gen/schemas/capabilities.json
src-tauri/gen/schemas/desktop-schema.json
src-tauri/gen/schemas/macOS-schema.json
src-tauri/src/a11y.rs
src-tauri/src/ai.rs
src-tauri/src/annotations.rs
src-tauri/src/batch.rs
src-tauri/src/dummy_check.rs
src-tauri/src/error.rs
src-tauri/src/export.rs
src-tauri/src/lib.rs
src-tauri/src/main.rs
src-tauri/src/manipulation.rs
src-tauri/src/ocr.rs
src-tauri/src/pdf_engine/mod.rs
src-tauri/src/pdf_engine/subprocess_manager.rs
src-tauri/src/plugins.rs
src-tauri/src/prepress.rs
src-tauri/src/redaction.rs
src-tauri/src/signature.rs
src-tauri/src/test_annot.rs
src-tauri/src/worker/ipc.rs
src-tauri/src/worker/mod.rs
src-tauri/src/worker/pdfium.rs
src-tauri/tauri.conf.json
src-tauri/tests/crash_tests.rs
src/App.svelte
src/annotationsStore.js
src/app.css
src/components/A11yPanel.svelte
src/components/AiContextMenu.svelte
src/components/AiSettingsModal.svelte
src/components/AiSidebar.svelte
src/components/AnnotationLayer.svelte
src/components/AutoRedactSidebar.svelte
src/components/DeveloperConsole.svelte
src/components/ExportModal.svelte
src/components/FormLayer.svelte
src/components/MergeSplitModal.svelte
src/components/OcrLayer.svelte
src/components/OutlineItem.svelte
src/components/PageOrganizer.svelte
src/components/PasswordModal.svelte
src/components/PdfPage.svelte
src/components/PdfXWizard.svelte
src/components/PluginManagerModal.svelte
src/components/PrepressPanel.svelte
src/components/ReadingOrderOverlay.svelte
src/components/RedactionToolbar.svelte
src/components/SanitizationDialog.svelte
src/components/SearchBar.svelte
src/components/SearchBar.test.js
src/components/ShortcutsModal.svelte
src/components/Sidebar.svelte
src/components/SignaturePanel.svelte
src/components/SignatureSettingsModal.svelte
src/components/SignatureWizardModal.svelte
src/components/TagTreeExplorer.svelte
src/components/Thumbnail.svelte
src/components/ThumbnailItem.svelte
src/configManager.js
src/ipc_profiler.js
src/main.js
src/pdfState.js
src/pluginStore.js
src/searchWorker.js
svelte.config.js
vite.config.js
vitest.config.js
```

### Başlangıç untracked — 18

```text
AGENTS.md
components.json
jsconfig.json
src-tauri/pdf_test/Cargo.toml
src-tauri/pdf_test/src/main.rs
src/components/PageOrganizer.test.js
src/components/PdfPage.test.js
src/components/PdfXWizard.test.js
src/components/PrepressPanel.test.js
src/lib/utils.ts
src/utils/canvasUtils.js
src/utils/canvasUtils.test.js
src/utils/searchCoordinator.js
src/utils/searchCoordinator.test.js
src/utils/searchResultUtils.js
src/utils/searchResultUtils.test.js
src/utils/textLayerUtils.js
src/utils/textLayerUtils.test.js
```

## Appendix B — Başlangıç source/config/lock SHA-256 manifesti

Bu manifest başlangıç snapshot'ındaki 104 source, config ve lock dosyasını kapsar.
Binary asset/fixture'lar tam-snapshot aggregate hash kapsamındadır; bu appendix
özellikle denetlenebilir source/config/lock çiftlerini verir.

```text
238e3c412d7aa890fdc79c3e1fc9343e7b0edfcf667c0c8bc6c36288055c5ea3  .github/workflows/build-windows.yml
2476ec833144a5e10f956e9c1a13fd005e8d2cc0ddc255906e1bfca1095b069f  .github/workflows/release.yml
41f464a8c670c3c99abeb7d70e7bf1355e4b1572e0ca363e08f8ba2a88aa52b8  AGENTS.md
d8974daf13c7d7205d1786f0ef508f4f21c2ab9646a1522f3354f25482a07996  components.json
f03789fac67e1826a4ec1f71bcba8f94a676a00f9b73d6fcb9b2406a8956b6d2  jsconfig.json
39d3458edd777964061b9c72d713f118087b1ce1d19d4c8fdc7858ae6ef90e92  package-lock.json
adf4930b7684bfc67111b0004d609fe2ffb76fc649589ae78a182d79d795de4d  package.json
6aeeab41bc35efde4e3c2b644a6248f7fc06d24ae30b75c0d33491cbb0c1fa31  src-tauri/build.rs
3e4216eb1f619227d07ca4376b995f2670067942f2b147d467da55c068c24d3a  src-tauri/capabilities/default.json
1d14b15c4d78a861a3e6e07d2516e0818b88b268f1f6a188bbb2ec48431db665  src-tauri/Cargo.lock
b9b795290889adef299ecdbbe5e385dffcdb607f37b9c7eae42a25ac26f26fe6  src-tauri/Cargo.toml
b12e115f8dd58e3cc81ae649d565c3a23fff18fc8a4bc5eaa7a109a0271d52f2  src-tauri/fuzz/Cargo.toml
8fa1bee710923b9a206cb01bb42a9268f3785d565fd766ca98a76fcc6911b972  src-tauri/fuzz/fuzz_targets/pdfium_lifecycle.rs
d74cf8f88dc9996a88f4d1b00fe651d7554201a4637f70e33a74ecf9707fa196  src-tauri/gen/schemas/acl-manifests.json
2183fbde0fe66ed789c9cd93018787374487f8115b5bcab89b1223fc6c625d97  src-tauri/gen/schemas/capabilities.json
21ffe99bb77dfeed601023d392a7cfafa8ba47ce38854c62478e634cd6b20750  src-tauri/gen/schemas/desktop-schema.json
21ffe99bb77dfeed601023d392a7cfafa8ba47ce38854c62478e634cd6b20750  src-tauri/gen/schemas/macOS-schema.json
ba3fa2bbcc0474ff0126c61559409221f34a5ae392d897a58ec654f1aa309c17  src-tauri/pdf_test/Cargo.toml
c8e0583694bb1e0188dbe28fe0d65ac1130723c55f968b6262b906c147f72549  src-tauri/pdf_test/src/main.rs
a33ca4b447f08c597039814488d9310249f9949c7a5d56b2424d1b74d98ca4a6  src-tauri/src/a11y.rs
c8f4d3e90a5490b0b2532834f820f0b633e5d14dcbdccf9a5e2f4480eca47997  src-tauri/src/ai.rs
112bee3bff2833ee50cda0e60b85c360f7fbcf5bf1c3c59d5248fc64cfcbff72  src-tauri/src/annotations.rs
635d2e3d1266fa3f13fad364baa7a8e722018a072a16e83f37ff912af3a33b11  src-tauri/src/batch.rs
53326bbc02a7ce5b8868144c7a160863a7a96918a61951116d47202b28aad711  src-tauri/src/dummy_check.rs
496a5fcb55a229b0fb96574e8388608918d0495c8ae54b76767660af59f7f1b2  src-tauri/src/error.rs
83afeb1812ab49ad1c1c78a230a0cab7d3ccc965d8f14ee4f81722dafb6692e4  src-tauri/src/export.rs
e5a2072eecc690ec39251c326d85735d2dbc2d00260bd16c9d536fac90360303  src-tauri/src/lib.rs
d59cc4d777e0a2f3d75dd32fd0d45a95946d8d79e80c3f1a6ed3a90ef78170e3  src-tauri/src/main.rs
cc32f1cc8691cbf6549cb7f09b3306079d0e4bcd6849dd90c344bc40d6e4882b  src-tauri/src/manipulation.rs
74717cc60126dae0d2753b224845b423121327fe18a015bba20b1fe40c053ebf  src-tauri/src/ocr.rs
131147acd707f7c4e8a50185799d20eb196277401bafd82b38ce423e2a6a78cd  src-tauri/src/pdf_engine/mod.rs
f8776e606b55271d22e0db2fc1d1432c51a97506eed1db61d142f2986fd400f2  src-tauri/src/pdf_engine/subprocess_manager.rs
32945fc3dcb0e0e8d104a575876fb4037bf402946b16b571bf298ad6a270cd84  src-tauri/src/plugins.rs
d04a2916f5567c205792708e6926887ffc58101e84ab1ddf1772a4c7a68f6a3f  src-tauri/src/prepress.rs
e46195fe7b766659a59bdf54f2348f9a4a1cae381f9b68da9d5e276e0ef9828d  src-tauri/src/redaction.rs
160b0026753cd4bfdff432b1b7f1e63873b02e6f327f08f0242afb127ffa02bd  src-tauri/src/signature.rs
a791d6c7aa45f1e88c21cb3fe31c6412eaf420f90c7d2ab119f59f26b3d254c4  src-tauri/src/test_annot.rs
5f763c5ef67e49983c36f6efe0dd6f6756d3a9f3589c31e19c39264e92304d2d  src-tauri/src/worker/ipc.rs
f8b86461a6a8e1410a1a45d6fa0d36a3b81fd5e4efc3b69f7aa581553ce6bfc5  src-tauri/src/worker/mod.rs
fbfa69db8b7f2b7f9649033b29469f2b388e9ce01641f17dd07584bc7dbc5cb5  src-tauri/src/worker/pdfium.rs
a7b695c296d84728b5c8863cd1ab52bdca6136c88baddcd98bcd55c61f9c4a0d  src-tauri/tauri.conf.json
9b7df5930f485444be0196e8253e08ee516b41cd45ff063c5d37662868bd8191  src-tauri/tests/crash_tests.rs
d2ee2383762e1df52855ab8eaa83bb17074b6a49dacc425d16c95cd2b1a51832  src-tauri/VERSION
0755c5fbdb439b19d2debb7a932e4399390ea081d3f750c833011fe4aaa10fc5  src/annotationsStore.js
bab833e893deae89d5b0c3436254ea0b71740c500dbc37302ca6e16b2665c68a  src/app.css
cfd38bc27b7178a63a405a265667ec7b28e36add4667403176ada17bdb7ef9b6  src/App.svelte
313da0e1e689a74948dbdfd0ab9fc193487b87b56beb1eff60c47132ea4e0ecb  src/components/A11yPanel.svelte
32d2303361f9df946e7e7781897de12a33e4e80c7fb65079ad1b9eb9270da795  src/components/AiAssistantModal.svelte
44eb759c9e360113fa4cc2e21f617798f3f372cbf1ca19bb6094f7cb69cf354c  src/components/AiContextMenu.svelte
d99445f8040bd2e33f5001abee297c6b4316b01890ae88193e29ae607a9ca4cd  src/components/AiSettingsModal.svelte
b6924011152ca306e2e7fef570ce6261c775536deb6416b583413f8c421ace0e  src/components/AiSidebar.svelte
a8dc0f262ef6c93b359d67b2c3f0221323f00625fe7f7fe1d8af1df3b64530a8  src/components/AnalyticsModal.svelte
cf07f922d7ef43ac42d2163fd272934f8b3574bdcddb6761148b9545ba4cbcf9  src/components/AnnotationLayer.svelte
212cf5d96329e78001f972f202e5ef9c178f97193ce89935c5feadece0584c1c  src/components/AutoRedactSidebar.svelte
cbb75f53377b57dc4a4d49016025781092f614e64b185df82afd169d098b6822  src/components/BionicReadingModal.svelte
77a7f4316cea8b8ef47639c77f811bdfa1eae2273034f314dea61e6717c67ebe  src/components/CommandPalette.svelte
8e6e8ec5f76038374863ee1e9f33674972c92f6cfc20583c649f57c94c2d0057  src/components/DeveloperConsole.svelte
2a1f065dd3bb8ae0fcc6e010dd11b3be027e72aab3b8ebd79bc73aa63abaa6f6  src/components/ExportModal.svelte
512138f8354c702ec6ea92c45c0f89e8b53ba1eff4ccf466fa838314adf95775  src/components/FormLayer.svelte
fcee99143d840551a1a9c032e789adb0bdee1cd928d3f1cfccc1e62e605497ec  src/components/MergeSplitModal.svelte
7e9b532c1761a50317d04437c137a41810852804c4a35a49461282f62335d9d1  src/components/MetadataModal.svelte
cd231206725e319e7d745c296f4697f9ed6c4d503fb75fd550e9bcdc37f2f4a4  src/components/OcrLayer.svelte
d2ed61a31364b2c947fd480fe25424b8d1d4cadf31a668b65d60d93e7fbe7e53  src/components/OutlineItem.svelte
aac7cd283279bf6bce89a23971a13a9b27397c22cbd67edf738d7665bce070df  src/components/PageOrganizer.svelte
8573774783fe0b99ed04c018a5993087a9ef37589d6e95ca9de08002f35a6720  src/components/PageOrganizer.test.js
18c3f0a3131f9da847dcb2c8850e6c74c0024d06004dd77a5e5e032af866306f  src/components/PasswordModal.svelte
da3b93c92f3b645dba2fc5cee8bacc9306f0f9ea758aeb0c8bb9d5312a9a0165  src/components/PdfPage.svelte
8ab6d356c83e4c4c5b773931c57c2b8e8e7201a45b2f310bd60cfead0c559457  src/components/PdfPage.test.js
af39d37995d19021cea14b07f226fa18c6bb6fa0a44d66de76e231ddaf2c0b27  src/components/PdfXWizard.svelte
34c4780bf8f44b095bc6dffd7623e2e312dacaeda265c34b9f63e098de51b195  src/components/PdfXWizard.test.js
8d7d109cd3b3b99ac02687961e17f34ce18e2b2222be4f1fa561561daab87e7f  src/components/PluginManagerModal.svelte
3be4994e41e20255689ef58a0914d4955d162e687ab9604ac1c5b26e962d1e89  src/components/PrepressPanel.svelte
366537aa13bba33181e460b4837e3af0e735fe75f0800c61346fe0bb07245b93  src/components/PrepressPanel.test.js
d8721be642e4e96efdd47d6f8dd65213efcf27412473a3cca090a9dfd07ed97c  src/components/ReadingOrderOverlay.svelte
cdb37829ae1f84582db7780a5813ef419de43095bb02c10b830c65612f0603e2  src/components/ReadingRuler.svelte
cc1ba10a262d17ad0faef6152256d1d5f13e558831ce95556f4191e3ad0b5328  src/components/RedactionToolbar.svelte
fccf0f6176151c4b60f74317039090dde8ca7f9ca9c89a7570e526ce1b717348  src/components/SanitizationDialog.svelte
4bab115ec14d930fa5e1b40d42e3ba8053bb9aa2a44c9beb0f05aa824e869d97  src/components/SearchBar.svelte
106ce78b1015a4b75b8543b713e9d75be2cd77c4d5f65c2540b896c2fa72dab4  src/components/SearchBar.test.js
3b658f5c038b5525e33af29bd5c542270a5cf8104544e0b86254627721619bd2  src/components/ShortcutsModal.svelte
4288ac60349507534bab64507cefea7a590fcf6ed4bac3fc93cabc27e8a37fa6  src/components/Sidebar.svelte
8c7549a1ddfd811e3454a8e35b29c113ae0e21d88159c816c9b47c09590f953f  src/components/SignaturePanel.svelte
2a25a02379472ef85d76a70734b8457a911145fb3b05fe811349d359f292f87d  src/components/SignatureSettingsModal.svelte
86c4e75fb8211518461f6fb5a81c82cafa19e790299d5ef8c9f47d6acd6772c1  src/components/SignatureWizardModal.svelte
8aaeaeddca23c98438f783164b4428aa67deb4480e942530bf280acc571aa356  src/components/TagTreeExplorer.svelte
48adec21d5c5c9f9c8df3f6dccaf060dccd59e82c2018e5991a644630bce12a4  src/components/Thumbnail.svelte
adaa7bfa1357fa5ab5463a874bd233dd50a158a2cf5f726cdbb92056c900d670  src/components/ThumbnailItem.svelte
b70d1f91c5253a44b44775b78588d032ef1f0640620c9cab0be70eb419723a5b  src/configManager.js
80c864fe4fb21651ca4b6da6921ecd2e7f6f039acd6a541f6577485e8941ad89  src/ipc_profiler.js
97d136e64b4db75064bcc44f3bff64293818f504e2c29524bac813ec425a1d63  src/lib/utils.ts
3b4d564934593237822e2946163d3b4f9e8502b225a246fdb3454b01704053f3  src/main.js
190877254e691059a3fb5cd4fa6033aabd6e5c37754ecb1fb21b1433e0753359  src/pdfState.js
d43d46446813c0cc8173d98dcba4d5ca93e5dbe1cb5d253ed83c32e4eea8cf51  src/pluginStore.js
d4f7f0c13de143f5e5035b21c8e5ee6141e0ea8f2ff461ed7bcb130d9186758b  src/utils/canvasUtils.js
d091f91611cdd5830abd5f29d9e7678442d92ab7503f1f12d46d986b3edfd1bc  src/utils/canvasUtils.test.js
c54fcbeac47ba1b48d214890be806c82d61afe43876fd3c5cb086a6bf7ce0ede  src/utils/searchCoordinator.js
2d4d0af9d499bee1494ec8660d2eddacc93719c4c340433617f8e5ff4cf92071  src/utils/searchCoordinator.test.js
dfd68411a6c06d0de480a552ba25b8e229f80be921cdebd6f3d34a4504b901e9  src/utils/searchResultUtils.js
fdbb194eedb785a539b94cae004ab3a93060545936a63901bcdd2c8d4b5ed920  src/utils/searchResultUtils.test.js
a4710283bb7f89d853cefccf3bc0796730ecb2cfaa5544c99d132fb16f36c136  src/utils/textLayerUtils.js
0cde81667543430881d8d972a391385238f6ac6b0d58ca91b10187066c194d8e  src/utils/textLayerUtils.test.js
38f4ed7a7ec5e5c793b4f3809b8f85876ce2ee232392e56c25c2d076e30370a4  svelte.config.js
5e56f3a26965ba9e7650f1738c894623c4b392a1235fba8b74d56d34816710b7  vite.config.js
ed312b510ec7619712fd40a6d29507745224afc2987ded233cf2ca478165903d  vitest.config.js
```

## Appendix C — Final `git status --porcelain=v2 --branch`

Komut asıl repository'de, salt-okunur olarak çalıştırıldı:

```text
# branch.oid 8a05265c121faab0aba05a76a21be4abc6191b8e
# branch.head main
# branch.upstream origin/main
# branch.ab +0 -0
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 .github/workflows/release.yml
1 .D N... 100644 100644 000000 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 diff.txt
1 .D N... 100644 100644 000000 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 my_diff.txt
1 .M N... 100644 100644 100644 62a71f720ff382dda1685b3a598eeb98431c4a56 62a71f720ff382dda1685b3a598eeb98431c4a56 package-lock.json
1 .M N... 100644 100644 100644 33f6e17fad1f52c60b1ac0672416ab3ba275cef8 33f6e17fad1f52c60b1ac0672416ab3ba275cef8 package.json
1 D. N... 100644 000000 000000 1e2f25f08ac1f888668024df1a7929084196ec14 0000000000000000000000000000000000000000 public/cmaps/README.md
1 D. N... 100644 000000 000000 6a1a1f5b8a1ee92fab3f56d4dd5dd1abed596112 0000000000000000000000000000000000000000 public/standard_fonts/README.md
1 .M N... 100644 100644 100644 287c7a283ad8b0e7f4d6ecf6a9ed329901fc1848 287c7a283ad8b0e7f4d6ecf6a9ed329901fc1848 src-tauri/Cargo.lock
1 .M N... 100644 100644 100644 25d2a18a4f7fe5446736c9625496c0f3af1abdac 25d2a18a4f7fe5446736c9625496c0f3af1abdac src-tauri/Cargo.toml
1 .M N... 100644 100644 100644 9a604f647169bbc22ceae57321aec427fe0795ed 9a604f647169bbc22ceae57321aec427fe0795ed src-tauri/capabilities/default.json
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/fuzz/Cargo.toml
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/fuzz/corpus/minimal.pdf
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/fuzz/dict/pdf.dict
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/fuzz/fuzz_targets/pdfium_lifecycle.rs
1 .M N... 100644 100644 100644 f2f210aeb2a9a6f28fe06b54a955059295ceaeda f2f210aeb2a9a6f28fe06b54a955059295ceaeda src-tauri/gen/schemas/acl-manifests.json
1 .M N... 100644 100644 100644 16bfcd81502ba61635604b731c94cfa79eb70b88 16bfcd81502ba61635604b731c94cfa79eb70b88 src-tauri/gen/schemas/capabilities.json
1 .M N... 100644 100644 100644 24c90014ed68021de21e9f8dd5da66e448fe87d5 24c90014ed68021de21e9f8dd5da66e448fe87d5 src-tauri/gen/schemas/desktop-schema.json
1 .M N... 100644 100644 100644 24c90014ed68021de21e9f8dd5da66e448fe87d5 24c90014ed68021de21e9f8dd5da66e448fe87d5 src-tauri/gen/schemas/macOS-schema.json
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/a11y.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/ai.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/annotations.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/batch.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/dummy_check.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/error.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/export.rs
1 .M N... 100644 100644 100644 041ccef2967be05f506079e23c92336a10c62b6f 041ccef2967be05f506079e23c92336a10c62b6f src-tauri/src/lib.rs
1 .M N... 100644 100644 100644 144ac534b76e538492b7a3b1c55174db1f8a9a59 144ac534b76e538492b7a3b1c55174db1f8a9a59 src-tauri/src/main.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/manipulation.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/ocr.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/pdf_engine/mod.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/pdf_engine/subprocess_manager.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/plugins.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/prepress.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/redaction.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/signature.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/test_annot.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/worker/ipc.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/worker/mod.rs
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/src/worker/pdfium.rs
1 .M N... 100644 100644 100644 733dd67028fb98227d2a90aef48a33238dc49d41 733dd67028fb98227d2a90aef48a33238dc49d41 src-tauri/tauri.conf.json
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src-tauri/tests/crash_tests.rs
1 .M N... 100644 100644 100644 f4feda6c9728cd053d94085e8f9a7cb7ef9a281e f4feda6c9728cd053d94085e8f9a7cb7ef9a281e src/App.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/annotationsStore.js
1 .M N... 100644 100644 100644 ad0d8ca5ba6f2eca09a469c675c1a3f3fc821c97 ad0d8ca5ba6f2eca09a469c675c1a3f3fc821c97 src/app.css
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/A11yPanel.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/AiContextMenu.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/AiSettingsModal.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/AiSidebar.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/AnnotationLayer.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/AutoRedactSidebar.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/DeveloperConsole.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/ExportModal.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/FormLayer.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/MergeSplitModal.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/OcrLayer.svelte
1 .M N... 100644 100644 100644 405fa4b2b390d627fdf408a6037d144de9988f5a 405fa4b2b390d627fdf408a6037d144de9988f5a src/components/OutlineItem.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/PageOrganizer.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/PasswordModal.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/PdfPage.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/PdfXWizard.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/PluginManagerModal.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/PrepressPanel.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/ReadingOrderOverlay.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/RedactionToolbar.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/SanitizationDialog.svelte
1 .M N... 100644 100644 100644 8210c76d5cbfe02f55fdd409009c5e4d26866c5f 8210c76d5cbfe02f55fdd409009c5e4d26866c5f src/components/SearchBar.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/SearchBar.test.js
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/ShortcutsModal.svelte
1 .M N... 100644 100644 100644 ba105d98ad048cfd9c846cd025f7157455d6ea3f ba105d98ad048cfd9c846cd025f7157455d6ea3f src/components/Sidebar.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/SignaturePanel.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/SignatureSettingsModal.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/SignatureWizardModal.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/TagTreeExplorer.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/components/Thumbnail.svelte
1 .M N... 100644 100644 100644 3413ae4beaa958654c4b4f158e1c19860dac6231 3413ae4beaa958654c4b4f158e1c19860dac6231 src/components/ThumbnailItem.svelte
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/configManager.js
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/ipc_profiler.js
1 .M N... 100644 100644 100644 8a909a15a0ebffc001e639084f10d592f0a73ebc 8a909a15a0ebffc001e639084f10d592f0a73ebc src/main.js
1 .M N... 100644 100644 100644 2d9576b70c0f4052768d6007ee56d893f19f9969 2d9576b70c0f4052768d6007ee56d893f19f9969 src/pdfState.js
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 src/pluginStore.js
1 .D N... 100644 100644 000000 ce00d20c66fde72f41a973fdb9c058ac98f180d3 ce00d20c66fde72f41a973fdb9c058ac98f180d3 src/searchWorker.js
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 svelte.config.js
1 .M N... 100644 100644 100644 304e91e82b092acc89654752aac9e39b168c9511 304e91e82b092acc89654752aac9e39b168c9511 vite.config.js
1 .A N... 000000 000000 100644 0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 vitest.config.js
? AGENTS.md
? components.json
? docs/
? jsconfig.json
? src-tauri/pdf_test/
? src/components/PageOrganizer.test.js
? src/components/PdfPage.test.js
? src/components/PdfXWizard.test.js
? src/components/PrepressPanel.test.js
? src/lib/
? src/utils/
```

Exact final untracked dosya yolları `git ls-files --others --exclude-standard`
ile ayrıca sayılmıştır: başlangıçtaki 18 dosya ve üç `docs/engineering/*.md`
dosyası olmak üzere 21.

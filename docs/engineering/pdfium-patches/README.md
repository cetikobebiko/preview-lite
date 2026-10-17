# Yamalı PDFium runtime'ı (macOS arm64)

`src-tauri/lib/libpdfium.dylib`, 2026-10-17'den beri aşağıdaki girdilerle yerelde
build edilen bir PDFium'dur. Amaç, PDFium'un form alanı görünüm akışlarından Türkçe
`ğ Ğ ı İ` harflerini düşürmesini düzeltmektir (bkz. `../form-filling-plan.md`, Faz 0b ve
Seçenek B).

| Girdi | Değer |
|---|---|
| PDFium | `chromium/7961`, commit `dd3583292697a0356cca1f64892892a92f0a2770` |
| Build yamaları | bblanchon/pdfium-binaries `chromium/7961` etiketi (`c6529b58791d`), `steps/03-patch.sh` |
| `args.gn` | `src-tauri/args.gn` ile birebir aynı (bblanchon `steps/05-configure.sh`, mac arm64) |
| Türkçe yama | `turkish-charset.patch` (charset dalı + `kDefaultTTFMap` Türkçe girdisi) |
| SDK | Command Line Tools, macOS SDK 26.1 (tam Xcode yok) |
| CLT uyarlaması | `clt_sdk_patch.py`: Chromium'un `build/config/apple/sdk_info.py` ve `build/mac/find_sdk.py` betiklerine CLT'ye düşen yedek dallar ekler |
| Çıktı | 7 199 216 bayt, SHA-256 `cb167bb40974e409a4de3e37fc97fe29802a33aa0bfb75ce0a4e6c991c7857ab` |

Bu build bayt düzeyinde yeniden üretilebilirliği doğrulanmış değildir. SDK, mutlak yollar ve
zaman damgaları çıktıyı değiştirebilir. Davranışsal doğrulama
`src-tauri/tests/form_fill_spike.rs` ile yapıldı.

## Yeniden build

```bash
W=$HOME/pdfium-build   # ~3,5 GB
mkdir -p "$W" && cd "$W"
git clone --depth 1 https://chromium.googlesource.com/chromium/tools/depot_tools.git
git clone --depth 1 --branch chromium/7961 https://github.com/bblanchon/pdfium-binaries.git binaries
export PATH="$W/depot_tools:$PATH" DEPOT_TOOLS_UPDATE=0 DEPOT_TOOLS_METRICS=0
export PDFium_TARGET_OS=mac PDFium_TARGET_CPU=arm64 PDFium_BRANCH=chromium/7961
export PDFium_SOURCE_DIR="$W/binaries/pdfium" PDFium_BUILD_DIR="$W/binaries/pdfium/out"
cd binaries
bash steps/02-checkout.sh          # gclient sync (~3,3 GB)
bash steps/03-patch.sh             # bblanchon yamaları
python3 -I <repo>/docs/engineering/pdfium-patches/clt_sdk_patch.py pdfium   # yalnız tam Xcode yoksa
(cd pdfium && patch -p1 < <repo>/docs/engineering/pdfium-patches/turkish-charset.patch)
mkdir -p pdfium/out && cp <repo>/src-tauri/args.gn pdfium/out/args.gn
cd pdfium && ./buildtools/mac/gn gen out && ./third_party/ninja/ninja -C out pdfium
```

Çıktı `pdfium/out/libpdfium.dylib`. Kopyaladıktan sonra
`docs/engineering/platform-support.json` (macOS `sha256`, `byteLength`) ile
`scripts/artifact-smoke-macos.mjs`, `scripts/validate-macos-packaging-contract.mjs` ve
`scripts/test-macos-smoke-harness.mjs` içindeki değerler güncellenmelidir.

Ölçülen süreler (8 çekirdek, 8 GB): tam build 272 sn, yalnız `cfx_font.cpp` değiştiğinde 2,3 sn.

## Dosyalar

- `turkish-charset.patch`: runtime'da kullanılan yama.
- `turkish-charset-upstream.patch`, `turkish-charset-cl.md`: upstream `main` için
  hazırlanmış, gönderilmemiş CL taslağı (yalnız charset dalı + birim testi).
- `clt_sdk_patch.py`: CLT ile build için yerel Chromium betik uyarlaması.

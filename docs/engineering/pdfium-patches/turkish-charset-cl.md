# PDFium upstream CL taslağı — Türkçe charset

Durum: **hazır, gönderilmedi.** Diff: `turkish-charset-upstream.patch` (upstream `main`
`9c866a4` üzerine; 2 dosya, +28 satır). Yalnız charset dalı ve birim testi içerir.
`kDefaultTTFMap` değişikliği bilinçli olarak dışarıda bırakıldı (public
`FPDF_GetDefaultTTFMap*` API'sine dokunur).

## Doğrulama (yerel, macOS arm64, `chromium/7961` ağacı)

- `CfxFontTest.GetCharSetFromUnicodeLatinExtendedA`: düzeltme varken geçiyor. Düzeltme geri
  alındığında dört assertion başarısız oluyor (beklenen `0xA2` Türkçe, gerçekleşen `0xEE`
  Doğu Avrupa).
- Tüm `pdfium_unittests`: 1034/1034 geçti.
- Uçtan uca: `form_unicode_spike`, yalnız charset dalını içeren dylib ile çalıştırıldı.
  Türkçe 6 alanın tamamı çiziliyor; PDFium `ArialUnicodeMS_A2` ekliyor (Yunanca'nın bugün
  izlediği yol). Diğer diller değişmedi.
- `main` üzerinde build ve test **yapılmadı**. Değişen bölge `main`'de 7961 ile aynı; diff
  `main` üzerine temiz uygulanıyor ve clang-format temiz. `main` doğrulamasını upstream
  try-bot'ları yapacak.
- Windows ve Linux davranışı ölçülmedi.

## Commit mesajı

```
Map Turkish-only Latin Extended-A letters to the Turkish charset

CFX_Font::GetCharSetFromUnicode() maps all of U+0100..U+024F to
FX_Charset::kMSWin_EasternEuropean. Windows-1250 does not contain
U+011E, U+011F, U+0130 or U+0131, so when CPDF_BAFontMap builds a form
field appearance it adds an Eastern European font that cannot encode
these letters, the universal fallback font is added with the ANSI code
page and cannot encode them either, and the letters are dropped from
the appearance stream. Filling a text field with "Çağrı Yılmaz İstanbul"
produces an appearance that shows "Çar Ylmaz stanbul", while /V keeps
the full value.

Return FX_Charset::kMSWin_Turkish for these four code points, since
Windows-1254 contains them. The rest of Latin Extended-A, including
U+015E and U+015F, which Windows-1250 also has, keeps the Eastern
European charset.

Bug: <issue number, if one is filed>
```

## Bug açıklaması (issues.chromium.org, PDFium bileşeni)

**Başlık:** Turkish letters ğ Ğ ı İ are dropped from generated form field appearances

**Açıklama:**

```
What steps will reproduce the problem?
1. Open a PDF with an AcroForm text field whose /DA uses a standard font,
   e.g. "/Helv 12 Tf 0 g" with /Helv = Helvetica, WinAnsiEncoding.
2. Initialize the form fill environment, focus the field with
   FORM_SetFocusedAnnot(), then FORM_SelectAllText() and
   FORM_ReplaceSelection() with "Çağrı Yılmaz İstanbul".
3. FORM_ForceToKillFocus(), save with FPDF_SaveAsCopy(), and render the
   page or extract the text of the flattened widget.

What is the expected output?
The appearance stream shows "Çağrı Yılmaz İstanbul".

What do you see instead?
The appearance stream shows "Çar Ylmaz stanbul". U+011E, U+011F, U+0130
and U+0131 are silently dropped; /V holds the full value. Polish, French,
Cyrillic, Greek and CJK values in the same setup are drawn correctly.

Analysis:
CFX_Font::GetCharSetFromUnicode() returns kMSWin_EasternEuropean for
U+0100..U+024F. CPDF_BAFontMap::GetWordFontIndex() then tries the /DA
font (charset mismatch), the native Eastern European font (Windows-1250,
no such letters) and "Arial Unicode MS" added with the ANSI code page
(no such letters), and returns -1. A /DR font with Windows-1254
/Differences is not used either, because its charset does not match.
Returning kMSWin_Turkish for these four code points makes the native
font path add a Windows-1254 font, and the letters are drawn.

Version: chromium/7961 (152.0.7961.0), the same code is on main.
Platform: macOS arm64, pdf_enable_v8=false, pdf_enable_xfa=false.
A fix with a unit test is uploaded at <CL link>.
```

## Gönderim adımları

Aşağıdaki adımlar Google hesabı ve hukuki onay gerektirdiği için kullanıcı tarafından
yapılmalıdır:

1. Google Individual CLA'yı imzala: https://cla.developers.google.com/about/google-individual
2. Gerrit'e giriş yap ve depot_tools kimlik doğrulamasını kur
   (https://pdfium-review.googlesource.com; `git credential-luci login` veya
   `.gitcookies`).
3. Commit kimliğini (ad ve CLA'daki e-posta) belirle. İlk CL `AUTHORS` dosyasına
   `Ad Soyad <e-posta>` satırını alfabetik sıraya eklemeli (CONTRIBUTING.md).

Bunlardan sonra commit oluşturulur ve `git cl upload` ile yüklenir. Bug isteğe bağlıdır:
CONTRIBUTING.md'ye göre izole değişiklikler için bug gerekmez.

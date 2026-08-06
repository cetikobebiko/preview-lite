# Antigravity PDF Reader — Proje Kuralları

## Teknoloji Yığını

* Desktop framework: Tauri v2
* Frontend: Svelte 5
* Reaktivite: Svelte 5 Runes
* UI: shadcn-svelte ve Tailwind CSS
* Native katman: Rust
* PDF motoru: PDFium
* Paket yöneticisi: npm
* Test altyapısı: Vitest

Yeni bir npm paketi, Rust crate’i, Tauri plugin’i veya framework eklemeden önce açık onay iste.

## Mimari Sınırlar

### PDF motoru ve yardımcı PDF kütüphaneleri

PDFium uygulamanın kanonik render, text extraction ve page/text geometry motorudur. Kullanıcı PDF’lerine ilişkin session-native işlemler mevcut worker izolasyonu, generation kontrolü, credential güvenliği ve atomic document I/O sınırlarını korumalıdır. Ana Tauri process’inde kullanıcı PDF’ini yeniden açan veya bu sınırları bypass eden yeni production yolları oluşturulamaz.

İkinci bir PDF render motoru eklenemez. PDF.js, MuPDF veya başka bir render motoru production PDF mimarisine yeniden sokulamaz.

`lopdf` bir render motoru değildir. Yalnız açıkça tanımlanmış structural parsing, inspection, serialization veya writing görevlerinde yardımcı kütüphane olarak kullanılabilir. PDFium’un render ve geometry otoritesinin yerine geçirilemez; session/worker izolasyonunu, immutable snapshot kullanımını, credential güvenliğini veya atomic document I/O kurallarını bypass etmek için kullanılamaz. Aktif kullanıcı PDF yolunu doğrudan açan yeni path-native production akışları oluşturmak için kullanılamaz. Yeni `lopdf` kullanımları açık mimari gerekçe, güvenlik değerlendirmesi ve doğrudan test gerektirir.

Aşağıdakileri yeniden ekleme:

* `pdfjs-dist`
* PDF.js worker
* PDF.js TextLayer veya AnnotationLayer
* `PDFDocumentProxy` veya `PDFPageProxy`
* JavaScript tabanlı PDF metin taraması
* `public/cmaps`
* `public/standard_fonts`
* İkinci bir PDF render motoru

Render, thumbnail, metin geometrisi, arama, annotation, form, OCR, prepress, export ve belge manipülasyonu Rust katmanında yürütülmelidir; render, text extraction ve page/text geometry için PDFium kanonik kaynaktır.

Frontend yalnızca şu sorumlulukları taşımalıdır:

* UI state
* Canvas’a RGBA boyama
* DOM metin ve highlight katmanları
* Kullanıcı etkileşimi
* Lazy rendering
* Async istek yaşam döngüsü
* Stale-result koruması

Ana UI thread üzerinde ağır PDF ayrıştırma veya sayfa tarama işlemi yapma.

## Svelte 5 Kuralları

Yeni veya görev kapsamında dönüştürülen bileşenlerde Svelte 5 Runes kullan:

* Reaktif mutable state: `$state`
* Türetilmiş state: `$derived`
* Side effect: `$effect`
* Props: `$props`

Yeni kodda legacy reaktivite kullanma:

```svelte
$: value = ...
export let value;
```

## Doğrulama Kuralları

* Başarısız veya çalıştırılamayan bir doğrulamayı başarılı gibi sunma.


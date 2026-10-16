// antigravity-macos-helper — build.rs tarafından swiftc ile derlenir ve
// Tauri externalBin olarak paketlenir. Çalışma zamanında `swift` yorumlayıcısı
// gerekmez. Yollar ve değerler yalnız süreç argümanı olarak alınır.
//
//   antigravity-macos-helper ocr <görsel-yolu>
//   antigravity-macos-helper share <dosya-yolu> <servis-üst-süresi-sn> [<x> <y> <genişlik> <yükseklik>]
//
// Paylaşımdaki isteğe bağlı dört değer, picker'ın bağlanacağı uygulama
// penceresi içerik alanıdır: birincil ekranın sol üst köşesine göre global
// mantıksal noktalar. Verilmezse picker ekranın ortasında açılır.
//
// Çıkış kodları: 0 başarı, 1 paylaşım hatası, 64 kullanım hatası,
// 65 girdi okunamadı, 69 platform desteklemiyor, 70 Vision hatası.
// Hata ayrıntısı stderr'e yazılır.

import AppKit
import Foundation
import Vision

let usage =
    "Kullanım: antigravity-macos-helper ocr <görsel-yolu> | share <dosya-yolu> <servis-üst-süresi-sn> [<x> <y> <genişlik> <yükseklik>]"

func fail(_ message: String, _ code: Int32) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(code)
}

// MARK: - Vision OCR

func runOCR(imagePath: String) -> Never {
    guard #available(macOS 10.15, *) else {
        fail("Vision metin tanıma macOS 10.15 veya üstünü gerektirir", 69)
    }
    let imageURL = URL(fileURLWithPath: imagePath)
    guard let image = NSImage(contentsOf: imageURL),
          let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
        fail("OCR görseli okunamadı", 65)
    }

    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
    do {
        try handler.perform([request])
    } catch {
        fail("Vision metin tanıma başarısız: \(error.localizedDescription)", 70)
    }
    let observations = request.results ?? []
    print(observations.compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n"))
    exit(0)
}

// MARK: - Share picker

/// Picker delegesi `delegateFor` ile seçilen servise kendisini atar; süreç
/// (dolayısıyla paylaşılan dosya) servis `didShareItems` veya
/// `didFailToShareItems` bildirene kadar yaşar. Picker servis seçilmeden
/// kapanırsa hemen çıkılır. Delegeyi hiç çağırmayan servislere karşı servis
/// seçiminden sonra `serviceTimeout` saniyelik üst süre uygulanır.
final class ShareCoordinator: NSObject, NSSharingServicePickerDelegate, NSSharingServiceDelegate {
    private let serviceTimeout: Double

    init(serviceTimeout: Double) {
        self.serviceTimeout = serviceTimeout
    }

    func sharingServicePicker(
        _ sharingServicePicker: NSSharingServicePicker,
        delegateFor sharingService: NSSharingService
    ) -> NSSharingServiceDelegate? {
        return self
    }

    func sharingServicePicker(
        _ sharingServicePicker: NSSharingServicePicker,
        didChoose service: NSSharingService?
    ) {
        guard service != nil else {
            exit(0)
        }
        let timeout = serviceTimeout
        DispatchQueue.main.asyncAfter(deadline: .now() + timeout) {
            FileHandle.standardError.write(
                Data("Paylaşım servisi \(Int(timeout)) sn içinde sonuç bildirmedi\n".utf8))
            exit(0)
        }
    }

    func sharingService(_ sharingService: NSSharingService, didShareItems items: [Any]) {
        exit(0)
    }

    func sharingService(
        _ sharingService: NSSharingService,
        didFailToShareItems items: [Any],
        error: Error
    ) {
        let nsError = error as NSError
        if nsError.domain == NSCocoaErrorDomain && nsError.code == NSUserCancelledError {
            exit(0)
        }
        fail("Paylaşım başarısız: \(error.localizedDescription)", 1)
    }
}

/// Uygulama penceresi içerik alanı; sol üst orijinli global mantıksal noktalar.
struct ShareAnchor {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
}

/// Dört sonlu değer bekler; genişlik ve yükseklik pozitif olmalıdır.
func parseAnchor(_ values: [String]) -> ShareAnchor? {
    let numbers = values.compactMap(Double.init)
    guard values.count == 4, numbers.count == 4, numbers.allSatisfy({ $0.isFinite }),
          numbers[2] > 0, numbers[3] > 0 else {
        return nil
    }
    return ShareAnchor(x: numbers[0], y: numbers[1], width: numbers[2], height: numbers[3])
}

/// Picker okunun göstereceği nokta (AppKit global koordinatı, sol alt orijin).
/// Pencere verildiyse içerik alanının üst kenarının ortası; picker oradan
/// aşağı doğru açılır. Sol üst orijinden çevirme birincil ekranın (menü
/// çubuğunun bulunduğu, orijini 0,0 olan) yüksekliğiyle yapılır.
func pickerAnchorPoint(_ anchor: ShareAnchor?) -> NSPoint {
    let topInset = 8.0
    if let anchor = anchor, let primary = NSScreen.screens.first {
        let contentTop = primary.frame.maxY - anchor.y
        return NSPoint(
            x: anchor.x + anchor.width / 2,
            y: contentTop - min(topInset, anchor.height / 2))
    }
    let visible = (NSScreen.main ?? NSScreen.screens.first)?.visibleFrame ?? .zero
    return NSPoint(x: visible.midX, y: visible.midY)
}

func runShare(filePath: String, serviceTimeout: Double, anchor: ShareAnchor?) -> Never {
    let fileURL = URL(fileURLWithPath: filePath)
    guard FileManager.default.isReadableFile(atPath: fileURL.path) else {
        fail("Paylaşılacak dosya okunamadı", 65)
    }
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)
    let coordinator = ShareCoordinator(serviceTimeout: serviceTimeout)
    let picker = NSSharingServicePicker(items: [fileURL])
    picker.delegate = coordinator
    let point = pickerAnchorPoint(anchor)
    // Picker'ı konumlandırmak için görünmez 1x1 pencere; tam ekran uygulama
    // Space'inde de görünebilmesi için tüm Space'lere katılır.
    let window = NSWindow(
        contentRect: NSRect(x: point.x - 0.5, y: point.y - 0.5, width: 1, height: 1),
        styleMask: .borderless,
        backing: .buffered,
        defer: false
    )
    window.isOpaque = false
    window.backgroundColor = .clear
    window.hasShadow = false
    window.ignoresMouseEvents = true
    window.level = .floating
    window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
    window.makeKeyAndOrderFront(nil)
    app.activate(ignoringOtherApps: true)
    picker.show(relativeTo: .zero, of: window.contentView!, preferredEdge: .minY)
    // picker.delegate zayıf referanstır; optimizasyonlu derlemede nesneler
    // run loop süresince açıkça canlı tutulur.
    withExtendedLifetime((coordinator, picker, window)) {
        app.run()
    }
    exit(0)
}

// MARK: - Entry point

let arguments = CommandLine.arguments
guard arguments.count >= 2 else {
    fail(usage, 64)
}
switch arguments[1] {
case "ocr" where arguments.count == 3:
    runOCR(imagePath: arguments[2])
case "share" where arguments.count == 4 || arguments.count == 8:
    guard let serviceTimeout = Double(arguments[3]), serviceTimeout > 0 else {
        fail(usage, 64)
    }
    var anchor: ShareAnchor? = nil
    if arguments.count == 8 {
        guard let parsed = parseAnchor(Array(arguments[4...])) else {
            fail(usage, 64)
        }
        anchor = parsed
    }
    runShare(filePath: arguments[2], serviceTimeout: serviceTimeout, anchor: anchor)
default:
    fail(usage, 64)
}

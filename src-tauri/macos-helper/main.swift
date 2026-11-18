// antigravity-macos-helper — build.rs tarafından swiftc ile derlenir ve
// Tauri externalBin olarak paketlenir. Çalışma zamanında `swift` yorumlayıcısı
// gerekmez. Yollar ve değerler yalnız süreç argümanı olarak alınır.
//
//   antigravity-macos-helper ocr <görsel-yolu>
//   antigravity-macos-helper share <dosya-yolu> <servis-üst-süresi-sn> [<x> <y> <genişlik> <yükseklik>]
//   antigravity-macos-helper scan <çıktı-dizini> <bekleme-üst-süresi-sn> [<x> <y> <genişlik> <yükseklik>]
//   antigravity-macos-helper normalize-pasteboard <girdi-dosyası> <uti> <çıktı-dizini>
//
// Paylaşım ve taramadaki isteğe bağlı dört değer, picker'ın/menünün bağlanacağı
// uygulama penceresi içerik alanıdır: birincil ekranın sol üst köşesine göre
// global mantıksal noktalar. Verilmezse ekranın ortasında açılır.
//
// `scan` Continuity Camera menüsünü (iPhone/iPad ile Fotoğraf Çek, Belgeleri
// Tara, Çizim Ekle) açar, gelen pasteboard içeriğini normalleştirip yalnız
// verilen çıktı dizinine `scan.pdf` ya da `scan-NNN.jpg|png` olarak yazar ve
// stdout'a tek satırlık JSON bildirir: {"kind":"pdf"|"images","files":[...]}.
// `normalize-pasteboard` aynı normalleştirmeyi cihazsız sınamak için bir
// dosyanın baytlarını özel bir pasteboard'a verilen UTI ile koyup çalıştırır.
//
// Çıkış kodları: 0 başarı, 1 paylaşım hatası, 2 tarama iptal edildi,
// 64 kullanım hatası, 65 girdi/çıktı dizini geçersiz, 66 desteklenmeyen içerik,
// 68 Continuity Camera cihazı yok, 69 platform desteklemiyor, 70 Vision hatası,
// 73 sınır aşıldı ya da yazılamadı, 75 zaman aşımı.
// Hata ayrıntısı stderr'e yazılır.

import AppKit
import Foundation
import ImageIO
import Vision

let usage =
    "Kullanım: antigravity-macos-helper ocr <görsel-yolu> | share <dosya-yolu> <servis-üst-süresi-sn> [<x> <y> <genişlik> <yükseklik>] | scan <çıktı-dizini> <bekleme-üst-süresi-sn> [<x> <y> <genişlik> <yükseklik>] | normalize-pasteboard <girdi-dosyası> <uti> <çıktı-dizini>"

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

// MARK: - Continuity Camera scan

/// Bir taramada en fazla sayfa/görsel.
let scanMaxPages = 100
/// Görselin uzun kenarı (piksel); daha büyükleri orantılı küçültülür.
let scanMaxPixelSize = 8192
/// Tek çıktı dosyasının ve tüm çıktının bayt sınırları.
let scanMaxFileBytes = 256 * 1024 * 1024
let scanMaxTotalBytes = 512 * 1024 * 1024
/// Menü açıkken seçim yapılmadan geçebilecek en uzun süre (sn).
let scanMenuIdleTimeout: Double = 120
/// AppKit'in menüye cihaz eylemlerini eklemesi için tanınan süre (sn); dolduğunda
/// eylem yoksa cihaz yok sayılır (68).
let scanDevicePopulationTimeout: Double = 2
/// Yardımcının etkin uygulama olmasını bekleme üst süresi (sn); dolunca menü yine açılır.
let scanActivationTimeout: Double = 1

/// Belge taraması PDF olarak gelirse o tercih edilir; görseller sırayla denenir.
let scanImageTypes: [NSPasteboard.PasteboardType] = [
    NSPasteboard.PasteboardType("public.jpeg"),
    NSPasteboard.PasteboardType("public.heic"),
    .png,
    .tiff,
]
let scanReturnTypes: [NSPasteboard.PasteboardType] = [.pdf] + scanImageTypes

struct ScanFailure: Error {
    let message: String
    let code: Int32
    init(_ message: String, _ code: Int32) {
        self.message = message
        self.code = code
    }
}

struct ScanImage {
    let data: Data
    let fileExtension: String
}

enum ScanOutput {
    case pdf(Data)
    case images([ScanImage])
}

/// Pasteboard içeriğini çıktıya çevirir: PDF olduğu gibi kalır; görseller
/// (HEIC/JPEG/PNG/TIFF, çok sayfalı TIFF dahil) EXIF yönü uygulanıp
/// saydamlık varsa PNG, yoksa JPEG olarak yeniden kodlanır. Böylece Rust
/// tarafı HEIC çözmek zorunda kalmaz.
func normalizeScanPasteboard(_ pasteboard: NSPasteboard) throws -> ScanOutput {
    let types = pasteboard.types ?? []
    if types.contains(.pdf), let data = pasteboard.data(forType: .pdf) {
        guard data.starts(with: Array("%PDF-".utf8)) else {
            throw ScanFailure("Gelen PDF geçerli bir PDF başlığı taşımıyor", 66)
        }
        guard data.count <= scanMaxFileBytes else {
            throw ScanFailure("Taranan PDF boyut sınırını aşıyor", 73)
        }
        return .pdf(data)
    }
    for type in scanImageTypes where types.contains(type) {
        guard let data = pasteboard.data(forType: type) else {
            continue
        }
        return .images(try normalizeScanImages(data))
    }
    throw ScanFailure("Continuity Camera desteklenmeyen bir içerik gönderdi", 66)
}

func normalizeScanImages(_ data: Data) throws -> [ScanImage] {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil) else {
        throw ScanFailure("Gelen görsel okunamadı", 66)
    }
    let count = CGImageSourceGetCount(source)
    guard count > 0 else {
        throw ScanFailure("Gelen görsel boş", 66)
    }
    guard count <= scanMaxPages else {
        throw ScanFailure("Tarama en fazla \(scanMaxPages) sayfa olabilir", 73)
    }
    let options: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: scanMaxPixelSize,
        kCGImageSourceShouldCacheImmediately: true,
    ]
    var images: [ScanImage] = []
    var totalBytes = 0
    for index in 0..<count {
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, options as CFDictionary) else {
            throw ScanFailure("Gelen görsel çözülemedi", 66)
        }
        let opaque = [CGImageAlphaInfo.none, .noneSkipFirst, .noneSkipLast].contains(image.alphaInfo)
        let (typeIdentifier, fileExtension) = opaque ? ("public.jpeg", "jpg") : ("public.png", "png")
        let buffer = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            buffer, typeIdentifier as CFString, 1, nil)
        else {
            throw ScanFailure("Görsel kodlayıcısı oluşturulamadı", 73)
        }
        let properties: [CFString: Any] =
            opaque ? [kCGImageDestinationLossyCompressionQuality: 0.9] : [:]
        CGImageDestinationAddImage(destination, image, properties as CFDictionary)
        guard CGImageDestinationFinalize(destination) else {
            throw ScanFailure("Görsel kodlanamadı", 73)
        }
        let encoded = buffer as Data
        totalBytes += encoded.count
        guard encoded.count <= scanMaxFileBytes, totalBytes <= scanMaxTotalBytes else {
            throw ScanFailure("Taranan görseller boyut sınırını aşıyor", 73)
        }
        images.append(ScanImage(data: encoded, fileExtension: fileExtension))
    }
    return images
}

/// Çıktı dizini mutlak, var olan, symlink olmayan ve boş bir dizin olmalıdır.
func validateScanOutputDirectory(_ path: String) throws -> URL {
    guard path.hasPrefix("/") else {
        throw ScanFailure("Tarama çıktı dizini mutlak yol olmalıdır", 65)
    }
    var info = stat()
    guard lstat(path, &info) == 0 else {
        throw ScanFailure("Tarama çıktı dizini bulunamadı", 65)
    }
    guard (info.st_mode & S_IFMT) == S_IFDIR else {
        throw ScanFailure("Tarama çıktı yolu bir dizin değil (symlink kabul edilmez)", 65)
    }
    let contents: [String]
    do {
        contents = try FileManager.default.contentsOfDirectory(atPath: path)
    } catch {
        throw ScanFailure("Tarama çıktı dizini okunamadı: \(error.localizedDescription)", 65)
    }
    guard contents.isEmpty else {
        throw ScanFailure("Tarama çıktı dizini boş değil", 65)
    }
    return URL(fileURLWithPath: path, isDirectory: true)
}

/// Dosyayı yalnız yeni oluşturarak (O_EXCL, symlink izlemeden) yazar.
func writeScanFile(_ data: Data, named name: String, in directory: URL) throws {
    let path = directory.appendingPathComponent(name).path
    let descriptor = open(path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
    guard descriptor >= 0 else {
        throw ScanFailure("\(name) oluşturulamadı: \(String(cString: strerror(errno)))", 73)
    }
    defer { close(descriptor) }
    let written = data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) -> Bool in
        guard let base = raw.baseAddress else {
            return true
        }
        var offset = 0
        while offset < raw.count {
            let result = Darwin.write(descriptor, base.advanced(by: offset), raw.count - offset)
            if result < 0 {
                if errno == EINTR {
                    continue
                }
                return false
            }
            offset += result
        }
        return true
    }
    guard written, fsync(descriptor) == 0 else {
        throw ScanFailure("\(name) yazılamadı: \(String(cString: strerror(errno)))", 73)
    }
}

/// Çıktıyı sabit adlarla yazar ve adları döndürür.
func writeScanOutput(_ output: ScanOutput, to directory: URL) throws -> (kind: String, files: [String]) {
    switch output {
    case .pdf(let data):
        try writeScanFile(data, named: "scan.pdf", in: directory)
        return ("pdf", ["scan.pdf"])
    case .images(let images):
        var names: [String] = []
        for (index, image) in images.enumerated() {
            let name = String(format: "scan-%03d", index + 1) + "." + image.fileExtension
            try writeScanFile(image.data, named: name, in: directory)
            names.append(name)
        }
        return ("images", names)
    }
}

/// Normalleştirme sonucunu yazar, stdout'a tek satır JSON bildirir ve çıkar.
/// Adlar sabit kalıpta (ASCII) olduğundan JSON elle kurulur.
func finishScan(_ result: Result<ScanOutput, Error>, directory: URL) -> Never {
    do {
        let written = try writeScanOutput(try result.get(), to: directory)
        let files = written.files.map { "\"\($0)\"" }.joined(separator: ",")
        print("{\"kind\":\"\(written.kind)\",\"files\":[\(files)]}")
        exit(0)
    } catch let failure as ScanFailure {
        fail(failure.message, failure.code)
    } catch {
        fail("Tarama çıktısı yazılamadı: \(error.localizedDescription)", 73)
    }
}

func scanOutputDirectoryOrFail(_ path: String) -> URL {
    do {
        return try validateScanOutputDirectory(path)
    } catch let failure as ScanFailure {
        fail(failure.message, failure.code)
    } catch {
        fail("Tarama çıktı dizini doğrulanamadı", 65)
    }
}

/// Continuity Camera eylemlerinin dönüşünü alan görünmez responder.
final class ScanRequestorView: NSView, NSServicesMenuRequestor {
    var onPasteboard: ((NSPasteboard) -> Void)?

    override var acceptsFirstResponder: Bool { true }

    override func validRequestor(
        forSendType sendType: NSPasteboard.PasteboardType?,
        returnType: NSPasteboard.PasteboardType?
    ) -> Any? {
        if sendType == nil, let returnType = returnType, scanReturnTypes.contains(returnType) {
            return self
        }
        return super.validRequestor(forSendType: sendType, returnType: returnType)
    }

    func readSelection(from pboard: NSPasteboard) -> Bool {
        onPasteboard?(pboard)
        return true
    }
}

/// Borderless pencereler varsayılan olarak key olamaz; responder zinciri için gerekir.
final class ScanAnchorWindow: NSWindow {
    override var canBecomeKey: Bool { true }
}

/// Menüde eylem içeren (cihaz eklemiş) etkin bir öğe var mı.
func menuHasDeviceActions(_ menu: NSMenu) -> Bool {
    return menu.items.contains { item in
        if item.isSeparatorItem || item.isHidden || !item.isEnabled {
            return false
        }
        if item.action != nil {
            return true
        }
        return item.submenu.map(menuHasDeviceActions) ?? false
    }
}

/// Menü yaşam döngüsü: cihaz yoksa 68, seçim yapılmadan kapanırsa 2, menü
/// boşta kalırsa ya da seçimden sonra içerik `waitTimeout` içinde gelmezse 75.
/// Seçilen öğenin eylemi menü kapandıktan sonra gönderilir; seçim, son
/// vurgulanan etkin öğe ya da `NSMenu.didSendActionNotification` ile anlaşılır.
/// Esc ile kapanmadan önce bir öğe vurgulanmışsa bekleme sürer; kullanıcı
/// uygulamadaki İptal düğmesiyle çıkabilir.
final class ScanMenuController: NSObject, NSMenuDelegate {
    let menu = NSMenu()
    private let waitTimeout: Double
    private var lastHighlighted: NSMenuItem?
    private var actionSent = false
    private var noDevice = false
    private var menuIdleExpired = false
    private var tracking = false

    init(waitTimeout: Double) {
        self.waitTimeout = waitTimeout
        super.init()
        menu.delegate = self
        menu.autoenablesItems = true
        let item = NSMenuItem(title: "", action: nil, keyEquivalent: "")
        if #available(macOS 10.14, *) {
            item.identifier = NSMenuItem.importFromDeviceIdentifier
        }
        menu.addItem(item)
        NotificationCenter.default.addObserver(
            self, selector: #selector(actionWasSent(_:)),
            name: NSMenu.didSendActionNotification, object: nil)
    }

    @objc func actionWasSent(_ notification: Notification) {
        actionSent = true
    }

    func menu(_ menu: NSMenu, willHighlight item: NSMenuItem?) {
        lastHighlighted = item
    }

    func menuWillOpen(_ menu: NSMenu) {
        guard menu === self.menu, !tracking else {
            return
        }
        tracking = true
        let population = Timer(timeInterval: scanDevicePopulationTimeout, repeats: false) { [weak self] _ in
            guard let self = self, self.tracking, !menuHasDeviceActions(self.menu) else {
                return
            }
            self.noDevice = true
            self.menu.cancelTracking()
        }
        let idle = Timer(timeInterval: scanMenuIdleTimeout, repeats: false) { [weak self] _ in
            guard let self = self, self.tracking else {
                return
            }
            self.menuIdleExpired = true
            self.menu.cancelTracking()
        }
        RunLoop.main.add(population, forMode: .common)
        RunLoop.main.add(idle, forMode: .common)
    }

    func menuDidClose(_ menu: NSMenu) {
        guard menu === self.menu else {
            return
        }
        tracking = false
        DispatchQueue.main.async { [weak self] in
            self?.decideAfterClose()
        }
    }

    private func decideAfterClose() {
        if noDevice {
            fail("Continuity Camera için uygun bir iPhone veya iPad bulunamadı", 68)
        }
        if menuIdleExpired {
            fail("Continuity Camera menüsünde \(Int(scanMenuIdleTimeout)) sn içinde seçim yapılmadı", 75)
        }
        let highlightedAction = lastHighlighted.map {
            $0.isEnabled && ($0.action != nil || $0.submenu != nil)
        } ?? false
        guard actionSent || highlightedAction else {
            fail("Continuity Camera menüsü seçim yapılmadan kapandı", 2)
        }
        let timeout = waitTimeout
        DispatchQueue.main.asyncAfter(deadline: .now() + timeout) {
            fail("iPhone veya iPad'den \(Int(timeout)) sn içinde içerik gelmedi", 75)
        }
    }
}

func runScan(outputDirectory: String, waitTimeout: Double, anchor: ShareAnchor?) -> Never {
    guard #available(macOS 10.14, *) else {
        fail("Continuity Camera macOS 10.14 veya üstünü gerektirir", 69)
    }
    let directory = scanOutputDirectoryOrFail(outputDirectory)
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)
    app.registerServicesMenuSendTypes([], returnTypes: scanReturnTypes)

    // Ana süreç ölürse (ör. zorla kapatma) menü ve bekleme açık kalmasın.
    let parent = getppid()
    let parentWatch = Timer(timeInterval: 1, repeats: true) { _ in
        if getppid() != parent {
            fail("Ana uygulama kapandı", 75)
        }
    }
    RunLoop.main.add(parentWatch, forMode: .common)

    let point = pickerAnchorPoint(anchor)
    let window = ScanAnchorWindow(
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
    let view = ScanRequestorView(frame: NSRect(x: 0, y: 0, width: 1, height: 1))
    view.onPasteboard = { pasteboard in
        let result = Result { try normalizeScanPasteboard(pasteboard) }
        finishScan(result, directory: directory)
    }
    window.contentView = view

    let controller = ScanMenuController(waitTimeout: waitTimeout)
    // Menü yardımcı etkin uygulama olduktan sonra açılır: etkinlik menü açıkken
    // değişirse AppKit takibi bitirir ve menü seçimsiz kapanmış görünür.
    // Bildirim gelmezse menü en geç `scanActivationTimeout` sonra açılır.
    var menuShown = false
    let showMenu = {
        guard !menuShown else {
            return
        }
        menuShown = true
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(view)
        DispatchQueue.main.async {
            _ = controller.menu.popUp(positioning: nil, at: .zero, in: view)
        }
    }
    let activation = NotificationCenter.default.addObserver(
        forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
    ) { _ in
        showMenu()
    }
    window.makeKeyAndOrderFront(nil)
    app.activate(ignoringOtherApps: true)
    DispatchQueue.main.asyncAfter(deadline: .now() + scanActivationTimeout) {
        showMenu()
    }
    // Delegeler zayıf referanstır; optimizasyonlu derlemede nesneler run loop
    // süresince açıkça canlı tutulur.
    withExtendedLifetime((controller, window, view, parentWatch, activation)) {
        app.run()
    }
    exit(0)
}

/// Cihazsız sınama: `inputPath` baytlarını özel bir pasteboard'a `typeIdentifier`
/// ile koyar ve taramayla aynı normalleştirme/yazma yolunu çalıştırır.
func runNormalizePasteboard(inputPath: String, typeIdentifier: String, outputDirectory: String) -> Never {
    let directory = scanOutputDirectoryOrFail(outputDirectory)
    let inputURL = URL(fileURLWithPath: inputPath)
    let data: Data
    do {
        let size = try inputURL.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        guard size <= scanMaxFileBytes else {
            fail("Girdi dosyası boyut sınırını aşıyor", 73)
        }
        data = try Data(contentsOf: inputURL)
    } catch {
        fail("Girdi dosyası okunamadı", 65)
    }
    let pasteboard = NSPasteboard(
        name: NSPasteboard.Name("com.antigravity.scan-normalize.\(UUID().uuidString)"))
    pasteboard.clearContents()
    pasteboard.setData(data, forType: NSPasteboard.PasteboardType(typeIdentifier))
    let result = Result { try normalizeScanPasteboard(pasteboard) }
    pasteboard.releaseGlobally()
    finishScan(result, directory: directory)
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
case "scan" where arguments.count == 4 || arguments.count == 8:
    guard let waitTimeout = Double(arguments[3]), waitTimeout.isFinite, waitTimeout > 0 else {
        fail(usage, 64)
    }
    var anchor: ShareAnchor? = nil
    if arguments.count == 8 {
        guard let parsed = parseAnchor(Array(arguments[4...])) else {
            fail(usage, 64)
        }
        anchor = parsed
    }
    runScan(outputDirectory: arguments[2], waitTimeout: waitTimeout, anchor: anchor)
case "normalize-pasteboard" where arguments.count == 5:
    runNormalizePasteboard(
        inputPath: arguments[2], typeIdentifier: arguments[3], outputDirectory: arguments[4])
default:
    fail(usage, 64)
}

import AppKit
import EivizMixer
import SwiftUI

final class ThumbImageView: NSView {
    var autoSizeFromBounds = false

    private let imageView = NSImageView()
    private var sourceId: UInt64 = 0
    private var width: UInt32 = 176
    private var height: UInt32 = 90
    private var interval: UInt32 = 3
    private var wanted = false
    private var subscribed = false
    private var scratch = [UInt8](repeating: 0, count: 960 * 540 * 4)
    private var lastBytes = 0
    private var lastW: UInt32 = 0
    private var lastH: UInt32 = 0
    private var lastHash: UInt64 = 0

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor
        imageView.imageScaling = .scaleProportionallyUpOrDown
        imageView.imageAlignment = .alignCenter
        imageView.animates = false
        imageView.frame = bounds
        imageView.autoresizingMask = [.width, .height]
        addSubview(imageView)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    deinit {
        ThumbPump.unregister(self)
        if sourceId != 0 {
            ThumbSubscriptions.release(self, sourceId: sourceId)
        }
    }

    override func layout() {
        super.layout()
        imageView.frame = bounds
        guard autoSizeFromBounds, sourceId != 0 else { return }
        let scale = window?.backingScaleFactor ?? 2
        let w = UInt32(min(960, max(2, (bounds.width * scale).rounded())))
        let h = UInt32(min(540, max(2, (bounds.height * scale).rounded())))
        bind(sourceId: sourceId, width: w, height: h, interval: interval)
    }

    func bind(sourceId: UInt64, width: UInt32, height: UInt32, interval: UInt32) {
        let w = min(960, max(2, width))
        let h = min(540, max(2, height))
        let frames = min(8, max(1, interval))
        if self.sourceId == sourceId, self.width == w, self.height == h, self.interval == frames, subscribed {
            return
        }
        let previous = self.sourceId
        self.sourceId = sourceId
        self.width = w
        self.height = h
        self.interval = frames
        if wanted {
            subscribe()
        } else if previous != 0, previous != sourceId {
            ThumbSubscriptions.release(self, sourceId: previous)
        }
    }

    func setWanted(_ wanted: Bool) {
        if self.wanted == wanted, subscribed == wanted {
            return
        }
        self.wanted = wanted
        if wanted {
            subscribe()
        } else {
            unsubscribe()
        }
    }

    func poll() {
        guard subscribed, sourceId != 0 else { return }
        var w: UInt32 = 0
        var h: UInt32 = 0
        var stride: UInt32 = 0
        let n = scratch.withUnsafeMutableBufferPointer { ptr in
            mixer_thumb_read(sourceId, ptr.baseAddress, ptr.count, &w, &h, &stride)
        }
        guard n > 0, w > 0, h > 0 else { return }
        let hash = Self.hashPixels(scratch, length: Int(n))
        if lastBytes == Int(n), lastW == w, lastH == h, lastHash == hash, imageView.image != nil {
            return
        }
        lastBytes = Int(n)
        lastW = w
        lastH = h
        lastHash = hash
        guard let image = Self.makeImage(bytes: scratch, count: Int(n), width: w, height: h, stride: stride) else {
            return
        }
        imageView.image = image
    }

    private func subscribe() {
        guard sourceId != 0, width > 0, height > 0 else { return }
        ThumbSubscriptions.retain(self, sourceId: sourceId, width: width, height: height, interval: interval)
        subscribed = true
        ThumbPump.register(self)
    }

    private func unsubscribe() {
        ThumbPump.unregister(self)
        if sourceId != 0 {
            ThumbSubscriptions.release(self, sourceId: sourceId)
        }
        subscribed = false
    }

    private static func hashPixels(_ data: [UInt8], length: Int) -> UInt64 {
        var hash: UInt64 = 14_695_981_039_346_656_037
        let step = max(1, length / 64)
        var i = 0
        while i < length {
            hash = (hash ^ UInt64(data[i])) &* 1_099_511_628_211
            i += step
        }
        if length > 0 {
            hash ^= UInt64(data[length - 1])
        }
        return hash ^ UInt64(UInt32(truncatingIfNeeded: length))
    }

    private static func makeImage(
        bytes: [UInt8],
        count: Int,
        width: UInt32,
        height: UInt32,
        stride: UInt32
    ) -> NSImage? {
        let row = Int(stride)
        guard row > 0, count >= row * Int(height) else { return nil }
        let data = Data(bytes.prefix(count))
        guard let provider = CGDataProvider(data: data as CFData) else { return nil }
        let bitmapInfo = CGBitmapInfo(rawValue: CGBitmapInfo.byteOrder32Little.rawValue | CGImageAlphaInfo.premultipliedFirst.rawValue)
        guard let cgImage = CGImage(
            width: Int(width),
            height: Int(height),
            bitsPerComponent: 8,
            bitsPerPixel: 32,
            bytesPerRow: row,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: bitmapInfo,
            provider: provider,
            decode: nil,
            shouldInterpolate: false,
            intent: .defaultIntent
        ) else { return nil }
        return NSImage(cgImage: cgImage, size: NSSize(width: CGFloat(width), height: CGFloat(height)))
    }
}

enum ThumbSubscriptions {
    private struct Sub {
        let view: ObjectIdentifier
        let width: UInt32
        let height: UInt32
        let interval: UInt32
    }

    private static var live: [UInt64: [Sub]] = [:]

    static func retain(
        _ view: ThumbImageView,
        sourceId: UInt64,
        width: UInt32,
        height: UInt32,
        interval: UInt32
    ) {
        let id = ObjectIdentifier(view)
        var stale: [UInt64] = []
        for (existingId, list) in live where existingId != sourceId {
            let next = list.filter { $0.view != id }
            if next.count != list.count {
                live[existingId] = next
                stale.append(existingId)
            }
        }
        for item in stale {
            push(item)
        }
        var current = live[sourceId] ?? []
        let same = current.contains {
            $0.view == id && $0.width == width && $0.height == height && $0.interval == interval
        }
        current.removeAll { $0.view == id }
        current.append(Sub(view: id, width: width, height: height, interval: interval))
        live[sourceId] = current
        if !same {
            push(sourceId)
        }
    }

    static func release(_ view: ThumbImageView, sourceId: UInt64) {
        let id = ObjectIdentifier(view)
        guard var current = live[sourceId] else { return }
        current.removeAll { $0.view == id }
        live[sourceId] = current
        push(sourceId)
        if current.isEmpty {
            live.removeValue(forKey: sourceId)
        }
    }

    private static func push(_ sourceId: UInt64) {
        guard let current = live[sourceId], !current.isEmpty else {
            _ = mixer_thumb_set(sourceId, 0, 0, 0)
            return
        }
        var width: UInt32 = 2
        var height: UInt32 = 2
        var interval: UInt32 = 8
        for item in current {
            width = max(width, item.width)
            height = max(height, item.height)
            interval = min(interval, item.interval)
        }
        _ = mixer_thumb_set(sourceId, width, height, interval)
    }
}

enum ThumbPump {
    private struct WeakThumb {
        weak var view: ThumbImageView?
    }

    private static var views: [ObjectIdentifier: WeakThumb] = [:]

    static func register(_ view: ThumbImageView) {
        views[ObjectIdentifier(view)] = WeakThumb(view: view)
    }

    static func unregister(_ view: ThumbImageView) {
        views.removeValue(forKey: ObjectIdentifier(view))
    }

    static func poll() {
        var dead: [ObjectIdentifier] = []
        for (id, item) in views {
            if let view = item.view {
                view.poll()
            } else {
                dead.append(id)
            }
        }
        for id in dead {
            views.removeValue(forKey: id)
        }
    }
}

final class ThumbHostView: NSView {
    var onClick: (() -> Void)?

    override var isOpaque: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.black.setFill()
        bounds.fill()
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseUp(with event: NSEvent) {
        onClick?()
    }
}

struct ThumbRepresentable: NSViewRepresentable {
    let sourceId: UInt64
    var width: UInt32 = 176
    var height: UInt32 = 90
    var interval: UInt32 = 3
    var wanted = false
    var onClick: (() -> Void)?

    func makeNSView(context: Context) -> ThumbHostView {
        let host = ThumbHostView(frame: .zero)
        host.onClick = onClick
        let thumb = ThumbImageView(frame: .zero)
        thumb.autoresizingMask = [.width, .height]
        host.addSubview(thumb)
        thumb.bind(sourceId: sourceId, width: width, height: height, interval: interval)
        thumb.setWanted(wanted)
        return host
    }

    func updateNSView(_ nsView: ThumbHostView, context: Context) {
        nsView.onClick = onClick
        guard let thumb = nsView.subviews.first as? ThumbImageView else { return }
        if thumb.frame != nsView.bounds {
            thumb.frame = nsView.bounds
        }
        thumb.bind(sourceId: sourceId, width: width, height: height, interval: interval)
        thumb.setWanted(wanted)
    }
}

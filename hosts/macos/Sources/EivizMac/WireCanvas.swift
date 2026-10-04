import AppKit
import SwiftUI

struct WireRect<ID: Hashable>: Identifiable, Equatable {
    var id: ID
    var x: Float
    var y: Float
    var width: Float
    var height: Float
    var enabled: Bool = true
    var locked: Bool = false
    var sizeLinked: Bool = false
    var cropX: Float = 0
    var cropY: Float = 0
    var cropWidth: Float = 1
    var cropHeight: Float = 1
}

struct WireCanvasView<ID: Hashable>: View {
    var items: [WireRect<ID>]
    var aspect: CGFloat = 16.0 / 9.0
    var snapEnabled = false
    var onFit: ((ID) -> Void)?
    var onCrop: ((ID, Float, Float, Float, Float, Bool) -> Void)?
    @Binding var selected: ID?
    var onChange: (ID, Float, Float, Float, Float, Bool) -> Void
    var camera: SceneCamera? = nil
    var onCamera: ((SceneCamera, Bool) -> Void)? = nil
    /// 1 fills the canvas with the frame. Smaller values show layers outside it.
    var viewZoom: CGFloat = 1

    @State private var dragging = false
    @State private var resizing = false
    @State private var cropping = false
    @State private var cropLeft = false
    @State private var cropRight = false
    @State private var cropUp = false
    @State private var cropDown = false
    @State private var last: CGPoint = .zero
    @State private var grab: CGPoint = .zero
    @State private var snapX: Float?
    @State private var snapY: Float?
    @State private var draft: (ID, Float, Float, Float, Float)?
    @State private var cropDraft: (ID, Float, Float, Float, Float)?
    @State private var cameraDragging = false
    @State private var cameraZooming = false
    @State private var sizeCorner: SizeCorner?
    @State private var camGrab: CGPoint = .zero
    @State private var camDraft: SceneCamera?

    private let hues: [Color] = [
        Color(red: 0xE8 / 255, green: 0x77 / 255, blue: 0x22 / 255),
        Color(red: 0x42 / 255, green: 0xA5 / 255, blue: 0xF5 / 255),
        Color(red: 0x66 / 255, green: 0xBB / 255, blue: 0x6A / 255),
        Color(red: 0xAB / 255, green: 0x47 / 255, blue: 0xBC / 255),
        Color(red: 0xEF / 255, green: 0x53 / 255, blue: 0x50 / 255)
    ]

    var body: some View {
        GeometryReader { geo in
            let size = fitted(geo.size)
            let origin = CGPoint(x: (geo.size.width - size.width) / 2, y: (geo.size.height - size.height) / 2)
            let zoom = min(1, max(0.5, viewZoom))
            let sceneSize = CGSize(width: size.width * zoom, height: size.height * zoom)
            let sceneOrigin = CGPoint(
                x: origin.x + (size.width - sceneSize.width) / 2,
                y: origin.y + (size.height - sceneSize.height) / 2
            )
            ZStack(alignment: .topLeading) {
                Rectangle().fill(Color(red: 0.04, green: 0.04, blue: 0.04))
                if zoom < 0.999 {
                    Rectangle()
                        .fill(Color(white: 0.08))
                        .frame(width: sceneSize.width, height: sceneSize.height)
                        .position(x: sceneOrigin.x + sceneSize.width / 2, y: sceneOrigin.y + sceneSize.height / 2)
                    Rectangle()
                        .stroke(Color.white.opacity(0.35), lineWidth: 1)
                        .frame(width: sceneSize.width, height: sceneSize.height)
                        .position(x: sceneOrigin.x + sceneSize.width / 2, y: sceneOrigin.y + sceneSize.height / 2)
                }
                ForEach(Array(items.enumerated().reversed()), id: \.element.id) { index, item in
                    itemMarks(index: index, item: item, origin: sceneOrigin, canvas: sceneSize)
                }
                if let camera {
                    cameraMarks(camera, origin: sceneOrigin, canvas: sceneSize)
                }
            }
            .clipped()
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        let local = CGPoint(x: value.location.x - sceneOrigin.x, y: value.location.y - sceneOrigin.y)
                        if !dragging && !resizing && !cropping && !cameraDragging && !cameraZooming {
                            if let hit = hitCorner(local, canvas: sceneSize) {
                                selected = hit.0
                                sizeCorner = hit.1
                                resizing = true
                                dragging = false
                                cropping = false
                            } else if beginCamera(at: local, canvas: sceneSize) {
                                // The first event only chooses pan or zoom.
                            } else {
                                begin(at: local, canvas: sceneSize)
                            }
                            last = local
                            return
                        }
                        if cameraDragging || cameraZooming {
                            moveCamera(at: local, canvas: sceneSize)
                            return
                        }
                        guard let id = selected, var item = items.first(where: { $0.id == id }), !item.locked else { return }
                        let dx = Float((local.x - last.x) / sceneSize.width)
                        let dy = Float((local.y - last.y) / sceneSize.height)
                        last = local
                        if cropping {
                            applyCrop(&item, dx: dx, dy: dy)
                            cropDraft = (id, item.cropX, item.cropY, item.cropWidth, item.cropHeight)
                            onCrop?(id, item.cropX, item.cropY, item.cropWidth, item.cropHeight, false)
                        } else if resizing {
                            var box = resized(item, corner: sizeCorner ?? .se, dx: dx, dy: dy)
                            if snapEnabled {
                                snapResize(corner: sizeCorner ?? .se, x: &box.x, y: &box.y, width: &box.width, height: &box.height, linked: item.sizeLinked, except: id, canvas: sceneSize)
                            }
                            draft = (id, box.x, box.y, box.width, box.height)
                            onChange(id, box.x, box.y, box.width, box.height, false)
                        } else if dragging {
                            var x = Float(local.x / sceneSize.width - grab.x)
                            var y = Float(local.y / sceneSize.height - grab.y)
                            if snapEnabled {
                                snapMove(x: &x, y: &y, width: item.width, height: item.height, except: id, canvas: sceneSize)
                            }
                            draft = (id, x, y, item.width, item.height)
                            onChange(id, x, y, item.width, item.height, false)
                        }
                    }
                    .onEnded { _ in
                        if (cameraDragging || cameraZooming), let camDraft {
                            onCamera?(camDraft, true)
                        }
                        cameraDragging = false
                        cameraZooming = false
                        sizeCorner = nil
                        camDraft = nil
                        if cropping, let crop = cropDraft {
                            onCrop?(crop.0, crop.1, crop.2, crop.3, crop.4, true)
                        } else if let draft, dragging || resizing {
                            onChange(draft.0, draft.1, draft.2, draft.3, draft.4, true)
                        }
                        dragging = false
                        resizing = false
                        cropping = false
                        snapX = nil
                        snapY = nil
                        draft = nil
                        cropDraft = nil
                    }
            )
        }
        .aspectRatio(aspect, contentMode: .fit)
        .clipped()
        .background(Color.black)
        .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
    }

    @ViewBuilder
    private func itemMarks(index: Int, item: WireRect<ID>, origin: CGPoint, canvas: CGSize) -> some View {
        let color = item.enabled ? hues[index % hues.count] : EivizTheme.dim
        let frame = CGRect(
            x: origin.x + CGFloat(item.x) * canvas.width,
            y: origin.y + CGFloat(item.y) * canvas.height,
            width: max(8, CGFloat(item.width) * canvas.width),
            height: max(8, CGFloat(item.height) * canvas.height)
        )
        Rectangle()
            .fill(color.opacity(0.16))
            .overlay(Rectangle().stroke(color, lineWidth: selected == item.id ? 4 : 2))
            .frame(width: frame.width, height: frame.height)
            .position(x: frame.midX, y: frame.midY)
            .contextMenu {
                if let onFit {
                    Button(L10n.t("scene.fitToScreen")) {
                        selected = item.id
                        if !item.locked { onFit(item.id) }
                    }
                    .disabled(item.locked)
                }
            }
        if item.cropX > 0.001 || item.cropY > 0.001 || item.cropWidth < 0.999 || item.cropHeight < 0.999 {
            let crop = CGRect(
                x: frame.minX + frame.width * CGFloat(item.cropX),
                y: frame.minY + frame.height * CGFloat(item.cropY),
                width: max(4, frame.width * CGFloat(item.cropWidth)),
                height: max(4, frame.height * CGFloat(item.cropHeight))
            )
            Rectangle()
                .stroke(color, style: StrokeStyle(lineWidth: 1, dash: [3, 2]))
                .frame(width: crop.width, height: crop.height)
                .position(x: crop.midX, y: crop.midY)
        }
        Text("\(index + 1)")
            .font(.system(size: 16, weight: .bold))
            .foregroundStyle(.white)
            .position(x: frame.minX + 14, y: frame.minY + 12)
        if selected == item.id && !item.locked {
            ForEach(0..<4, id: \.self) { index in
                let x = index % 2 == 0 ? frame.minX : frame.maxX
                let y = index < 2 ? frame.minY : frame.maxY
                Rectangle()
                    .fill(color)
                    .frame(width: 14, height: 14)
                    .position(x: x, y: y)
            }
        }
    }

    private enum SizeCorner { case se, sw, ne, nw }

    private func hitCorner(_ pos: CGPoint, canvas: CGSize) -> (ID, SizeCorner)? {
        let ordered: [WireRect<ID>] = {
            if let id = selected, let item = items.first(where: { $0.id == id }) {
                return [item] + items.filter { $0.id != id }
            }
            return items
        }()
        for item in ordered where !item.locked {
            let rect = CGRect(
                x: CGFloat(item.x) * canvas.width,
                y: CGFloat(item.y) * canvas.height,
                width: CGFloat(item.width) * canvas.width,
                height: CGFloat(item.height) * canvas.height
            )
            let corners: [(SizeCorner, CGPoint)] = [
                (.se, CGPoint(x: rect.maxX, y: rect.maxY)),
                (.sw, CGPoint(x: rect.minX, y: rect.maxY)),
                (.ne, CGPoint(x: rect.maxX, y: rect.minY)),
                (.nw, CGPoint(x: rect.minX, y: rect.minY))
            ]
            for (corner, point) in corners where hypot(pos.x - point.x, pos.y - point.y) <= 22 {
                return (item.id, corner)
            }
        }
        return nil
    }

    private func resized(_ item: WireRect<ID>, corner: SizeCorner, dx: Float, dy: Float) -> (x: Float, y: Float, width: Float, height: Float) {
        let linked = item.sizeLinked && item.width > 0
        let ratio = item.width > 0 ? item.height / item.width : 1
        var width = item.width
        var height = item.height
        switch corner {
        case .se, .ne:
            width = max(0.02, item.width + dx)
        case .sw, .nw:
            width = max(0.02, item.width - dx)
        }
        if linked {
            height = max(0.02, width * ratio)
        } else {
            switch corner {
            case .se, .sw:
                height = max(0.02, item.height + dy)
            case .ne, .nw:
                height = max(0.02, item.height - dy)
            }
        }
        let x: Float
        let y: Float
        switch corner {
        case .se:
            x = item.x
            y = item.y
        case .sw:
            x = item.x + item.width - width
            y = item.y
        case .ne:
            x = item.x
            y = item.y + item.height - height
        case .nw:
            x = item.x + item.width - width
            y = item.y + item.height - height
        }
        return (x, y, width, height)
    }

    @ViewBuilder
    private func cameraMarks(_ camera: SceneCamera, origin: CGPoint, canvas: CGSize) -> some View {
        let frame = cameraRect(camera, canvas: canvas)
        let color = Color(red: 1, green: 0xE0 / 255, blue: 0x82 / 255)
        Rectangle()
            .stroke(color, style: StrokeStyle(lineWidth: 3, dash: [8, 4]))
            .frame(width: frame.width, height: frame.height)
            .position(x: origin.x + frame.midX, y: origin.y + frame.midY)
        Rectangle()
            .fill(color)
            .frame(width: 16, height: 16)
            .position(x: origin.x + frame.maxX - 8, y: origin.y + frame.maxY - 8)
    }

    private func cameraRect(_ camera: SceneCamera, canvas: CGSize) -> CGRect {
        let zoom = CGFloat(max(1, min(8, camera.zoom)))
        let width = canvas.width / zoom
        let height = canvas.height / zoom
        return CGRect(
            x: CGFloat(camera.x) * canvas.width - width / 2,
            y: CGFloat(camera.y) * canvas.height - height / 2,
            width: width,
            height: height
        )
    }

    /// Zoom handle first. Empty space inside the frame pans, and a layer underneath keeps the drag.
    private func beginCamera(at local: CGPoint, canvas: CGSize) -> Bool {
        guard let camera else { return false }
        let frame = cameraRect(camera, canvas: canvas)
        let handle = CGRect(x: frame.maxX - 20, y: frame.maxY - 20, width: 24, height: 24)
        if handle.contains(local) {
            cameraZooming = true
            cameraDragging = false
            return true
        }
        let onLayer = items.contains { item in
            CGRect(
                x: CGFloat(item.x) * canvas.width,
                y: CGFloat(item.y) * canvas.height,
                width: CGFloat(item.width) * canvas.width,
                height: CGFloat(item.height) * canvas.height
            ).contains(local)
        }
        guard frame.insetBy(dx: -4, dy: -4).contains(local), !onLayer else { return false }
        cameraDragging = true
        cameraZooming = false
        camGrab = CGPoint(
            x: local.x / canvas.width - CGFloat(camera.x),
            y: local.y / canvas.height - CGFloat(camera.y)
        )
        return true
    }

    private func moveCamera(at local: CGPoint, canvas: CGSize) {
        guard var camera else { return }
        if cameraZooming {
            let dx = abs(Float(local.x / canvas.width) - camera.x)
            let dy = abs(Float(local.y / canvas.height) - camera.y)
            let half = max(dx, dy)
            camera.zoom = half < 1e-3 ? 8 : 0.5 / half
        } else if cameraDragging {
            camera.x = Float(local.x / canvas.width) - Float(camGrab.x)
            camera.y = Float(local.y / canvas.height) - Float(camGrab.y)
        }
        camera = camera.clamped()
        camDraft = camera
        onCamera?(camera, false)
    }

    private func fitted(_ size: CGSize) -> CGSize {
        let ratio = max(aspect, 0.01)
        if size.width / size.height > ratio {
            return CGSize(width: size.height * ratio, height: size.height)
        }
        return CGSize(width: size.width, height: size.width / ratio)
    }

    private func begin(at pos: CGPoint, canvas: CGSize) {
        let option = NSEvent.modifierFlags.contains(.option)
        snapX = nil
        snapY = nil
        if let id = selected, let item = items.first(where: { $0.id == id }), !item.locked {
            let handle = CGRect(
                x: CGFloat(item.x + item.width) * canvas.width - 16,
                y: CGFloat(item.y + item.height) * canvas.height - 16,
                width: 16,
                height: 16
            )
            if !option, handle.insetBy(dx: -4, dy: -4).contains(pos) {
                resizing = true
                dragging = false
                cropping = false
                return
            }
        }
        let hits = items.filter { item in
            CGRect(
                x: CGFloat(item.x) * canvas.width,
                y: CGFloat(item.y) * canvas.height,
                width: CGFloat(item.width) * canvas.width,
                height: CGFloat(item.height) * canvas.height
            ).contains(pos)
        }
        if let current = selected, hits.contains(where: { $0.id == current }) {
            selected = current
        } else {
            selected = hits.first?.id
        }
        guard let id = selected, let item = items.first(where: { $0.id == id }), !item.locked else {
            dragging = false
            resizing = false
            cropping = false
            return
        }
        if option, onCrop != nil, beginCrop(item, pos: pos, canvas: canvas) {
            cropping = true
            dragging = false
            resizing = false
            return
        }
        grab = CGPoint(
            x: pos.x / canvas.width - CGFloat(item.x),
            y: pos.y / canvas.height - CGFloat(item.y)
        )
        dragging = true
        resizing = false
        cropping = false
    }

    private func beginCrop(_ item: WireRect<ID>, pos: CGPoint, canvas: CGSize) -> Bool {
        let left = CGFloat(item.x) * canvas.width
        let top = CGFloat(item.y) * canvas.height
        let right = CGFloat(item.x + item.width) * canvas.width
        let bottom = CGFloat(item.y + item.height) * canvas.height
        cropLeft = abs(pos.x - left) <= 8
        cropRight = abs(pos.x - right) <= 8
        cropUp = abs(pos.y - top) <= 8
        cropDown = abs(pos.y - bottom) <= 8
        return cropLeft || cropRight || cropUp || cropDown
    }

    private func applyCrop(_ item: inout WireRect<ID>, dx: Float, dy: Float) {
        guard item.width > 0, item.height > 0 else { return }
        if cropLeft { item.setCrop(.left, item.cropX + dx / item.width) }
        if cropRight { item.setCrop(.right, 1 - item.cropX - item.cropWidth - dx / item.width) }
        if cropUp { item.setCrop(.up, item.cropY + dy / item.height) }
        if cropDown { item.setCrop(.down, 1 - item.cropY - item.cropHeight - dy / item.height) }
    }

    private func snapMove(x: inout Float, y: inout Float, width: Float, height: Float, except: ID, canvas: CGSize) {
        let xs = guides(except: except, horizontal: true)
        let ys = guides(except: except, horizontal: false)
        let horizontal = snappedAxis(raw: x, size: width, guides: xs, pixels: canvas.width, latched: snapX)
        let vertical = snappedAxis(raw: y, size: height, guides: ys, pixels: canvas.height, latched: snapY)
        x = horizontal.value
        y = vertical.value
        snapX = horizontal.latch
        snapY = vertical.latch
    }

    private func snapResize(corner: SizeCorner, x: inout Float, y: inout Float, width: inout Float, height: inout Float, linked: Bool, except: ID, canvas: CGSize) {
        let xThreshold = Float(6 / max(canvas.width, 1))
        let yThreshold = Float(6 / max(canvas.height, 1))
        let ratio = height / max(width, 0.0001)
        let moveLeft = corner == .sw || corner == .nw
        let moveTop = corner == .nw || corner == .ne
        var right = x + width
        var bottom = y + height
        if moveLeft {
            x = chooseEdge(moving: x, anchor: right, sizes: sizes(except: except, horizontal: true), guides: guides(except: except, horizontal: true), threshold: xThreshold, anchorIsEnd: true)
        } else {
            right = chooseEdge(moving: right, anchor: x, sizes: sizes(except: except, horizontal: true), guides: guides(except: except, horizontal: true), threshold: xThreshold, anchorIsEnd: false)
        }
        width = max(0.02, right - x)
        if linked {
            let nextHeight = max(0.02, width * ratio)
            if moveTop { y = bottom - nextHeight }
            height = nextHeight
            return
        }
        if moveTop {
            y = chooseEdge(moving: y, anchor: bottom, sizes: sizes(except: except, horizontal: false), guides: guides(except: except, horizontal: false), threshold: yThreshold, anchorIsEnd: true)
        } else {
            bottom = chooseEdge(moving: bottom, anchor: y, sizes: sizes(except: except, horizontal: false), guides: guides(except: except, horizontal: false), threshold: yThreshold, anchorIsEnd: false)
        }
        height = max(0.02, bottom - y)
    }

    /// Frame size, plus the size of every other layer. Either can sit outside the frame.
    private func sizes(except: ID, horizontal: Bool) -> [Float] {
        var values: [Float] = [1]
        for item in items where item.id != except && item.enabled {
            let size = horizontal ? item.width : item.height
            if size > 0.02 { values.append(size) }
        }
        return values
    }

    private func chooseEdge(moving: Float, anchor: Float, sizes: [Float], guides: [Float], threshold: Float, anchorIsEnd: Bool) -> Float {
        var best = moving
        var bestAbs = threshold
        func take(_ candidate: Float) {
            let absv = abs(candidate - moving)
            if absv <= bestAbs {
                bestAbs = absv
                best = candidate
            }
        }
        for guide in guides { take(guide) }
        for size in sizes {
            take(anchorIsEnd ? anchor - size : anchor + size)
        }
        return best
    }

    private func guides(except: ID, horizontal: Bool) -> [Float] {
        var values: [Float] = [0, 0.5, 1]
        for item in items where item.id != except && item.enabled {
            if horizontal {
                values.append(item.x)
                values.append(item.x + item.width * 0.5)
                values.append(item.x + item.width)
            } else {
                values.append(item.y)
                values.append(item.y + item.height * 0.5)
                values.append(item.y + item.height)
            }
        }
        return values
    }

    private func snappedAxis(raw: Float, size: Float, guides: [Float], pixels: CGFloat, latched: Float?) -> (value: Float, latch: Float?) {
        let rendered = Float(max(pixels, 1))
        if let target = latched, abs(raw - target) * rendered <= 12 {
            return (target, target)
        }
        let points = [raw, raw + size * 0.5, raw + size]
        if let delta = snapDelta(points, guides, 6 / rendered) {
            let target = raw + delta
            return (target, target)
        }
        return (raw, nil)
    }

    private func snapDelta(_ points: [Float], _ guides: [Float], _ threshold: Float) -> Float? {
        var best: Float?
        var bestAbs = threshold
        for point in points {
            for guide in guides {
                let delta = guide - point
                let absv = abs(delta)
                if absv <= bestAbs {
                    bestAbs = absv
                    best = delta
                }
            }
        }
        return best
    }
}

private extension WireRect {
    enum CropEdge { case left, right, up, down }

    mutating func setCrop(_ edge: CropEdge, _ value: Float) {
        var left = cropX
        var up = cropY
        var right = 1 - cropX - cropWidth
        var down = 1 - cropY - cropHeight
        switch edge {
        case .left: left = max(0, min(value, 1 - right))
        case .up: up = max(0, min(value, 1 - down))
        case .right: right = max(0, min(value, 1 - left))
        case .down: down = max(0, min(value, 1 - up))
        }
        cropX = left
        cropY = up
        cropWidth = max(0, 1 - left - right)
        cropHeight = max(0, 1 - up - down)
    }
}

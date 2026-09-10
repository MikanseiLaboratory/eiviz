import AppKit
import SwiftUI

struct WireRect: Identifiable, Equatable {
    var id: UUID
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

struct WireCanvasView: View {
    var items: [WireRect]
    var aspect: CGFloat = 16.0 / 9.0
    var snapEnabled = false
    var onFit: ((UUID) -> Void)?
    var onCrop: ((UUID, Float, Float, Float, Float, Bool) -> Void)?
    @Binding var selected: UUID?
    var onChange: (UUID, Float, Float, Float, Float, Bool) -> Void

    @State private var dragging = false
    @State private var resizing = false
    @State private var cropping = false
    @State private var cropLeft = false
    @State private var cropRight = false
    @State private var cropUp = false
    @State private var cropDown = false
    @State private var last: CGPoint = .zero
    @State private var draft: (UUID, Float, Float, Float, Float)?
    @State private var cropDraft: (UUID, Float, Float, Float, Float)?

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
            ZStack(alignment: .topLeading) {
                Rectangle().fill(Color(red: 0.04, green: 0.04, blue: 0.04))
                ForEach(Array(items.enumerated().reversed()), id: \.element.id) { index, item in
                    let color = item.enabled ? hues[index % hues.count] : EivizTheme.dim
                    let frame = CGRect(
                        x: origin.x + CGFloat(item.x) * size.width,
                        y: origin.y + CGFloat(item.y) * size.height,
                        width: max(8, CGFloat(item.width) * size.width),
                        height: max(8, CGFloat(item.height) * size.height)
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
                        Rectangle()
                            .fill(color)
                            .frame(width: 16, height: 16)
                            .position(x: frame.maxX - 8, y: frame.maxY - 8)
                    }
                }
            }
            .clipped()
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        let local = CGPoint(x: value.location.x - origin.x, y: value.location.y - origin.y)
                        if !dragging && !resizing && !cropping {
                            begin(at: local, canvas: size)
                            last = local
                            return
                        }
                        guard let id = selected, var item = items.first(where: { $0.id == id }), !item.locked else { return }
                        let dx = Float((local.x - last.x) / size.width)
                        let dy = Float((local.y - last.y) / size.height)
                        last = local
                        if cropping {
                            applyCrop(&item, dx: dx, dy: dy)
                            cropDraft = (id, item.cropX, item.cropY, item.cropWidth, item.cropHeight)
                            onCrop?(id, item.cropX, item.cropY, item.cropWidth, item.cropHeight, false)
                        } else if resizing {
                            var width = max(0.02, item.width + dx)
                            var height = item.sizeLinked && item.width > 0
                                ? max(0.02, width * (item.height / item.width))
                                : max(0.02, item.height + dy)
                            if snapEnabled {
                                snapResize(x: item.x, y: item.y, width: &width, height: &height, linked: item.sizeLinked, except: id, canvas: size)
                            }
                            draft = (id, item.x, item.y, width, height)
                            onChange(id, item.x, item.y, width, height, false)
                        } else if dragging {
                            var x = item.x + dx
                            var y = item.y + dy
                            if snapEnabled {
                                snapMove(x: &x, y: &y, width: item.width, height: item.height, except: id, canvas: size)
                            }
                            draft = (id, x, y, item.width, item.height)
                            onChange(id, x, y, item.width, item.height, false)
                        }
                    }
                    .onEnded { _ in
                        if cropping, let crop = cropDraft {
                            onCrop?(crop.0, crop.1, crop.2, crop.3, crop.4, true)
                        } else if let draft, dragging || resizing {
                            onChange(draft.0, draft.1, draft.2, draft.3, draft.4, true)
                        }
                        dragging = false
                        resizing = false
                        cropping = false
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

    private func fitted(_ size: CGSize) -> CGSize {
        let ratio = max(aspect, 0.01)
        if size.width / size.height > ratio {
            return CGSize(width: size.height * ratio, height: size.height)
        }
        return CGSize(width: size.width, height: size.width / ratio)
    }

    private func begin(at pos: CGPoint, canvas: CGSize) {
        let option = NSEvent.modifierFlags.contains(.option)
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
        dragging = true
        resizing = false
        cropping = false
    }

    private func beginCrop(_ item: WireRect, pos: CGPoint, canvas: CGSize) -> Bool {
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

    private func applyCrop(_ item: inout WireRect, dx: Float, dy: Float) {
        guard item.width > 0, item.height > 0 else { return }
        if cropLeft { item.setCrop(.left, item.cropX + dx / item.width) }
        if cropRight { item.setCrop(.right, 1 - item.cropX - item.cropWidth - dx / item.width) }
        if cropUp { item.setCrop(.up, item.cropY + dy / item.height) }
        if cropDown { item.setCrop(.down, 1 - item.cropY - item.cropHeight - dy / item.height) }
    }

    private func snapMove(x: inout Float, y: inout Float, width: Float, height: Float, except: UUID, canvas: CGSize) {
        let threshold = Float(8 / max(canvas.width, 1))
        let xs = guides(except: except, horizontal: true)
        let ys = guides(except: except, horizontal: false)
        x += bestDelta([x, x + width * 0.5, x + width], xs, threshold)
        y += bestDelta([y, y + height * 0.5, y + height], ys, threshold)
    }

    private func snapResize(x: Float, y: Float, width: inout Float, height: inout Float, linked: Bool, except: UUID, canvas: CGSize) {
        let threshold = Float(8 / max(canvas.width, 1))
        let xs = guides(except: except, horizontal: true)
        width = max(0.02, x + width + bestDelta([x + width], xs, threshold) - x)
        if linked {
            return
        }
        let ys = guides(except: except, horizontal: false)
        height = max(0.02, y + height + bestDelta([y + height], ys, threshold) - y)
    }

    private func guides(except: UUID, horizontal: Bool) -> [Float] {
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

    private func bestDelta(_ points: [Float], _ guides: [Float], _ threshold: Float) -> Float {
        var best: Float = 0
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
        return bestAbs <= threshold ? best : 0
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

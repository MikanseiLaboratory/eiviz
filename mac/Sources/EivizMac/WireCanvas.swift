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
    @Binding var selected: UUID?
    var onChange: (UUID, Float, Float, Float, Float, Bool) -> Void

    @State private var dragging = false
    @State private var resizing = false
    @State private var last: CGPoint = .zero

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
                    let color = item.enabled ? hues[index % hues.count] : Color(white: 0.33)
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
                        if !dragging && !resizing {
                            begin(at: local, canvas: size)
                            last = local
                            return
                        }
                        guard let id = selected, let item = items.first(where: { $0.id == id }), !item.locked else { return }
                        let dx = Float((local.x - last.x) / size.width)
                        let dy = Float((local.y - last.y) / size.height)
                        last = local
                        if resizing {
                            let width = max(0.02, item.width + dx)
                            let height = item.sizeLinked && item.width > 0
                                ? max(0.02, width * (item.height / item.width))
                                : max(0.02, item.height + dy)
                            onChange(id, item.x, item.y, width, height, false)
                        } else if dragging {
                            onChange(id, item.x + dx, item.y + dy, item.width, item.height, false)
                        }
                    }
                    .onEnded { _ in
                        if let id = selected, let item = items.first(where: { $0.id == id }), dragging || resizing {
                            onChange(id, item.x, item.y, item.width, item.height, true)
                        }
                        dragging = false
                        resizing = false
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
        if let id = selected, let item = items.first(where: { $0.id == id }), !item.locked {
            let handle = CGRect(
                x: CGFloat(item.x + item.width) * canvas.width - 16,
                y: CGFloat(item.y + item.height) * canvas.height - 16,
                width: 16,
                height: 16
            )
            if handle.insetBy(dx: -4, dy: -4).contains(pos) {
                resizing = true
                dragging = false
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
            dragging = items.first(where: { $0.id == current })?.locked != true
            resizing = false
            return
        }
        let hit = hits.first?.id
        selected = hit
        let locked = items.first(where: { $0.id == hit })?.locked == true
        dragging = hit != nil && !locked
        resizing = false
    }
}

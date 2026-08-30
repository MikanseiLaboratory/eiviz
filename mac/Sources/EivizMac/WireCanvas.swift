import SwiftUI

struct WireRect: Identifiable, Equatable {
    var id: UUID
    var x: Float
    var y: Float
    var width: Float
    var height: Float
    var enabled: Bool = true
}

struct WireCanvasView: View {
    var items: [WireRect]
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
                ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
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
                    if selected == item.id {
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
                        guard let id = selected, let item = items.first(where: { $0.id == id }) else { return }
                        let dx = Float((local.x - last.x) / size.width)
                        let dy = Float((local.y - last.y) / size.height)
                        last = local
                        if resizing {
                            onChange(id, item.x, item.y, max(0.02, item.width + dx), max(0.02, item.height + dy), false)
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
        .aspectRatio(16.0 / 9.0, contentMode: .fit)
        .clipped()
        .background(Color.black)
        .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
    }

    private func fitted(_ size: CGSize) -> CGSize {
        let aspect: CGFloat = 16.0 / 9.0
        if size.width / size.height > aspect {
            return CGSize(width: size.height * aspect, height: size.height)
        }
        return CGSize(width: size.width, height: size.width / aspect)
    }

    private func begin(at pos: CGPoint, canvas: CGSize) {
        if let id = selected, let item = items.first(where: { $0.id == id }) {
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
        var hit: UUID?
        for item in items.reversed() {
            let rect = CGRect(
                x: CGFloat(item.x) * canvas.width,
                y: CGFloat(item.y) * canvas.height,
                width: CGFloat(item.width) * canvas.width,
                height: CGFloat(item.height) * canvas.height
            )
            if rect.contains(pos) {
                hit = item.id
                break
            }
        }
        selected = hit
        dragging = hit != nil
        resizing = false
    }
}

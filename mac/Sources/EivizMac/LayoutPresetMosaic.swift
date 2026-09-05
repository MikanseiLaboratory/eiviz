import SwiftUI

enum SceneLayoutPresets {
    static let builtIn = ["Full", "Split H", "Split V", "Quad", "PiP TR", "PiP TL", "PiP BR", "PiP BL"]

    static func boxes(_ name: String) -> [(CGFloat, CGFloat, CGFloat, CGFloat)] {
        switch name {
        case "Full": return [(0, 0, 1, 1)]
        case "Split H": return [(0, 0, 0.5, 1), (0.5, 0, 0.5, 1)]
        case "Split V": return [(0, 0, 1, 0.5), (0, 0.5, 1, 0.5)]
        case "Quad": return [(0, 0, 0.5, 0.5), (0.5, 0, 0.5, 0.5), (0, 0.5, 0.5, 0.5), (0.5, 0.5, 0.5, 0.5)]
        case "PiP TR": return [(0, 0, 1, 1), (0.62, 0.08, 0.32, 0.32)]
        case "PiP TL": return [(0, 0, 1, 1), (0.06, 0.08, 0.32, 0.32)]
        case "PiP BR": return [(0, 0, 1, 1), (0.62, 0.60, 0.32, 0.32)]
        case "PiP BL": return [(0, 0, 1, 1), (0.06, 0.60, 0.32, 0.32)]
        default: return []
        }
    }
}

struct LayoutPresetMosaic: View {
    let boxes: [(CGFloat, CGFloat, CGFloat, CGFloat)]

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                Color.black
                ForEach(Array(boxes.enumerated()), id: \.offset) { index, box in
                    let rect = CGRect(
                        x: box.0 * geo.size.width,
                        y: box.1 * geo.size.height,
                        width: max(1, box.2 * geo.size.width - 1),
                        height: max(1, box.3 * geo.size.height - 1)
                    )
                    ZStack {
                        Rectangle().fill(EivizTheme.list)
                        Rectangle().stroke(Color.black, lineWidth: 1)
                        Text("\(index + 1)")
                            .font(.system(size: min(18, min(rect.width, rect.height) * 0.42), weight: .bold))
                            .foregroundStyle(.white)
                    }
                    .frame(width: rect.width, height: rect.height)
                    .position(x: rect.midX, y: rect.midY)
                }
            }
            .clipped()
        }
        .aspectRatio(16 / 9, contentMode: .fit)
        .background(Color.black)
    }
}

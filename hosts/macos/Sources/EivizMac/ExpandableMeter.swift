import SwiftUI

struct ExpandableMeter: View {
    let title: String
    @Binding var value: Float
    var range: ClosedRange<Float>
    var disabled: Bool = false
    var pixelsPerUnit: Float = 2
    var onChange: (_ ended: Bool) -> Void

    @State private var expanded = false
    @State private var lastY: CGFloat = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 4) {
                Button {
                    guard !disabled else { return }
                    expanded.toggle()
                } label: {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 8, weight: .bold))
                        .frame(width: 10)
                }
                .buttonStyle(.plain)
                .disabled(disabled)
                Text(title)
                    .font(.system(size: 11))
                    .frame(minWidth: 48, alignment: .leading)
                    .contentShape(Rectangle())
                    .gesture(drag)
                    .disabled(disabled)
                mixerFloatField(
                    Binding(
                        get: { value },
                        set: { next in
                            value = clamp(next)
                        }
                    ),
                    onSubmit: { onChange(true) }
                )
                .frame(width: 72)
                .disabled(disabled)
            }
            if expanded {
                Slider(
                    value: Binding(
                        get: { Double(clamp(value)) },
                        set: { next in
                            value = Float(next)
                            onChange(false)
                        }
                    ),
                    in: Double(range.lowerBound) ... Double(range.upperBound),
                    onEditingChanged: { editing in
                        if !editing { onChange(true) }
                    }
                )
                .disabled(disabled)
            }
        }
    }

    private var drag: some Gesture {
        DragGesture(minimumDistance: 2)
            .onChanged { gesture in
                guard !disabled else { return }
                let y = gesture.translation.height
                let step = Float(lastY - y) / pixelsPerUnit
                lastY = y
                if step != 0 {
                    value = clamp(value + step)
                    onChange(false)
                }
            }
            .onEnded { _ in
                lastY = 0
                onChange(true)
            }
    }

    private func clamp(_ next: Float) -> Float {
        min(range.upperBound, max(range.lowerBound, next))
    }
}

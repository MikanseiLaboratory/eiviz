import SwiftUI

struct ScenePreviewTile: View, Equatable {
    let sceneId: UInt64
    let monitorId: UInt64
    let gpuId: UInt64
    let name: String
    let number: Int
    let preview: Bool
    let program: Bool
    let loopOn: Bool
    let playing: Bool
    let muted: Bool
    let hasVideo: Bool
    let previewColor: Color
    let programColor: Color
    let inactiveColor: Color
    let onPreview: () -> Void
    let onCut: () -> Void
    let onLoop: () -> Void
    let onPlay: () -> Void
    let onAudio: () -> Void
    let onOpenPreview: () -> Void
    let onEdit: () -> Void
    let onDelete: () -> Void

    static func == (lhs: ScenePreviewTile, rhs: ScenePreviewTile) -> Bool {
        lhs.sceneId == rhs.sceneId
            && lhs.monitorId == rhs.monitorId
            && lhs.gpuId == rhs.gpuId
            && lhs.name == rhs.name
            && lhs.number == rhs.number
            && lhs.preview == rhs.preview
            && lhs.program == rhs.program
            && lhs.loopOn == rhs.loopOn
            && lhs.playing == rhs.playing
            && lhs.muted == rhs.muted
            && lhs.hasVideo == rhs.hasVideo
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Text("\(number)")
                    .font(.system(size: 11, weight: .bold))
                Text(name)
                    .font(.system(size: 12))
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button("X", action: onDelete)
                    .buttonStyle(MixerTileButtonStyle())
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(Color(white: 0.2))
            .contentShape(Rectangle())
            .onTapGesture(perform: onPreview)
            MetalPreviewRepresentable(
                role: .monitor(monitorId: monitorId, sourceId: gpuId),
                presentInterval: 1,
                onClick: onPreview
            )
            .frame(width: 176, height: 90)
            HStack(spacing: 1) {
                chip("CUT", action: onCut)
                chip("Loop", action: onLoop)
                    .opacity(hasVideo ? (loopOn ? 1 : 0.55) : 0.35)
                    .disabled(!hasVideo)
                chip(playing ? "❚❚" : "▶", action: onPlay)
                    .disabled(!hasVideo)
                chip("Aud", action: onAudio)
                    .opacity(muted ? 0.45 : 1)
                chip("Prev", action: onOpenPreview)
                chip("Set", action: onEdit)
            }
            .padding(2)
        }
        .frame(width: 176)
        .background(Rectangle().stroke(
            program ? programColor : preview ? previewColor : inactiveColor,
            lineWidth: 2
        ))
        .contextMenu {
            Button("Edit", action: onEdit)
        }
    }

    private func chip(_ title: String, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .buttonStyle(MixerTileButtonStyle())
    }
}

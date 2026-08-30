import EivizMixer
import Foundation

enum InputKind: String, Codable, CaseIterable {
    case color, bars, black, still, video, omt, ndi, uvc

    var category: String {
        switch self {
        case .color, .bars, .black: return "Colours"
        case .still: return "Still"
        case .video: return "Video"
        case .omt: return "OMT"
        case .ndi: return "NDI®"
        case .uvc: return "Video Capture"
        }
    }
}

enum BandwidthSave: UInt32, Codable {
    case alwaysLow = 0
    case notOnProgram = 1
    case notOnPreviewOrProgram = 2
    case alwaysFull = 3
}

enum OutputTransport: UInt32, Codable {
    case omt = 0
    case ndi = 1
    case deckLink = 2
}

enum OutputSourceKind: UInt32, Codable {
    case scene = 0
    case muPreview = 1
    case muProgram = 2
    case multiview = 3
    case input = 4
}

enum MvSlotKind: String, Codable {
    case none, input, scene, muPreview, muProgram
}

struct MvSlot: Codable, Identifiable {
    var id = UUID()
    var kind: MvSlotKind = .none
    var sourceId: UInt64 = 0
}

struct InputEntry: Identifiable, Codable, Hashable {
    var id: UInt64
    var name: String
    var kind: InputKind
    var pathOrAddress: String?
    var colorR: Float = 1
    var colorG: Float = 0
    var colorB: Float = 0
    var scroll: Bool = false
    var busMask: UInt32 = 1
    var gain: Float = 1
    var mute: Bool = false
    var useGpu: Bool = true
    var frameBufferFrames: UInt32 = 1
    var bandwidthSave: BandwidthSave = .notOnPreviewOrProgram
    var keepFullOnMultiview: Bool = false
    var omtQuality: UInt32 = 0
    var ndiBandwidth: UInt32 = 0
    var isBuiltin: Bool { id <= EIVIZ_SRC_BLUE }
}

struct SceneLayer: Identifiable, Codable {
    var id = UUID()
    var inputId: UInt64
    var x: Float = 0
    var y: Float = 0
    var width: Float = 1
    var height: Float = 1
    var opacity: Float = 1
    var z: Int32 = 0
    var audioFollow: Bool = true
}

struct SceneEntry: Identifiable, Codable {
    var id: UInt64
    var name: String
    var monitorId: UInt64
    var layers: [SceneLayer] = []
    var gpuId: UInt64 { EIVIZ_SCENE_BASE | id }
}

struct TransitionPreset: Identifiable, Codable {
    var id = UUID()
    var kind: UInt32 = EIVIZ_TRANSITION_FADE
    var durationFrames: UInt32 = 30
    var swap: Bool = true
    var label: String {
        switch kind {
        case EIVIZ_TRANSITION_CUT: return "Cut"
        case EIVIZ_TRANSITION_DIP: return "Dip"
        default: return "Fade"
        }
    }
}

struct OverlaySlot: Identifiable, Codable {
    var id = UUID()
    var sceneGpuId: UInt64 = 0
    var x: Float = 0.62
    var y: Float = 0.08
    var width: Float = 0.32
    var height: Float = 0.32
    var opacity: Float = 1
    var z: Int32 = 0
    var enabled: Bool = true
}

struct MixingUnitEntry: Identifiable, Codable {
    var id: UInt64
    var name: String
    var width: UInt32 = 1920
    var height: UInt32 = 1080
    var fpsNum: UInt32 = 60_000
    var fpsDen: UInt32 = 1_001
    var transitions: [TransitionPreset] = []
    var overlays: [OverlaySlot] = []
    var audioBusId: UInt64 = 1
    var displayName: String { "\(name)  \(width)x\(height) \(fpsLabel)" }
    var fpsLabel: String {
        if fpsNum == 60_000 && fpsDen == 1_001 { return "59.94p" }
        if fpsDen == 1 { return "\(fpsNum)p" }
        return "\(fpsNum)/\(fpsDen)"
    }
    func durationMs(_ frames: UInt32) -> UInt32 {
        UInt32(max(1, (Double(frames) * 1000.0 * Double(fpsDen) / Double(fpsNum)).rounded()))
    }
}

struct OutputEntry: Identifiable, Codable {
    var id: UInt64
    var name: String
    var transport: OutputTransport = .omt
    var sourceKind: OutputSourceKind = .muProgram
    var sourceId: UInt64 = 0
    var unitId: UInt64 = 1
    var useGpu: Bool = true
}

struct MultiviewLayout: Identifiable, Codable {
    var id: UInt64
    var name: String
    var monitorId: UInt64
    var previewUnitId: UInt64 = 1
    var programUnitId: UInt64 = 1
    var presentInterval: UInt32 = 0
    var tiles: [MvSlot] = Array(repeating: MvSlot(), count: 8)
    var gpuId: UInt64 { EIVIZ_MULTIVIEW_BASE | id }
}

struct AudioBusEntry: Identifiable, Codable {
    var id: UInt64
    var name: String
    var role: UInt32 = 0
    var gain: Float = 1
    var mute: Bool = false
}

struct SessionSettings: Codable {
    var masterFpsNum: UInt32 = 60_000
    var masterFpsDen: UInt32 = 1_001
    var defaultWidth: UInt32 = 1920
    var defaultHeight: UInt32 = 1080
    var frameBufferFrames: UInt32 = 3
    var defaultPresentInterval: UInt32 = 3
}

struct MixerSessionData: Codable {
    var settings = SessionSettings()
    var inputs: [InputEntry] = []
    var scenes: [SceneEntry] = []
    var units: [MixingUnitEntry] = []
    var outputs: [OutputEntry] = []
    var multiviews: [MultiviewLayout] = []
    var buses: [AudioBusEntry] = []
    var nextInputId: UInt64 = 10
    var nextSceneId: UInt64 = 1
    var nextUnitId: UInt64 = 2
    var nextMonitorId: UInt64 = 1000
    var nextOutputId: UInt64 = 100
    var nextMultiviewId: UInt64 = 1
    var selectedUnitId: UInt64 = 1

    static func `default`() -> MixerSessionData {
        var session = MixerSessionData()
        session.inputs = [
            InputEntry(id: EIVIZ_SRC_COLOR, name: "Color Red", kind: .color, colorR: 1),
            InputEntry(id: EIVIZ_SRC_BARS, name: "SMPTE Bars", kind: .bars),
            InputEntry(id: EIVIZ_SRC_BLACK, name: "Black", kind: .black, colorR: 0, colorG: 0, colorB: 0),
            InputEntry(id: EIVIZ_SRC_BLUE, name: "Blue", kind: .color, colorR: 0, colorG: 0, colorB: 1)
        ]
        var unit = MixingUnitEntry(id: 1, name: "Mixing Unit 1")
        unit.transitions = [
            TransitionPreset(kind: EIVIZ_TRANSITION_CUT, durationFrames: 1, swap: true),
            TransitionPreset(kind: EIVIZ_TRANSITION_FADE, durationFrames: 30, swap: true)
        ]
        session.units = [unit]
        session.buses = [
            AudioBusEntry(id: 1, name: "Master", role: 0),
            AudioBusEntry(id: 2, name: "Headphone", role: 1)
        ]
        session.addScene(name: "Scene 1", input: EIVIZ_SRC_BARS)
        session.addScene(name: "Scene 2", input: EIVIZ_SRC_COLOR)
        session.outputs = [
            OutputEntry(id: session.nextOutputId, name: "eiviz-pgm", transport: .omt, useGpu: true)
        ]
        session.nextOutputId += 1
        return session
    }

    @discardableResult
    mutating func addScene(name: String, input: UInt64?) -> SceneEntry {
        let scene = SceneEntry(
            id: nextSceneId,
            name: name,
            monitorId: nextMonitorId,
            layers: input.map { [SceneLayer(inputId: $0)] } ?? []
        )
        nextSceneId += 1
        nextMonitorId += 1
        scenes.append(scene)
        return scene
    }
}

import EivizMixer
import Foundation

enum InputKind: String, Codable, CaseIterable {
    case color = "Color"
    case bars = "Bars"
    case black = "Black"
    case still = "Still"
    case video = "Video"
    case omt = "Omt"
    case ndi = "Ndi"
    case uvc = "Uvc"

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

enum BandwidthSave: String, Codable {
    case alwaysLow = "AlwaysLow"
    case notOnProgram = "NotOnProgram"
    case notOnPreviewOrProgram = "NotOnPreviewOrProgram"
    case alwaysFull = "AlwaysFull"

    var rawUInt: UInt32 {
        switch self {
        case .alwaysLow: return 0
        case .notOnProgram: return 1
        case .notOnPreviewOrProgram: return 2
        case .alwaysFull: return 3
        }
    }
}

enum OmtQuality: String, Codable {
    case `default` = "Default"
    case low = "Low"
    case medium = "Medium"
    case high = "High"

    var rawUInt: UInt32 {
        switch self {
        case .default: return 0
        case .low: return 1
        case .medium: return 50
        case .high: return 100
        }
    }
}

enum NdiBandwidth: String, Codable {
    case highest = "Highest"
    case lowest = "Lowest"
    var rawUInt: UInt32 { self == .lowest ? 1 : 0 }
}

enum OutputTransport: String, Codable {
    case omt = "Omt"
    case ndi = "Ndi"
    case deckLink = "DeckLink"
    var rawValueU32: UInt32 {
        switch self {
        case .omt: return 0
        case .ndi: return 1
        case .deckLink: return 2
        }
    }
}

enum OutputSourceKind: String, Codable {
    case scene = "Scene"
    case muPreview = "MuPreview"
    case muProgram = "MuProgram"
    case multiview = "Multiview"
    case input = "Input"
    var rawValueU32: UInt32 {
        switch self {
        case .scene: return 0
        case .muPreview: return 1
        case .muProgram: return 2
        case .multiview: return 3
        case .input: return 4
        }
    }
}

enum MvSlotKind: String, Codable {
    case none = "None"
    case input = "Input"
    case scene = "Scene"
    case muPreview = "MuPreview"
    case muProgram = "MuProgram"
}

enum AudioBusRole: String, Codable {
    case master = "Master"
    case headphone = "Headphone"
    case aux = "Aux"
    var rawUInt: UInt32 {
        switch self {
        case .master: return 0
        case .headphone: return 1
        case .aux: return 2
        }
    }
}

enum AudioDeviceKind: String, Codable {
    case none = "None"
    case wasapi = "Wasapi"
    case asio = "Asio"
    case coreAudio = "CoreAudio"
    var rawUInt: UInt32 {
        switch self {
        case .none: return 0
        case .wasapi: return 1
        case .asio: return 2
        case .coreAudio: return 3
        }
    }
}

enum AudioLinkMode: String, Codable {
    case follow = "Follow"
    case independent = "Independent"
    var rawUInt: UInt32 { self == .independent ? 1 : 0 }
}

enum InternalColorFormat: String, Codable {
    case uyvy = "Uyvy"
    case bgra = "Bgra"
}

struct MvSlot: Codable, Equatable {
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
    var omtQuality: OmtQuality = .default
    var ndiBandwidth: NdiBandwidth = .highest
    var isBuiltin: Bool { id <= EIVIZ_SRC_BLUE }
}

struct SceneLayer: Identifiable, Codable, Equatable {
    var id = UUID()
    var inputId: UInt64
    var x: Float = 0
    var y: Float = 0
    var width: Float = 1
    var height: Float = 1
    var opacity: Float = 1
    var z: Int32 = 0
    var audioFollow: Bool = true

    enum CodingKeys: String, CodingKey {
        case inputId, x, y, width, height, opacity, z, audioFollow
    }
}

struct SceneEntry: Identifiable, Codable {
    var id: UInt64
    var name: String
    var monitorId: UInt64 = 0
    var layers: [SceneLayer] = []
    var gpuId: UInt64 { EIVIZ_SCENE_BASE | id }

    enum CodingKeys: String, CodingKey {
        case id, name, layers
    }
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

    enum CodingKeys: String, CodingKey {
        case kind, durationFrames, swap
    }
}

struct OverlaySlot: Identifiable, Codable, Equatable {
    var id = UUID()
    var sceneGpuId: UInt64 = 0
    var x: Float = 0.62
    var y: Float = 0.08
    var width: Float = 0.32
    var height: Float = 0.32
    var opacity: Float = 1
    var z: Int32 = 0
    var enabled: Bool = true

    enum CodingKeys: String, CodingKey {
        case sceneGpuId, x, y, width, height, opacity, z, enabled
    }
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
    var multiviewTiles: [MvSlot] = Array(repeating: MvSlot(), count: 8)
    var audioBusId: UInt64 = 1
    var audioLink: AudioLinkMode = .follow
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
    var monitorId: UInt64 = 0
    var previewUnitId: UInt64 = 1
    var programUnitId: UInt64 = 1
    var presentInterval: UInt32 = 0
    var tiles: [MvSlot] = Array(repeating: MvSlot(), count: 8)
    var gpuId: UInt64 { EIVIZ_MULTIVIEW_BASE | id }

    enum CodingKeys: String, CodingKey {
        case id, name, previewUnitId, programUnitId, presentInterval, tiles
    }
}

struct AudioBusEntry: Identifiable, Codable {
    var id: UInt64
    var name: String
    var role: AudioBusRole = .master
    var deviceKind: AudioDeviceKind = .none
    var deviceId: String = ""
    var mapLeft: Int32 = 0
    var mapRight: Int32 = 1
    var exclusive: Bool = false
    var bit: UInt32 = 0
    var gain: Float = 1
    var mute: Bool = false
}

struct SessionSettings: Codable {
    var masterFpsNum: UInt32 = 60_000
    var masterFpsDen: UInt32 = 1_001
    var defaultWidth: UInt32 = 1920
    var defaultHeight: UInt32 = 1080
    var theme: String = "Charcoal"
    var defaultMultiviewUnitId: UInt64 = 1
    var frameBufferFrames: UInt32 = 3
    var defaultPresentInterval: UInt32 = 3
    var internalColorFormat: InternalColorFormat = .uyvy
    var lastSessionPath: String?
}

struct MixerSessionData: Codable {
    var version: Int = 1
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
    var nextBusId: UInt64 = 3
    var selectedUnitId: UInt64 = 1
    var headphoneCopyMaster: Bool = false

    enum CodingKeys: String, CodingKey {
        case version, settings, inputs, scenes, units, outputs, multiviews, buses
        case nextInputId, nextSceneId, nextUnitId, nextOutputId, nextMultiviewId, nextBusId
        case selectedUnitId, headphoneCopyMaster
    }

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
            AudioBusEntry(id: 1, name: "Master", role: .master, deviceKind: .wasapi, mapLeft: 0, mapRight: 1, bit: 0),
            AudioBusEntry(id: 2, name: "Headphone", role: .headphone, deviceKind: .none, mapLeft: 0, mapRight: 1, bit: 1)
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

    mutating func assignMonitors() {
        if nextMonitorId < 1000 { nextMonitorId = 1000 }
        for i in scenes.indices where scenes[i].monitorId == 0 {
            scenes[i].monitorId = nextMonitorId
            nextMonitorId += 1
        }
        for i in multiviews.indices where multiviews[i].monitorId == 0 {
            multiviews[i].monitorId = nextMonitorId
            nextMonitorId += 1
        }
    }
}

enum SessionFile {
    static func encode(_ session: MixerSessionData) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return try encoder.encode(session)
    }

    static func decode(_ data: Data) throws -> MixerSessionData {
        var session = try JSONDecoder().decode(MixerSessionData.self, from: data)
        session.assignMonitors()
        return session
    }
}

struct AudioDevice: Identifiable, Hashable {
    var kind: UInt32
    var channels: UInt32
    var id: String
    var name: String
}

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
    case mix = "Mix"

    var category: String {
        switch self {
        case .color, .bars, .black: return "Colours"
        case .still: return "Still"
        case .video: return "Video"
        case .omt: return "OMT"
        case .ndi: return "NDI®"
        case .uvc: return "Video Capture"
        case .mix: return "Mix"
        }
    }
}

enum MixSource: String, Codable {
    case muPreview = "MuPreview"
    case muProgram = "MuProgram"
    case sessionMultiview = "SessionMultiview"

    var sourceKind: UInt32 {
        switch self {
        case .muPreview: return EIVIZ_SRC_KIND_MU_PREVIEW
        case .sessionMultiview: return EIVIZ_SRC_KIND_MU_MULTIVIEW
        case .muProgram: return EIVIZ_SRC_KIND_MU_PROGRAM
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

    func encoded(_ sourceId: UInt64) -> UInt64 {
        switch self {
        case .none: return 0
        case .input, .scene: return sourceId
        case .muPreview: return EIVIZ_MU_SOURCE_FLAG | EIVIZ_MU_BUS_PREVIEW | (sourceId & EIVIZ_MU_ID_MASK)
        case .muProgram: return EIVIZ_MU_SOURCE_FLAG | (sourceId & EIVIZ_MU_ID_MASK)
        }
    }
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
    var labelFollow: Bool = true
    var label: String = ""

    enum CodingKeys: String, CodingKey {
        case kind, sourceId, labelFollow, label
    }

    init(kind: MvSlotKind = .none, sourceId: UInt64 = 0, labelFollow: Bool = true, label: String = "") {
        self.kind = kind
        self.sourceId = sourceId
        self.labelFollow = labelFollow
        self.label = label
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.decodeIfPresent(MvSlotKind.self, forKey: .kind) ?? .none
        sourceId = try container.decodeIfPresent(UInt64.self, forKey: .sourceId) ?? 0
        labelFollow = try container.decodeIfPresent(Bool.self, forKey: .labelFollow) ?? true
        label = try container.decodeIfPresent(String.self, forKey: .label) ?? ""
    }
}

enum VideoPlayWhen: String, Codable, Hashable {
    case never = "Never"
    case onActive = "OnActive"
    case onPreview = "OnPreview"
    case always = "Always"
}

enum VideoTriggerWhen: String, Codable, Hashable {
    case never = "Never"
    case onActive = "OnActive"
    case onDeactivated = "OnDeactivated"
    case onPreview = "OnPreview"
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
    var toneHz: Float = 0
    var toneLevelDbfs: Float = -20
    var busMask: UInt32 = 1
    var gain: Float = 1
    var mute: Bool = false
    var useGpu: Bool = true
    var frameBufferFrames: UInt32 = 1
    var bandwidthSave: BandwidthSave = .notOnPreviewOrProgram
    var keepFullOnMultiview: Bool = false
    var omtQuality: OmtQuality = .default
    var ndiBandwidth: NdiBandwidth = .highest
    var videoLoop: Bool = true
    var videoPlayWhen: VideoPlayWhen = .never
    var videoRestartWhen: VideoTriggerWhen = .never
    var videoPauseWhen: VideoTriggerWhen = .never
    var guid: String = UUID().uuidString
    var captureWidth: UInt32 = 0
    var captureHeight: UInt32 = 0
    var captureFpsNum: UInt32 = 0
    var captureFpsDen: UInt32 = 0
    var tags: [String] = []
    var mixSource: MixSource = .muProgram
    var mixTargetId: UInt64 = 0
    var mixAudioBusId: UInt64 = 0
    var isBuiltin: Bool { id <= EIVIZ_SRC_BLUE }
    var videoStartsPlaying: Bool { videoPlayWhen == .never || videoPlayWhen == .always }

    enum CodingKeys: String, CodingKey {
        case id, name, kind, pathOrAddress, colorR, colorG, colorB, scroll, toneHz, toneLevelDbfs
        case busMask, gain, mute, useGpu, frameBufferFrames, bandwidthSave
        case keepFullOnMultiview, omtQuality, ndiBandwidth
        case videoLoop, videoPlayWhen, videoRestartWhen, videoPauseWhen
        case guid, captureWidth, captureHeight, captureFpsNum, captureFpsDen, tags
        case mixSource, mixTargetId, mixAudioBusId
    }

    init(
        id: UInt64,
        name: String,
        kind: InputKind,
        pathOrAddress: String? = nil,
        colorR: Float = 1,
        colorG: Float = 0,
        colorB: Float = 0,
        scroll: Bool = false,
        toneHz: Float = 0,
        toneLevelDbfs: Float = -20,
        busMask: UInt32 = 1,
        gain: Float = 1,
        mute: Bool = false,
        useGpu: Bool = true,
        frameBufferFrames: UInt32 = 1,
        bandwidthSave: BandwidthSave = .notOnPreviewOrProgram,
        keepFullOnMultiview: Bool = false,
        omtQuality: OmtQuality = .default,
        ndiBandwidth: NdiBandwidth = .highest,
        videoLoop: Bool = true,
        videoPlayWhen: VideoPlayWhen = .never,
        videoRestartWhen: VideoTriggerWhen = .never,
        videoPauseWhen: VideoTriggerWhen = .never
    ) {
        self.id = id
        self.name = name
        self.kind = kind
        self.pathOrAddress = pathOrAddress
        self.colorR = colorR
        self.colorG = colorG
        self.colorB = colorB
        self.scroll = scroll
        self.toneHz = toneHz
        self.toneLevelDbfs = toneLevelDbfs
        self.busMask = busMask
        self.gain = gain
        self.mute = mute
        self.useGpu = useGpu
        self.frameBufferFrames = frameBufferFrames
        self.bandwidthSave = bandwidthSave
        self.keepFullOnMultiview = keepFullOnMultiview
        self.omtQuality = omtQuality
        self.ndiBandwidth = ndiBandwidth
        self.videoLoop = videoLoop
        self.videoPlayWhen = videoPlayWhen
        self.videoRestartWhen = videoRestartWhen
        self.videoPauseWhen = videoPauseWhen
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UInt64.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        kind = try container.decode(InputKind.self, forKey: .kind)
        pathOrAddress = try container.decodeIfPresent(String.self, forKey: .pathOrAddress)
        colorR = try container.decodeIfPresent(Float.self, forKey: .colorR) ?? 1
        colorG = try container.decodeIfPresent(Float.self, forKey: .colorG) ?? 0
        colorB = try container.decodeIfPresent(Float.self, forKey: .colorB) ?? 0
        scroll = try container.decodeIfPresent(Bool.self, forKey: .scroll) ?? false
        toneHz = try container.decodeIfPresent(Float.self, forKey: .toneHz) ?? 0
        toneLevelDbfs = try container.decodeIfPresent(Float.self, forKey: .toneLevelDbfs) ?? -20
        busMask = try container.decodeIfPresent(UInt32.self, forKey: .busMask) ?? 1
        gain = try container.decodeIfPresent(Float.self, forKey: .gain) ?? 1
        mute = try container.decodeIfPresent(Bool.self, forKey: .mute) ?? false
        useGpu = try container.decodeIfPresent(Bool.self, forKey: .useGpu) ?? true
        frameBufferFrames = try container.decodeIfPresent(UInt32.self, forKey: .frameBufferFrames) ?? 1
        bandwidthSave = try container.decodeIfPresent(BandwidthSave.self, forKey: .bandwidthSave) ?? .notOnPreviewOrProgram
        keepFullOnMultiview = try container.decodeIfPresent(Bool.self, forKey: .keepFullOnMultiview) ?? false
        omtQuality = try container.decodeIfPresent(OmtQuality.self, forKey: .omtQuality) ?? .default
        ndiBandwidth = try container.decodeIfPresent(NdiBandwidth.self, forKey: .ndiBandwidth) ?? .highest
        videoLoop = try container.decodeIfPresent(Bool.self, forKey: .videoLoop) ?? true
        videoPlayWhen = try container.decodeIfPresent(VideoPlayWhen.self, forKey: .videoPlayWhen) ?? .never
        videoRestartWhen = try container.decodeIfPresent(VideoTriggerWhen.self, forKey: .videoRestartWhen) ?? .never
        videoPauseWhen = try container.decodeIfPresent(VideoTriggerWhen.self, forKey: .videoPauseWhen) ?? .never
        guid = try container.decodeIfPresent(String.self, forKey: .guid) ?? UUID().uuidString
        captureWidth = try container.decodeIfPresent(UInt32.self, forKey: .captureWidth) ?? 0
        captureHeight = try container.decodeIfPresent(UInt32.self, forKey: .captureHeight) ?? 0
        captureFpsNum = try container.decodeIfPresent(UInt32.self, forKey: .captureFpsNum) ?? 0
        captureFpsDen = try container.decodeIfPresent(UInt32.self, forKey: .captureFpsDen) ?? 0
        tags = try container.decodeIfPresent([String].self, forKey: .tags) ?? []
        mixSource = try container.decodeIfPresent(MixSource.self, forKey: .mixSource) ?? .muProgram
        mixTargetId = try container.decodeIfPresent(UInt64.self, forKey: .mixTargetId) ?? 0
        mixAudioBusId = try container.decodeIfPresent(UInt64.self, forKey: .mixAudioBusId) ?? 0
        if kind != .mix {
            mixSource = .muProgram
            mixTargetId = 0
            mixAudioBusId = 0
        } else {
            busMask = 0
        }
    }
}

enum CropEdit {
    case all, left, up, right, down
}

private func cropInsets(x: Float, y: Float, w: Float, h: Float) -> (Float, Float, Float, Float) {
    (x, y, 1 - x - w, 1 - y - h)
}

private func applyCropClamp(
    cropX: inout Float,
    cropY: inout Float,
    cropWidth: inout Float,
    cropHeight: inout Float,
    minX: Float,
    minY: Float,
    edit: CropEdit,
    value: Float? = nil
) {
    var (left, up, right, down) = cropInsets(x: cropX, y: cropY, w: cropWidth, h: cropHeight)
    if let value {
        switch edit {
        case .left: left = value
        case .up: up = value
        case .right: right = value
        case .down: down = value
        case .all: break
        }
    }
    left = max(0, left)
    up = max(0, up)
    right = max(0, right)
    down = max(0, down)
    switch edit {
    case .left:
        left = min(max(0, 1 - right), max(0, left))
    case .up:
        up = min(max(0, 1 - down), max(0, up))
    case .right:
        right = min(max(0, 1 - left), max(0, right))
    case .down:
        down = min(max(0, 1 - up), max(0, down))
    case .all:
        left = min(1, max(0, left))
        up = min(1, max(0, up))
        right = min(max(0, 1 - left), max(0, right))
        down = min(max(0, 1 - up), max(0, down))
    }
    cropX = left
    cropY = up
    cropWidth = max(minX, 1 - left - right)
    cropHeight = max(minY, 1 - up - down)
}

struct SceneLayer: Identifiable, Codable, Equatable, Hashable {
    var id = UUID()
    var inputId: UInt64
    var x: Float = 0
    var y: Float = 0
    var width: Float = 1
    var height: Float = 1
    var opacity: Float = 1
    var z: Int32 = 0
    var audioFollow: Bool = true
    var locked: Bool = false
    var hidden: Bool = false
    var sizeLinked: Bool = true
    var cropX: Float = 0
    var cropY: Float = 0
    var cropWidth: Float = 1
    var cropHeight: Float = 1

    mutating func clampCrop(minX: Float = 0, minY: Float = 0, edit: CropEdit = .all) {
        applyCropClamp(
            cropX: &cropX,
            cropY: &cropY,
            cropWidth: &cropWidth,
            cropHeight: &cropHeight,
            minX: minX,
            minY: minY,
            edit: edit
        )
    }

    mutating func setCropInset(_ value: Float, edit: CropEdit) {
        applyCropClamp(
            cropX: &cropX,
            cropY: &cropY,
            cropWidth: &cropWidth,
            cropHeight: &cropHeight,
            minX: 0,
            minY: 0,
            edit: edit,
            value: value
        )
    }

    mutating func resetLayout() {
        x = 0
        y = 0
        width = 1
        height = 1
        sizeLinked = true
        resetLayoutExtras()
    }

    mutating func resetLayoutExtras() {
        opacity = 1
        cropX = 0
        cropY = 0
        cropWidth = 1
        cropHeight = 1
    }

    enum CodingKeys: String, CodingKey {
        case inputId, x, y, width, height, opacity, z, audioFollow, locked, hidden, sizeLinked, cropX, cropY, cropWidth, cropHeight
    }

    init(inputId: UInt64, x: Float = 0, y: Float = 0, width: Float = 1, height: Float = 1, opacity: Float = 1, z: Int32 = 0, audioFollow: Bool = true, locked: Bool = false, hidden: Bool = false, sizeLinked: Bool = true, cropX: Float = 0, cropY: Float = 0, cropWidth: Float = 1, cropHeight: Float = 1) {
        self.id = UUID()
        self.inputId = inputId
        self.x = x
        self.y = y
        self.width = width
        self.height = height
        self.opacity = opacity
        self.z = z
        self.audioFollow = audioFollow
        self.locked = locked
        self.hidden = hidden
        self.sizeLinked = sizeLinked
        self.cropX = cropX
        self.cropY = cropY
        self.cropWidth = cropWidth
        self.cropHeight = cropHeight
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        inputId = try container.decode(UInt64.self, forKey: .inputId)
        x = try container.decodeIfPresent(Float.self, forKey: .x) ?? 0
        y = try container.decodeIfPresent(Float.self, forKey: .y) ?? 0
        width = try container.decodeIfPresent(Float.self, forKey: .width) ?? 1
        height = try container.decodeIfPresent(Float.self, forKey: .height) ?? 1
        opacity = try container.decodeIfPresent(Float.self, forKey: .opacity) ?? 1
        z = try container.decodeIfPresent(Int32.self, forKey: .z) ?? 0
        audioFollow = try container.decodeIfPresent(Bool.self, forKey: .audioFollow) ?? true
        locked = try container.decodeIfPresent(Bool.self, forKey: .locked) ?? false
        hidden = try container.decodeIfPresent(Bool.self, forKey: .hidden) ?? false
        sizeLinked = try container.decodeIfPresent(Bool.self, forKey: .sizeLinked) ?? true
        cropX = try container.decodeIfPresent(Float.self, forKey: .cropX) ?? 0
        cropY = try container.decodeIfPresent(Float.self, forKey: .cropY) ?? 0
        cropWidth = try container.decodeIfPresent(Float.self, forKey: .cropWidth) ?? 1
        cropHeight = try container.decodeIfPresent(Float.self, forKey: .cropHeight) ?? 1
    }
}

struct SceneEntry: Identifiable, Codable {
    var id: UInt64
    var guid: String = UUID().uuidString
    var name: String
    var monitorId: UInt64 = 0
    var layers: [SceneLayer] = []
    var tags: [String] = []
    var previewCollapsed: Bool = false
    var gpuId: UInt64 { EIVIZ_SCENE_BASE | id }

    enum CodingKeys: String, CodingKey {
        case id, guid, name, layers, tags, previewCollapsed
    }

    init(
        id: UInt64,
        guid: String = UUID().uuidString,
        name: String,
        monitorId: UInt64 = 0,
        layers: [SceneLayer] = [],
        tags: [String] = [],
        previewCollapsed: Bool = false
    ) {
        self.id = id
        self.guid = guid
        self.name = name
        self.monitorId = monitorId
        self.layers = layers
        self.tags = tags
        self.previewCollapsed = previewCollapsed
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UInt64.self, forKey: .id)
        guid = try container.decodeIfPresent(String.self, forKey: .guid) ?? UUID().uuidString
        name = try container.decode(String.self, forKey: .name)
        layers = try container.decodeIfPresent([SceneLayer].self, forKey: .layers) ?? []
        tags = try container.decodeIfPresent([String].self, forKey: .tags) ?? []
        previewCollapsed = try container.decodeIfPresent(Bool.self, forKey: .previewCollapsed) ?? false
    }
}

struct SceneLayoutPreset: Identifiable, Codable {
    var id = UUID()
    var name: String
    var layers: [SceneLayer]

    enum CodingKeys: String, CodingKey {
        case name, layers
    }
}

struct TransitionPreset: Identifiable, Codable {
    var id = UUID()
    var kind: UInt32 = EIVIZ_TRANSITION_FADE
    var durationValue: UInt32 = 30
    var durationUnit: UInt32 = 0
    var swap: Bool = true
    var keepPreview: Bool = true
    var easing: UInt32 = 0
    var direction: UInt32 = 0
    var dipR: Float = 0
    var dipG: Float = 0
    var dipB: Float = 0
    var dipA: Float = 1
    var softness: Float = 0.02
    var param: Float = 0
    var customWgsl: String?
    var durationLabel: String { "\(durationValue)\(durationUnit == EIVIZ_DURATION_MS ? "ms" : "f")" }
    var hasDuration: Bool { kind != EIVIZ_TRANSITION_CUT }
    var hasEasing: Bool { hasDuration }
    var hasDirection: Bool { TransitionCatalog.info(kind).hasDirection }
    var hasDipColor: Bool { TransitionCatalog.info(kind).hasDipColor }
    var hasSoftness: Bool { TransitionCatalog.showsSoftness(kind) }
    var hasParam: Bool { TransitionCatalog.info(kind).hasParam }
    var hasCustomWgsl: Bool { kind == EIVIZ_TRANSITION_CUSTOM }
    var label: String { TransitionCatalog.label(kind) }

    enum CodingKeys: String, CodingKey {
        case kind, durationValue, durationUnit, swap, keepPreview, easing, direction, dipR, dipG, dipB, dipA, softness, param, customWgsl
    }

    init(
        kind: UInt32 = EIVIZ_TRANSITION_FADE,
        durationValue: UInt32 = 30,
        durationUnit: UInt32 = 0,
        swap: Bool = true,
        keepPreview: Bool = true,
        easing: UInt32 = 0,
        direction: UInt32 = 0,
        dipR: Float = 0,
        dipG: Float = 0,
        dipB: Float = 0,
        dipA: Float = 1,
        softness: Float = 0.02,
        param: Float = 0,
        customWgsl: String? = nil
    ) {
        self.kind = kind
        self.durationValue = durationValue
        self.durationUnit = durationUnit
        self.swap = swap
        self.keepPreview = keepPreview
        self.easing = easing
        self.direction = direction
        self.dipR = dipR
        self.dipG = dipG
        self.dipB = dipB
        self.dipA = dipA
        self.softness = softness
        self.param = param
        self.customWgsl = customWgsl
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.decodeIfPresent(UInt32.self, forKey: .kind) ?? EIVIZ_TRANSITION_FADE
        durationValue = try container.decodeIfPresent(UInt32.self, forKey: .durationValue) ?? 30
        durationUnit = try container.decodeIfPresent(UInt32.self, forKey: .durationUnit) ?? 0
        swap = try container.decodeIfPresent(Bool.self, forKey: .swap) ?? true
        keepPreview = try container.decodeIfPresent(Bool.self, forKey: .keepPreview) ?? true
        easing = try container.decodeIfPresent(UInt32.self, forKey: .easing) ?? 0
        direction = try container.decodeIfPresent(UInt32.self, forKey: .direction) ?? 0
        dipR = try container.decodeIfPresent(Float.self, forKey: .dipR) ?? 0
        dipG = try container.decodeIfPresent(Float.self, forKey: .dipG) ?? 0
        dipB = try container.decodeIfPresent(Float.self, forKey: .dipB) ?? 0
        dipA = try container.decodeIfPresent(Float.self, forKey: .dipA) ?? 1
        softness = try container.decodeIfPresent(Float.self, forKey: .softness) ?? 0.02
        param = try container.decodeIfPresent(Float.self, forKey: .param) ?? 0
        customWgsl = try container.decodeIfPresent(String.self, forKey: .customWgsl)
    }
}

enum OverlaySourceKind: String, Codable {
    case scene = "Scene"
    case input = "Input"
}

struct OverlaySlot: Identifiable, Codable, Equatable, Hashable {
    var id = UUID()
    var sourceKind: OverlaySourceKind = .scene
    var sceneGpuId: UInt64 = 0
    var x: Float = 0.62
    var y: Float = 0.08
    var width: Float = 0.32
    var height: Float = 0.32
    var opacity: Float = 1
    var z: Int32 = 0
    var enabled: Bool = false
    var transitionKind: UInt32 = EIVIZ_TRANSITION_FADE
    var durationValue: UInt32 = 15
    var durationUnit: UInt32 = 0
    var audioFollow: Bool = true
    var locked: Bool = false
    var hidden: Bool = false
    var sizeLinked: Bool = true
    var cropX: Float = 0
    var cropY: Float = 0
    var cropWidth: Float = 1
    var cropHeight: Float = 1

    mutating func clampCrop(minX: Float = 0, minY: Float = 0, edit: CropEdit = .all) {
        applyCropClamp(
            cropX: &cropX,
            cropY: &cropY,
            cropWidth: &cropWidth,
            cropHeight: &cropHeight,
            minX: minX,
            minY: minY,
            edit: edit
        )
    }

    mutating func setCropInset(_ value: Float, edit: CropEdit) {
        applyCropClamp(
            cropX: &cropX,
            cropY: &cropY,
            cropWidth: &cropWidth,
            cropHeight: &cropHeight,
            minX: 0,
            minY: 0,
            edit: edit,
            value: value
        )
    }

    mutating func resetLayout() {
        x = 0.62
        y = 0.08
        width = 0.32
        height = 0.32
        sizeLinked = true
        opacity = 1
        cropX = 0
        cropY = 0
        cropWidth = 1
        cropHeight = 1
    }

    enum CodingKeys: String, CodingKey {
        case sourceKind, sceneGpuId, x, y, width, height, opacity, z, enabled, transitionKind, durationValue, durationUnit, audioFollow, locked, hidden, sizeLinked, cropX, cropY, cropWidth, cropHeight
    }

    init(sourceKind: OverlaySourceKind = .scene, sceneGpuId: UInt64 = 0) {
        self.sourceKind = sourceKind
        self.sceneGpuId = sceneGpuId
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        sourceKind = try container.decodeIfPresent(OverlaySourceKind.self, forKey: .sourceKind) ?? .scene
        sceneGpuId = try container.decodeIfPresent(UInt64.self, forKey: .sceneGpuId) ?? 0
        x = try container.decodeIfPresent(Float.self, forKey: .x) ?? 0.62
        y = try container.decodeIfPresent(Float.self, forKey: .y) ?? 0.08
        width = try container.decodeIfPresent(Float.self, forKey: .width) ?? 0.32
        height = try container.decodeIfPresent(Float.self, forKey: .height) ?? 0.32
        opacity = try container.decodeIfPresent(Float.self, forKey: .opacity) ?? 1
        z = try container.decodeIfPresent(Int32.self, forKey: .z) ?? 0
        enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        transitionKind = try container.decodeIfPresent(UInt32.self, forKey: .transitionKind) ?? EIVIZ_TRANSITION_FADE
        durationValue = try container.decodeIfPresent(UInt32.self, forKey: .durationValue) ?? 15
        durationUnit = try container.decodeIfPresent(UInt32.self, forKey: .durationUnit) ?? 0
        audioFollow = try container.decodeIfPresent(Bool.self, forKey: .audioFollow) ?? true
        locked = try container.decodeIfPresent(Bool.self, forKey: .locked) ?? false
        hidden = try container.decodeIfPresent(Bool.self, forKey: .hidden) ?? false
        sizeLinked = try container.decodeIfPresent(Bool.self, forKey: .sizeLinked) ?? true
        cropX = try container.decodeIfPresent(Float.self, forKey: .cropX) ?? 0
        cropY = try container.decodeIfPresent(Float.self, forKey: .cropY) ?? 0
        cropWidth = try container.decodeIfPresent(Float.self, forKey: .cropWidth) ?? 1
        cropHeight = try container.decodeIfPresent(Float.self, forKey: .cropHeight) ?? 1
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
    var audioBusId: UInt64 = 1
    var audioLink: AudioLinkMode = .follow
    var alwaysOnTop: Bool = true
    var switcherSceneFilter: SwitcherSceneFilter = .all
    var switcherSceneIds: [UInt64] = []
    var displayName: String { "\(name)  \(width)x\(height) \(fpsLabel)" }

    func showsOnSwitcher(_ scene: SceneEntry) -> Bool {
        switch switcherSceneFilter {
        case .all: return true
        case .include: return switcherSceneIds.contains(scene.id)
        case .exclude: return !switcherSceneIds.contains(scene.id)
        }
    }
    var fpsLabel: String {
        if fpsNum == 60_000 && fpsDen == 1_001 { return "59.94p" }
        if fpsDen == 1 { return "\(fpsNum)p" }
        return "\(fpsNum)/\(fpsDen)"
    }
    func durationMs(_ frames: UInt32) -> UInt32 {
        UInt32(max(1, (Double(frames) * 1000.0 * Double(fpsDen) / Double(fpsNum)).rounded()))
    }

    func durationMs(for preset: TransitionPreset) -> UInt32 {
        preset.durationUnit == 1 ? max(1, preset.durationValue) : durationMs(preset.durationValue)
    }

    init(id: UInt64, name: String) {
        self.id = id
        self.name = name
    }

    enum CodingKeys: String, CodingKey {
        case id, name, width, height, fpsNum, fpsDen, transitions, overlays, audioBusId, audioLink, alwaysOnTop
        case switcherSceneFilter, switcherSceneIds
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UInt64.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        width = try container.decodeIfPresent(UInt32.self, forKey: .width) ?? 1920
        height = try container.decodeIfPresent(UInt32.self, forKey: .height) ?? 1080
        fpsNum = try container.decodeIfPresent(UInt32.self, forKey: .fpsNum) ?? 60_000
        fpsDen = try container.decodeIfPresent(UInt32.self, forKey: .fpsDen) ?? 1_001
        transitions = try container.decodeIfPresent([TransitionPreset].self, forKey: .transitions) ?? []
        overlays = try container.decodeIfPresent([OverlaySlot].self, forKey: .overlays) ?? []
        audioBusId = try container.decodeIfPresent(UInt64.self, forKey: .audioBusId) ?? 1
        audioLink = try container.decodeIfPresent(AudioLinkMode.self, forKey: .audioLink) ?? .follow
        alwaysOnTop = try container.decodeIfPresent(Bool.self, forKey: .alwaysOnTop) ?? true
        switcherSceneFilter = try container.decodeIfPresent(SwitcherSceneFilter.self, forKey: .switcherSceneFilter) ?? .all
        switcherSceneIds = try container.decodeIfPresent([UInt64].self, forKey: .switcherSceneIds) ?? []
    }
}

enum SwitcherSceneFilter: String, Codable {
    case all = "All"
    case include = "Include"
    case exclude = "Exclude"
}

struct OutputEntry: Identifiable, Codable {
    var id: UInt64
    var name: String
    var transport: OutputTransport = .omt
    var sourceKind: OutputSourceKind = .muProgram
    var sourceId: UInt64 = 0
    var unitId: UInt64 = 1
    var useGpu: Bool = true
    var enabled: Bool = true

    enum CodingKeys: String, CodingKey {
        case id, name, transport, sourceKind, sourceId, unitId, useGpu, enabled
    }

    init(
        id: UInt64,
        name: String,
        transport: OutputTransport = .omt,
        sourceKind: OutputSourceKind = .muProgram,
        sourceId: UInt64 = 0,
        unitId: UInt64 = 1,
        useGpu: Bool = true,
        enabled: Bool = true
    ) {
        self.id = id
        self.name = name
        self.transport = transport
        self.sourceKind = sourceKind
        self.sourceId = sourceId
        self.unitId = unitId
        self.useGpu = useGpu
        self.enabled = enabled
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UInt64.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        transport = try container.decodeIfPresent(OutputTransport.self, forKey: .transport) ?? .omt
        sourceKind = try container.decodeIfPresent(OutputSourceKind.self, forKey: .sourceKind) ?? .muProgram
        sourceId = try container.decodeIfPresent(UInt64.self, forKey: .sourceId) ?? 0
        unitId = try container.decodeIfPresent(UInt64.self, forKey: .unitId) ?? 1
        useGpu = try container.decodeIfPresent(Bool.self, forKey: .useGpu) ?? true
        enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
    }
}

enum MultiviewTemplate: String, Codable, CaseIterable, Identifiable {
    case previewProgram8 = "PreviewProgram8"
    case previewProgram8Bottom = "PreviewProgram8Bottom"
    case previewProgram8Left = "PreviewProgram8Left"
    case previewProgram8Right = "PreviewProgram8Right"
    case previewProgram2 = "PreviewProgram2"
    case quad4TopLeft = "Quad4TopLeft"
    case quad4TopRight = "Quad4TopRight"
    case quad4BottomLeft = "Quad4BottomLeft"
    case quad4BottomRight = "Quad4BottomRight"
    case large5TopLeft = "Large5TopLeft"
    case large5TopRight = "Large5TopRight"
    case large5BottomLeft = "Large5BottomLeft"
    case large5BottomRight = "Large5BottomRight"
    case grid2x2 = "Grid2x2"
    case grid3x3 = "Grid3x3"
    case grid4x4 = "Grid4x4"

    var id: String { rawValue }

    static let groups: [(title: String, items: [MultiviewTemplate])] = [
        ("Preview + Program + 8", [.previewProgram8, .previewProgram8Bottom, .previewProgram8Left, .previewProgram8Right]),
        ("Preview + Program + 2", [.previewProgram2]),
        ("1 + 5", [.large5TopLeft, .large5TopRight, .large5BottomLeft, .large5BottomRight]),
        ("3 + 4", [.quad4TopLeft, .quad4TopRight, .quad4BottomLeft, .quad4BottomRight]),
        ("Grid", [.grid2x2, .grid3x3, .grid4x4])
    ]

    static var choices: [MultiviewTemplate] { groups.flatMap(\.items) }

    var title: String {
        switch self {
        case .previewProgram8: return "Buses on top"
        case .previewProgram8Bottom: return "Buses on bottom"
        case .previewProgram8Left: return "Buses on left"
        case .previewProgram8Right: return "Buses on right"
        case .previewProgram2: return "Buses on top"
        case .large5TopLeft: return "Large top-left"
        case .large5TopRight: return "Large top-right"
        case .large5BottomLeft: return "Large bottom-left"
        case .large5BottomRight: return "Large bottom-right"
        case .quad4TopLeft: return "Four top-left"
        case .quad4TopRight: return "Four top-right"
        case .quad4BottomLeft: return "Four bottom-left"
        case .quad4BottomRight: return "Four bottom-right"
        case .grid2x2: return "2×2"
        case .grid3x3: return "3×3"
        case .grid4x4: return "4×4"
        default: return rawValue
        }
    }

    var tileCount: Int {
        switch self {
        case .previewProgram2, .grid2x2: return 4
        case .quad4TopLeft, .quad4TopRight, .quad4BottomLeft, .quad4BottomRight: return 7
        case .large5TopLeft, .large5TopRight, .large5BottomLeft, .large5BottomRight: return 6
        case .grid3x3: return 9
        case .grid4x4: return 16
        default: return 10
        }
    }

    var panes: [MultiviewPane] {
        switch self {
        case .previewProgram2:
            return [
                MultiviewPane(x: 0, y: 0, width: 0.5, height: 0.5),
                MultiviewPane(x: 0.5, y: 0, width: 0.5, height: 0.5)
            ] + MultiviewPane.grid(cols: 2, rows: 1, x: 0, y: 0.5, width: 1, height: 0.5)
        case .previewProgram8:
            return [
                MultiviewPane(x: 0, y: 0, width: 0.5, height: 0.5),
                MultiviewPane(x: 0.5, y: 0, width: 0.5, height: 0.5)
            ] + MultiviewPane.grid(cols: 4, rows: 2, x: 0, y: 0.5, width: 1, height: 0.5)
        case .previewProgram8Bottom:
            return MultiviewPane.grid(cols: 4, rows: 2, x: 0, y: 0, width: 1, height: 0.5) + [
                MultiviewPane(x: 0, y: 0.5, width: 0.5, height: 0.5),
                MultiviewPane(x: 0.5, y: 0.5, width: 0.5, height: 0.5)
            ]
        case .previewProgram8Left:
            return [
                MultiviewPane(x: 0, y: 0.5, width: 0.5, height: 0.5),
                MultiviewPane(x: 0, y: 0, width: 0.5, height: 0.5)
            ] + MultiviewPane.grid(cols: 2, rows: 4, x: 0.5, y: 0, width: 0.5, height: 1)
        case .previewProgram8Right:
            return MultiviewPane.grid(cols: 2, rows: 4, x: 0, y: 0, width: 0.5, height: 1) + [
                MultiviewPane(x: 0.5, y: 0.5, width: 0.5, height: 0.5),
                MultiviewPane(x: 0.5, y: 0, width: 0.5, height: 0.5)
            ]
        case .quad4TopLeft: return MultiviewPane.quad4(smallQuad: 0)
        case .quad4TopRight: return MultiviewPane.quad4(smallQuad: 1)
        case .quad4BottomLeft: return MultiviewPane.quad4(smallQuad: 2)
        case .quad4BottomRight: return MultiviewPane.quad4(smallQuad: 3)
        case .large5TopLeft: return MultiviewPane.large5(largeCol: 0, largeRow: 0)
        case .large5TopRight: return MultiviewPane.large5(largeCol: 1, largeRow: 0)
        case .large5BottomLeft: return MultiviewPane.large5(largeCol: 0, largeRow: 1)
        case .large5BottomRight: return MultiviewPane.large5(largeCol: 1, largeRow: 1)
        case .grid3x3:
            return MultiviewPane.grid(cols: 3, rows: 3, x: 0, y: 0, width: 1, height: 1)
        case .grid4x4:
            return MultiviewPane.grid(cols: 4, rows: 4, x: 0, y: 0, width: 1, height: 1)
        default:
            return MultiviewPane.grid(cols: 2, rows: 2, x: 0, y: 0, width: 1, height: 1)
        }
    }
}

struct MultiviewPane {
    var x: Float
    var y: Float
    var width: Float
    var height: Float

    static func grid(cols: Int, rows: Int, x: Float, y: Float, width: Float, height: Float) -> [MultiviewPane] {
        return (0..<(cols * rows)).map { i in
            let col = i % cols
            let row = i / cols
            let x0 = x + width * Float(col) / Float(cols)
            let y0 = y + height * Float(row) / Float(rows)
            let x1 = x + width * Float(col + 1) / Float(cols)
            let y1 = y + height * Float(row + 1) / Float(rows)
            return MultiviewPane(x: x0, y: y0, width: x1 - x0, height: y1 - y0)
        }
    }

    static func quad4(smallQuad: Int) -> [MultiviewPane] {
        (0..<4).flatMap { quad -> [MultiviewPane] in
            let x = Float(quad % 2) * 0.5
            let y = Float(quad / 2) * 0.5
            if quad == smallQuad {
                return grid(cols: 2, rows: 2, x: x, y: y, width: 0.5, height: 0.5)
            }
            return [MultiviewPane(x: x, y: y, width: 0.5, height: 0.5)]
        }
    }

    static func large5(largeCol: Int, largeRow: Int) -> [MultiviewPane] {
        let x0 = Float(largeCol) / 3
        let y0 = Float(largeRow) / 3
        let x1 = Float(largeCol + 2) / 3
        let y1 = Float(largeRow + 2) / 3
        var panes = [MultiviewPane(x: x0, y: y0, width: x1 - x0, height: y1 - y0)]
        for row in 0..<3 {
            for col in 0..<3 {
                if col >= largeCol && col < largeCol + 2 && row >= largeRow && row < largeRow + 2 {
                    continue
                }
                let sx0 = Float(col) / 3
                let sy0 = Float(row) / 3
                let sx1 = Float(col + 1) / 3
                let sy1 = Float(row + 1) / 3
                panes.append(MultiviewPane(x: sx0, y: sy0, width: sx1 - sx0, height: sy1 - sy0))
            }
        }
        return panes
    }
}

struct MultiviewLayout: Identifiable, Codable {
    var id: UInt64
    var name: String
    var monitorId: UInt64 = 0
    var previewUnitId: UInt64 = 1
    var programUnitId: UInt64 = 1
    var presentInterval: UInt32 = 0
    var template: MultiviewTemplate = .previewProgram8
    var tiles: [MvSlot] = Array(repeating: MvSlot(), count: 10)
    var previewLabelFollow: Bool = true
    var previewLabel: String = ""
    var programLabelFollow: Bool = true
    var programLabel: String = ""
    var labelAnchor: MvLabelAnchor?
    var labelSize: Float?
    var labelUnit: MvLabelUnit?
    var alwaysOnTop: Bool = true
    var gpuId: UInt64 { EIVIZ_MULTIVIEW_BASE | id }

    enum CodingKeys: String, CodingKey {
        case id, name, previewUnitId, programUnitId, presentInterval, tiles, template
        case previewLabelFollow, previewLabel, programLabelFollow, programLabel
        case labelAnchor, labelSize, labelUnit, alwaysOnTop
    }

    init(
        id: UInt64,
        name: String,
        monitorId: UInt64 = 0,
        previewUnitId: UInt64 = 1,
        programUnitId: UInt64 = 1,
        presentInterval: UInt32 = 0,
        template: MultiviewTemplate = .previewProgram8,
        tiles: [MvSlot] = Array(repeating: MvSlot(), count: 10),
        previewLabelFollow: Bool = true,
        previewLabel: String = "",
        programLabelFollow: Bool = true,
        programLabel: String = "",
        labelAnchor: MvLabelAnchor? = nil,
        labelSize: Float? = nil,
        labelUnit: MvLabelUnit? = nil,
        alwaysOnTop: Bool = true
    ) {
        self.id = id
        self.name = name
        self.monitorId = monitorId
        self.previewUnitId = previewUnitId
        self.programUnitId = programUnitId
        self.presentInterval = presentInterval
        self.template = template
        self.tiles = tiles
        self.previewLabelFollow = previewLabelFollow
        self.previewLabel = previewLabel
        self.programLabelFollow = programLabelFollow
        self.programLabel = programLabel
        self.labelAnchor = labelAnchor
        self.labelSize = labelSize
        self.labelUnit = labelUnit
        self.alwaysOnTop = alwaysOnTop
        ensureTiles()
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UInt64.self, forKey: .id)
        name = try container.decodeIfPresent(String.self, forKey: .name) ?? "Multiview \(id)"
        previewUnitId = try container.decodeIfPresent(UInt64.self, forKey: .previewUnitId) ?? 1
        programUnitId = try container.decodeIfPresent(UInt64.self, forKey: .programUnitId) ?? 1
        presentInterval = try container.decodeIfPresent(UInt32.self, forKey: .presentInterval) ?? 0
        template = try container.decodeIfPresent(MultiviewTemplate.self, forKey: .template) ?? .previewProgram8
        tiles = try container.decodeIfPresent([MvSlot].self, forKey: .tiles) ?? []
        previewLabelFollow = try container.decodeIfPresent(Bool.self, forKey: .previewLabelFollow) ?? true
        previewLabel = try container.decodeIfPresent(String.self, forKey: .previewLabel) ?? ""
        programLabelFollow = try container.decodeIfPresent(Bool.self, forKey: .programLabelFollow) ?? true
        programLabel = try container.decodeIfPresent(String.self, forKey: .programLabel) ?? ""
        labelAnchor = try container.decodeIfPresent(MvLabelAnchor.self, forKey: .labelAnchor)
        labelSize = try container.decodeIfPresent(Float.self, forKey: .labelSize)
        labelUnit = try container.decodeIfPresent(MvLabelUnit.self, forKey: .labelUnit)
        alwaysOnTop = try container.decodeIfPresent(Bool.self, forKey: .alwaysOnTop) ?? true
        ensureTiles()
    }

    func resolvedLabelSize(_ settings: SessionSettings) -> Float {
        let size = labelSize ?? settings.multiviewLabelSize
        return min(200, max(1, size > 0 ? size : 18))
    }

    func resolvedLabelUnit(_ settings: SessionSettings) -> MvLabelUnit {
        labelUnit ?? settings.multiviewLabelUnit
    }

    func resolvedLabelAnchor(_ settings: SessionSettings) -> MvLabelAnchor {
        labelAnchor ?? settings.multiviewLabelAnchor
    }

    mutating func ensureTiles() {
        absorbFixedBusPanes()
        let want = template.tileCount
        if tiles.count < want {
            tiles.append(contentsOf: Array(repeating: MvSlot(), count: want - tiles.count))
        }
        if tiles.count > want {
            tiles.removeLast(tiles.count - want)
        }
    }

    mutating func seedDefaultBuses(_ unitId: UInt64) {
        guard unitId != 0, tiles.count >= 2, tiles.allSatisfy({ $0.kind == .none }) else { return }
        tiles[0].kind = .muPreview
        tiles[0].sourceId = unitId
        tiles[1].kind = .muProgram
        tiles[1].sourceId = unitId
    }

    private mutating func absorbFixedBusPanes() {
        let buses = [
            MvSlot(
                kind: .muPreview,
                sourceId: max(1, previewUnitId),
                labelFollow: previewLabelFollow,
                label: previewLabel
            ),
            MvSlot(
                kind: .muProgram,
                sourceId: max(1, programUnitId),
                labelFollow: programLabelFollow,
                label: programLabel
            )
        ]
        switch (template, tiles.count) {
        case (.previewProgram2, 2), (.previewProgram8, 8), (.previewProgram8Left, 8):
            tiles.insert(contentsOf: buses, at: 0)
        case (.previewProgram8Bottom, 8), (.previewProgram8Right, 8):
            tiles.append(contentsOf: buses)
        default:
            break
        }
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

struct RgbColor: Codable, Equatable, Hashable {
    var r: UInt8
    var g: UInt8
    var b: UInt8

    static let previewDefault = RgbColor(r: 0, g: 255, b: 0)
    static let programDefault = RgbColor(r: 255, g: 0, b: 0)
    static let inactiveDefault = RgbColor(r: 64, g: 64, b: 64)
}

enum MvLabelUnit: String, Codable {
    case px = "Px"
    case percent = "Percent"
}

enum MvLabelAnchor: String, Codable {
    case top = "Top"
    case bottom = "Bottom"
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
    var flipSwapchainLimit: UInt32 = 0
    var internalColorFormat: InternalColorFormat = .uyvy
    var rebarOptimization: Bool?
    var rebarDirectSample: Bool?
    var ndiGpuUpload: Bool?
    var previewColor: RgbColor = .previewDefault
    var programColor: RgbColor = .programDefault
    var inactiveColor: RgbColor = .inactiveDefault
    var multiviewLabelSize: Float = 18
    var multiviewLabelUnit: MvLabelUnit = .px
    var multiviewLabelAnchor: MvLabelAnchor = .bottom
    var lastSessionPath: String?

    var rebarOptimizationEnabled: Bool { rebarOptimization != false }
    var ndiGpuUploadEnabled: Bool { ndiGpuUpload != false }

    var resolvedPresentInterval: UInt32 {
        let frames = defaultPresentInterval == 0 ? 3 : defaultPresentInterval
        return max(1, min(8, frames))
    }

    init() {}

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        masterFpsNum = try container.decodeIfPresent(UInt32.self, forKey: .masterFpsNum) ?? 60_000
        masterFpsDen = try container.decodeIfPresent(UInt32.self, forKey: .masterFpsDen) ?? 1_001
        defaultWidth = try container.decodeIfPresent(UInt32.self, forKey: .defaultWidth) ?? 1920
        defaultHeight = try container.decodeIfPresent(UInt32.self, forKey: .defaultHeight) ?? 1080
        theme = try container.decodeIfPresent(String.self, forKey: .theme) ?? "Charcoal"
        defaultMultiviewUnitId = try container.decodeIfPresent(UInt64.self, forKey: .defaultMultiviewUnitId) ?? 1
        frameBufferFrames = try container.decodeIfPresent(UInt32.self, forKey: .frameBufferFrames) ?? 3
        defaultPresentInterval = try container.decodeIfPresent(UInt32.self, forKey: .defaultPresentInterval) ?? 3
        let flip = try container.decodeIfPresent(UInt32.self, forKey: .flipSwapchainLimit) ?? 0
        flipSwapchainLimit = [0, 4, 6, 8, 10, 12, 16].contains(flip) ? flip : 0
        internalColorFormat = try container.decodeIfPresent(InternalColorFormat.self, forKey: .internalColorFormat) ?? .uyvy
        rebarOptimization = try container.decodeIfPresent(Bool.self, forKey: .rebarOptimization)
        rebarDirectSample = try container.decodeIfPresent(Bool.self, forKey: .rebarDirectSample)
        ndiGpuUpload = try container.decodeIfPresent(Bool.self, forKey: .ndiGpuUpload)
        previewColor = try container.decodeIfPresent(RgbColor.self, forKey: .previewColor) ?? .previewDefault
        programColor = try container.decodeIfPresent(RgbColor.self, forKey: .programColor) ?? .programDefault
        inactiveColor = try container.decodeIfPresent(RgbColor.self, forKey: .inactiveColor) ?? .inactiveDefault
        multiviewLabelSize = try container.decodeIfPresent(Float.self, forKey: .multiviewLabelSize) ?? 18
        multiviewLabelUnit = try container.decodeIfPresent(MvLabelUnit.self, forKey: .multiviewLabelUnit) ?? .px
        multiviewLabelAnchor = try container.decodeIfPresent(MvLabelAnchor.self, forKey: .multiviewLabelAnchor) ?? .bottom
        lastSessionPath = try container.decodeIfPresent(String.self, forKey: .lastSessionPath)
    }
}

struct MixerSessionData: Codable {
    var version: Int = 2
    var settings = SessionSettings()
    var inputs: [InputEntry] = []
    var scenes: [SceneEntry] = []
    var scenePresets: [SceneLayoutPreset] = []
    var units: [MixingUnitEntry] = []
    var outputs: [OutputEntry] = []
    var multiviews: [MultiviewLayout] = []
    var buses: [AudioBusEntry] = []
    var inputTags: [String] = []
    var sceneTags: [String] = []
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
        case version, settings, inputs, scenes, scenePresets, units, outputs, multiviews, buses
        case inputTags, sceneTags
        case nextInputId, nextSceneId, nextUnitId, nextOutputId, nextMultiviewId, nextBusId
        case selectedUnitId, headphoneCopyMaster
    }

    static func `default`() -> MixerSessionData {
        var session = MixerSessionData()
        session.inputs = [
            InputEntry(id: EIVIZ_SRC_COLOR, name: "Color Red", kind: .color, colorR: 1),
            InputEntry(id: EIVIZ_SRC_BARS, name: "SMPTE HD Bars", kind: .bars, scroll: true, toneHz: 1000),
            InputEntry(id: EIVIZ_SRC_BLACK, name: "Black", kind: .black, colorR: 0, colorG: 0, colorB: 0),
            InputEntry(id: EIVIZ_SRC_BLUE, name: "Blue", kind: .color, colorR: 0, colorG: 0, colorB: 1)
        ]
        var unit = MixingUnitEntry(id: 1, name: "Mixing Unit 1")
        unit.transitions = [
            TransitionPreset(kind: EIVIZ_TRANSITION_CUT, durationValue: 1, swap: true),
            TransitionPreset(kind: EIVIZ_TRANSITION_FADE, durationValue: 30, swap: true)
        ]
        session.units = [unit]
        session.buses = [
            AudioBusEntry(id: 1, name: "Master", role: .master, deviceKind: .coreAudio, mapLeft: 0, mapRight: 1, bit: 0),
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

    @discardableResult
    mutating func addMultiview(unitId: UInt64) -> MultiviewLayout {
        var layout = MultiviewLayout(
            id: nextMultiviewId,
            name: "Multiview \(nextMultiviewId)",
            monitorId: nextMonitorId,
            previewUnitId: unitId,
            programUnitId: unitId,
            labelAnchor: settings.multiviewLabelAnchor,
            labelSize: settings.multiviewLabelSize,
            labelUnit: settings.multiviewLabelUnit
        )
        layout.seedDefaultBuses(unitId)
        nextMultiviewId += 1
        nextMonitorId += 1
        multiviews.append(layout)
        return layout
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
        for i in buses.indices where buses[i].deviceKind == .wasapi {
            buses[i].deviceKind = .coreAudio
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

struct VideoCaptureDevice: Identifiable, Hashable {
    var id: String
    var name: String
}

struct CaptureMode: Identifiable, Hashable {
    var width: UInt32
    var height: UInt32
    var fpsNum: UInt32
    var fpsDen: UInt32
    var format: UInt32 = 0
    var id: String { "\(width)x\(height)@\(fpsNum)/\(fpsDen)/\(format)" }
    var label: String {
        let fps = fpsDen == 0 ? 0 : Double(fpsNum) / Double(max(1, fpsDen))
        return String(format: "%ux%u %.2f fps", width, height, fps)
    }
}

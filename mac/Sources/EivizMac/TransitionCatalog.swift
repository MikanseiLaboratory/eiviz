import EivizMixer

enum TransitionGroup: Int, CaseIterable {
    case basic
    case wipe
    case motion
    case shader

    var title: String {
        switch self {
        case .basic: return "Basic"
        case .wipe: return "Wipe"
        case .motion: return "Motion"
        case .shader: return "Shader"
        }
    }
}

struct TransitionInfo: Identifiable {
    var id: UInt32 { kind }
    let kind: UInt32
    let label: String
    let group: TransitionGroup
    let hasDirection: Bool
    let hasDipColor: Bool
    let hasSoftness: Bool
    let hasParam: Bool
    let softnessLabel: String
    let paramLabel: String
}

enum TransitionCatalog {
    static let all: [TransitionInfo] = [
        .init(kind: EIVIZ_TRANSITION_CUT, label: "Cut", group: .basic, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_FADE, label: "Fade", group: .basic, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_DIP, label: "Dip", group: .basic, hasDirection: false, hasDipColor: true, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_ADDITIVE, label: "Additive", group: .basic, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_CUSTOM, label: "Custom WGSL", group: .basic, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_WIPE, label: "Wipe", group: .wipe, hasDirection: true, hasDipColor: false, hasSoftness: true, hasParam: false, softnessLabel: "Edge", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_IRIS, label: "Iris", group: .wipe, hasDirection: false, hasDipColor: false, hasSoftness: true, hasParam: false, softnessLabel: "Edge", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_BLINDS, label: "Blinds", group: .wipe, hasDirection: true, hasDipColor: false, hasSoftness: true, hasParam: true, softnessLabel: "Edge", paramLabel: "Strips"),
        .init(kind: EIVIZ_TRANSITION_BARN_DOOR, label: "BarnDoor", group: .wipe, hasDirection: true, hasDipColor: false, hasSoftness: true, hasParam: false, softnessLabel: "Edge", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_CLOCK, label: "Clock", group: .wipe, hasDirection: true, hasDipColor: false, hasSoftness: true, hasParam: false, softnessLabel: "Edge", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_HEART, label: "Heart", group: .wipe, hasDirection: false, hasDipColor: false, hasSoftness: true, hasParam: false, softnessLabel: "Edge", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_DIAMOND, label: "Diamond", group: .wipe, hasDirection: false, hasDipColor: false, hasSoftness: true, hasParam: false, softnessLabel: "Edge", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_STAR, label: "Star", group: .wipe, hasDirection: false, hasDipColor: false, hasSoftness: true, hasParam: false, softnessLabel: "Edge", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_ROLLER_DOOR, label: "RollerDoor", group: .wipe, hasDirection: true, hasDipColor: false, hasSoftness: true, hasParam: false, softnessLabel: "Edge", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_SLIDE, label: "Slide", group: .motion, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_PUSH, label: "Push", group: .motion, hasDirection: true, hasDipColor: true, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_ZOOM, label: "Zoom", group: .motion, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_CROSS_ZOOM, label: "CrossZoom", group: .motion, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_FLY_ROTATE, label: "FlyRotate", group: .motion, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: true, softnessLabel: "", paramLabel: "Spin"),
        .init(kind: EIVIZ_TRANSITION_FLIP, label: "Flip", group: .motion, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_CUBE, label: "Cube", group: .motion, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_CUBE_ZOOM, label: "CubeZoom", group: .motion, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_MULTITASK, label: "MultiTask", group: .motion, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_LOREZ, label: "LoRez", group: .shader, hasDirection: false, hasDipColor: false, hasSoftness: true, hasParam: false, softnessLabel: "Pixel size", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_METAMIX, label: "MetaMix", group: .shader, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: true, softnessLabel: "", paramLabel: "Copies"),
        .init(kind: EIVIZ_TRANSITION_TILE, label: "Tile", group: .shader, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: true, softnessLabel: "", paramLabel: "Tiles"),
        .init(kind: EIVIZ_TRANSITION_PARTS, label: "Parts", group: .shader, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: true, softnessLabel: "", paramLabel: "Chunks"),
        .init(kind: EIVIZ_TRANSITION_STATIC, label: "Static", group: .shader, hasDirection: false, hasDipColor: false, hasSoftness: true, hasParam: true, softnessLabel: "Edge", paramLabel: "Intensity"),
        .init(kind: EIVIZ_TRANSITION_SHIFT_RGB, label: "Shift RGB", group: .shader, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_DISPLACE, label: "Displace", group: .shader, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: true, softnessLabel: "", paramLabel: "Intensity"),
        .init(kind: EIVIZ_TRANSITION_GLITCH, label: "Glitch", group: .shader, hasDirection: false, hasDipColor: true, hasSoftness: true, hasParam: true, softnessLabel: "Edge", paramLabel: "Intensity"),
        .init(kind: EIVIZ_TRANSITION_SWIRL, label: "Swirl", group: .shader, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: true, softnessLabel: "", paramLabel: "Turns"),
        .init(kind: EIVIZ_TRANSITION_LUMA_MORPH, label: "LumaMorph", group: .shader, hasDirection: false, hasDipColor: false, hasSoftness: true, hasParam: false, softnessLabel: "Edge", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_RIPPLE, label: "Ripple", group: .shader, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: true, softnessLabel: "", paramLabel: "Intensity"),
        .init(kind: EIVIZ_TRANSITION_GRID_DISSOLVE, label: "GridDissolve", group: .shader, hasDirection: false, hasDipColor: false, hasSoftness: true, hasParam: true, softnessLabel: "Edge", paramLabel: "Cells"),
        .init(kind: EIVIZ_TRANSITION_POLAR, label: "Polar", group: .shader, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_KALEIDOSCOPE, label: "Kaleidoscope", group: .shader, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: true, softnessLabel: "", paramLabel: "Segments"),
        .init(kind: EIVIZ_TRANSITION_PAGE_CURL, label: "PageCurl", group: .shader, hasDirection: true, hasDipColor: false, hasSoftness: false, hasParam: false, softnessLabel: "", paramLabel: ""),
        .init(kind: EIVIZ_TRANSITION_FILM_BURN, label: "FilmBurn", group: .shader, hasDirection: false, hasDipColor: true, hasSoftness: true, hasParam: true, softnessLabel: "Edge", paramLabel: "Intensity"),
        .init(kind: EIVIZ_TRANSITION_ZOOM_BLUR, label: "ZoomBlur", group: .shader, hasDirection: false, hasDipColor: false, hasSoftness: false, hasParam: true, softnessLabel: "", paramLabel: "Intensity"),
    ]

    static func info(_ kind: UInt32) -> TransitionInfo {
        all.first { $0.kind == kind } ?? all[1]
    }

    static func label(_ kind: UInt32) -> String {
        kind == EIVIZ_TRANSITION_STINGER ? "Stinger" : info(kind).label
    }

    static func items(in group: TransitionGroup) -> [TransitionInfo] {
        all.filter { $0.group == group }
    }
}

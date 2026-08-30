import Combine
import EivizMixer
import Foundation

final class MixerSession: ObservableObject {
    static let unitId: UInt64 = 1
    static let stillId: UInt64 = 10
    static let sceneBars: UInt64 = EIVIZ_SCENE_BASE + 1
    static let sceneColor: UInt64 = EIVIZ_SCENE_BASE + 2
    static let width: UInt32 = 1920
    static let height: UInt32 = 1080

    @Published var mix: Float = 0
    @Published var tbarLocked = false
    @Published var status = "1920x1080 59.94p   Mixing Unit 1"
    @Published var errorText = ""

    private var booted = false
    private var tbarLatching = false

    func boot() {
        guard !booted else { return }
        guard mixer_ping() == 0x4549_5649 else {
            errorText = "The Rust mixer ABI does not match this host."
            return
        }
        check(mixer_create(0, 60_000, 1_001), "Metal mixer initialization")
        check(mixer_create_unit(Self.unitId, Self.width, Self.height), "Create Mixing Unit")
        defineScene(Self.sceneBars, source: EIVIZ_SRC_BARS)
        var programSource = EIVIZ_SRC_COLOR
        if CommandLine.arguments.count > 1 {
            let path = CommandLine.arguments[1]
            path.withCString { cstr in
                if mixer_load_still(Self.stillId, cstr) == EIVIZ_OK {
                    programSource = Self.stillId
                } else {
                    errorText = lastError("Still load")
                }
            }
        }
        defineScene(Self.sceneColor, source: programSource)
        pushState(program: Self.sceneColor, preview: Self.sceneBars, mix: 0)
        "eiviz-mac".withCString { name in
            check(mixer_omt_start_send(Self.unitId, name), "OMT Program send")
        }
        booted = true
    }

    func shutdown() {
        guard booted else { return }
        mixer_destroy()
        booted = false
    }

    func cut() {
        check(mixer_unit_cut(Self.unitId, 1), "CUT")
        mix = 0
        tbarLocked = false
    }

    func auto() {
        check(mixer_unit_auto(Self.unitId, 500, 1), "AUTO")
        mix = 0
        tbarLocked = false
    }

    func preview(_ scene: UInt64) {
        var state = emptyUnitState()
        guard mixer_unit_get_state(Self.unitId, &state) == EIVIZ_OK else { return }
        state.preview_source = scene
        state.mix = mix
        check(mixer_unit_set_state(Self.unitId, &state), "Preview scene")
    }

    func setMix(_ value: Float) {
        if tbarLatching {
            return
        }
        if tbarLocked {
            if value < 1 {
                tbarLatching = true
                mix = 1
                tbarLatching = false
            }
            return
        }
        if value >= 0.999 {
            tbarLocked = true
            tbarLatching = true
            mix = 1
            tbarLatching = false
            check(mixer_unit_cut(Self.unitId, 1), "T-bar TAKE")
            return
        }
        mix = value
        var state = emptyUnitState()
        guard mixer_unit_get_state(Self.unitId, &state) == EIVIZ_OK else { return }
        state.mix = value
        check(mixer_unit_set_state(Self.unitId, &state), "T-bar")
    }

    func finishTBar() {
        guard tbarLocked else { return }
        tbarLatching = true
        mix = 0
        tbarLatching = false
        tbarLocked = false
    }

    private func defineScene(_ id: UInt64, source: UInt64) {
        var layer = emptyOverlay()
        layer.source_id = source
        layer.rect = EivizRect(x: 0, y: 0, width: 1, height: 1)
        layer.opacity = 1
        check(mixer_define_scene(id, Self.width, Self.height, 1, &layer), "Define scene")
    }

    private func pushState(program: UInt64, preview: UInt64, mix: Float) {
        var state = emptyUnitState()
        state.program_source = program
        state.preview_source = preview
        state.mix = mix
        state.transition_kind = 1
        check(mixer_unit_set_state(Self.unitId, &state), "Set Mixing Unit state")
    }

    private func check(_ code: Int32, _ action: String) {
        if code != EIVIZ_OK {
            errorText = lastError(action)
        }
    }

    private func lastError(_ action: String) -> String {
        var buffer = [UInt8](repeating: 0, count: 512)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_last_error(ptr.baseAddress, ptr.count)
        }
        if n > 0 {
            let detail = String(bytes: buffer.prefix(Int(n)), encoding: .utf8) ?? ""
            if !detail.isEmpty {
                return "\(action): \(detail)"
            }
        }
        return "\(action) failed."
    }
}

private func emptyOverlay() -> EivizOverlayDesc {
    zeroed()
}

private func emptyUnitState() -> EivizUnitState {
    zeroed()
}

private func zeroed<T>() -> T {
    let pointer = UnsafeMutableRawPointer.allocate(
        byteCount: MemoryLayout<T>.stride,
        alignment: MemoryLayout<T>.alignment
    )
    defer { pointer.deallocate() }
    pointer.initializeMemory(as: UInt8.self, repeating: 0, count: MemoryLayout<T>.stride)
    return pointer.load(as: T.self)
}

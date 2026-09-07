import EivizMixer
import Foundation

enum SceneResolver {
    static func resolve(_ session: MixerSessionData, key: String) -> SceneEntry? {
        let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if let guid = UUID(uuidString: trimmed) {
            let text = guid.uuidString
            return session.scenes.first {
                $0.guid.caseInsensitiveCompare(text) == .orderedSame
                    || $0.guid.caseInsensitiveCompare(trimmed) == .orderedSame
            }
        }
        if let number = UInt64(trimmed) {
            if let byId = session.scenes.first(where: { $0.id == number || $0.gpuId == number }) {
                return byId
            }
            if number >= 1 && number <= UInt64(session.scenes.count) {
                return session.scenes[Int(number) - 1]
            }
        }
        return session.scenes.first { $0.name.caseInsensitiveCompare(trimmed) == .orderedSame }
    }
}

enum SourceResolver {
    static func resolveIncoming(_ session: MixerSessionData, key: String) -> UInt64? {
        let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if isPreview(trimmed) { return EIVIZ_INCOMING_PREVIEW }
        if isProgram(trimmed) { return EIVIZ_INCOMING_PROGRAM }
        if let scene = SceneResolver.resolve(session, key: trimmed) {
            return scene.gpuId
        }
        if let input = InputResolver.resolve(session, key: trimmed) {
            return input.id
        }
        return nil
    }

    private static func isPreview(_ key: String) -> Bool {
        key == "0" || key.caseInsensitiveCompare("preview") == .orderedSame
            || key.caseInsensitiveCompare("prv") == .orderedSame
    }

    private static func isProgram(_ key: String) -> Bool {
        key == "-1" || key.caseInsensitiveCompare("program") == .orderedSame
            || key.caseInsensitiveCompare("pgm") == .orderedSame
    }
}

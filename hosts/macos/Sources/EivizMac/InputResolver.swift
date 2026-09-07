import Foundation

enum InputResolver {
    static func resolve(_ session: MixerSessionData, key: String) -> InputEntry? {
        let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if let guid = UUID(uuidString: trimmed) {
            let text = guid.uuidString
            return session.inputs.first {
                $0.guid.caseInsensitiveCompare(text) == .orderedSame
                    || $0.guid.caseInsensitiveCompare(trimmed) == .orderedSame
            }
        }
        if let number = UInt64(trimmed) {
            if let byId = session.inputs.first(where: { $0.id == number }) {
                return byId
            }
            if number >= 1 && number <= UInt64(session.inputs.count) {
                return session.inputs[Int(number) - 1]
            }
        }
        return session.inputs.first { $0.name.caseInsensitiveCompare(trimmed) == .orderedSame }
    }
}

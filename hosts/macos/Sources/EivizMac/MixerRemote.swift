import EivizRemote
import Foundation
import Security

enum KeychainStore {
    static func save(account: String, token: String) {
        let data = Data(token.utf8)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "eiviz.api",
            kSecAttrAccount as String: account
        ]
        SecItemDelete(query as CFDictionary)
        guard !token.isEmpty else { return }
        var add = query
        add[kSecValueData as String] = data
        SecItemAdd(add as CFDictionary, nil)
    }

    static func load(account: String) -> String {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "eiviz.api",
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess, let data = item as? Data else { return "" }
        return String(data: data, encoding: .utf8) ?? ""
    }
}

enum MixerRemote {
    static let previewMonitor: UInt64 = 0x0006_0001
    static let programMonitor: UInt64 = 0x0006_0002
    static let multiviewMonitor: UInt64 = 0x0006_0003
    static let sourceBase: UInt64 = 0x0005_0000

    static func copy(_ handle: Int32, _ fn: (UnsafeMutablePointer<UInt8>?, Int) -> Int32, cap: Int = 1 << 20) -> String {
        var size = cap
        while size <= 16 << 20 {
            var buffer = [UInt8](repeating: 0, count: size)
            let n = buffer.withUnsafeMutableBufferPointer { ptr in
                fn(ptr.baseAddress, ptr.count)
            }
            if n >= 0 {
                return n == 0 ? "" : String(bytes: buffer.prefix(Int(n)), encoding: .utf8) ?? ""
            }
            if n == -1 {
                size *= 2
                continue
            }
            return ""
        }
        return ""
    }

    static func snapshot(_ handle: Int32) -> String {
        copy(handle, { mixer_remote_copy_snapshot(handle, $0, $1) })
    }

    static func live(_ handle: Int32) -> String {
        copy(handle, { mixer_remote_copy_live(handle, $0, $1) }, cap: 1 << 16)
    }

    static func status(_ handle: Int32) -> String {
        copy(handle, { mixer_remote_copy_status(handle, $0, $1) }, cap: 4096)
    }

    static func mutate(_ handle: Int32, _ json: String, expected: UInt64) -> Int32 {
        let data = Data(json.utf8)
        return data.withUnsafeBytes { ptr in
            mixer_remote_mutate(handle, ptr.bindMemory(to: UInt8.self).baseAddress, data.count, expected)
        }
    }

    static func upsertScene(_ scene: SceneEntry) -> String {
        encode(["kind": "upsertScene", "scene": encodeValue(scene)])
    }

    static func upsertInput(_ input: InputEntry) -> String {
        let encoder = JSONEncoder()
        guard let data = try? encoder.encode(input),
              let object = try? JSONSerialization.jsonObject(with: data)
        else { return "{}" }
        return encode(["kind": "upsertInput", "input": object])
    }

    static func deleteInput(_ id: UInt64) -> String {
        encode(["kind": "deleteInput", "id": NSNumber(value: id)])
    }

    static func deleteScene(_ id: UInt64) -> String {
        encode(["kind": "deleteScene", "id": NSNumber(value: id)])
    }

    static func setOverlaySlot(unitId: UInt64, index: UInt32, slot: OverlaySlot) -> String {
        let wire: [String: Any] = [
            "sceneGpuId": NSNumber(value: slot.sceneGpuId),
            "x": slot.x,
            "y": slot.y,
            "width": slot.width,
            "height": slot.height,
            "opacity": slot.opacity,
            "z": slot.z,
            "enabled": slot.enabled,
            "transitionKind": slot.transitionKind,
            "durationValue": slot.durationValue,
            "durationUnit": slot.durationUnit,
            "audioFollow": slot.audioFollow,
            "sourceKind": slot.sourceKind == .input ? 1 : 0,
            "locked": slot.locked,
            "hidden": slot.hidden
        ]
        encode([
            "kind": "setOverlaySlot",
            "unitId": NSNumber(value: unitId),
            "index": index,
            "slot": wire
        ])
    }

    static func mix(from liveJson: String, unitId: UInt64) -> Float? {
        guard let data = liveJson.data(using: .utf8),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let units = root["units"] as? [String: Any],
              let unit = units[String(unitId)] as? [String: Any]
        else { return nil }
        if let value = unit["mix"] as? Double { return Float(value) }
        if let value = unit["mix"] as? NSNumber { return value.floatValue }
        return nil
    }

    private static func encode(_ object: [String: Any]) -> String {
        guard JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(withJSONObject: object, options: [])
        else { return "{}" }
        return String(data: data, encoding: .utf8) ?? "{}"
    }

    private static func encodeValue(_ scene: SceneEntry) -> [String: Any] {
        [
            "id": NSNumber(value: scene.id),
            "guid": scene.guid,
            "name": scene.name,
            "tags": scene.tags,
            "previewCollapsed": scene.previewCollapsed,
            "layers": scene.layers.map { layer -> [String: Any] in
                [
                    "inputId": NSNumber(value: layer.inputId),
                    "x": layer.x,
                    "y": layer.y,
                    "width": layer.width,
                    "height": layer.height,
                    "opacity": layer.opacity,
                    "z": layer.z,
                    "audioFollow": layer.audioFollow,
                    "locked": layer.locked,
                    "hidden": layer.hidden,
                    "sizeLinked": layer.sizeLinked,
                    "cropX": layer.cropX,
                    "cropY": layer.cropY,
                    "cropWidth": layer.cropWidth,
                    "cropHeight": layer.cropHeight
                ]
            }
        ]
    }
}

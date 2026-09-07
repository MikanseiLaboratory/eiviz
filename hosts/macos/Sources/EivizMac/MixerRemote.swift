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
    static let mainMultiviewMonitor: UInt64 = 0x0006_0004
    static let sourceBase: UInt64 = 0x0005_0100
    static let previewSourceId: UInt64 = 0x0005_0001
    static let programSourceId: UInt64 = 0x0005_0002
    static let mainMultiviewSourceId: UInt64 = 0x0005_0003

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

    static func discover(_ handle: Int32, kind: String, query: String) -> String {
        kind.withCString { kindPtr in
            query.withCString { queryPtr in
                copy(handle, { mixer_remote_discover(handle, kindPtr, queryPtr, $0, $1) }, cap: 8192)
            }
        }
    }

    static func upsertScene(_ scene: SceneEntry) -> String {
        encode(["kind": "upsertScene", "scene": encodeValue(scene)])
    }

    static func lines(_ payload: String) -> [String] {
        payload
            .split(whereSeparator: { $0 == "\n" || $0 == "\r" })
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    static func captures(_ payload: String) -> [VideoCaptureDevice] {
        guard let data = payload.data(using: .utf8),
              let rows = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else { return [] }
        return rows.compactMap { row in
            guard let id = row["id"] as? String, !id.isEmpty,
                  let name = row["name"] as? String, !name.isEmpty
            else { return nil }
            return VideoCaptureDevice(id: id, name: name)
        }
    }

    static func modes(_ payload: String) -> [CaptureMode] {
        guard let data = payload.data(using: .utf8),
              let rows = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else { return [] }
        return rows.compactMap { row in
            let width = uint32(row["width"])
            let height = uint32(row["height"])
            let fpsNum = uint32(row["fpsNum"])
            let fpsDen = uint32(row["fpsDen"])
            guard width > 0, height > 0 else { return nil }
            return CaptureMode(
                width: width,
                height: height,
                fpsNum: fpsNum,
                fpsDen: fpsDen,
                format: uint32(row["format"])
            )
        }
    }

    private static func uint32(_ value: Any?) -> UInt32 {
        if let number = value as? NSNumber { return number.uint32Value }
        if let number = value as? Int { return UInt32(clamping: number) }
        return 0
    }

    static func upsertUnit(_ unit: MixingUnitEntry) -> String {
        let encoder = JSONEncoder()
        guard let data = try? encoder.encode(unit),
              let object = try? JSONSerialization.jsonObject(with: data)
        else { return "{}" }
        return encode(["kind": "upsertUnit", "unit": object])
    }

    static func deleteUnit(_ id: UInt64) -> String {
        encode(["kind": "deleteUnit", "id": NSNumber(value: id)])
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

    static func upsertMultiview(_ layout: MultiviewLayout) -> String {
        let tiles: [[String: Any]] = layout.tiles.map { tile in
            [
                "kind": tile.kind.rawValue,
                "sourceId": NSNumber(value: tile.sourceId),
                "labelFollow": tile.labelFollow,
                "label": tile.label
            ]
        }
        var body: [String: Any] = [
            "id": NSNumber(value: layout.id),
            "name": layout.name,
            "previewUnitId": NSNumber(value: layout.previewUnitId),
            "programUnitId": NSNumber(value: layout.programUnitId),
            "presentInterval": layout.presentInterval,
            "tiles": tiles,
            "template": layout.template.rawValue,
            "previewLabelFollow": layout.previewLabelFollow,
            "previewLabel": layout.previewLabel,
            "programLabelFollow": layout.programLabelFollow,
            "programLabel": layout.programLabel,
            "alwaysOnTop": layout.alwaysOnTop
        ]
        if let anchor = layout.labelAnchor {
            body["labelAnchor"] = anchor.rawValue
        }
        if let size = layout.labelSize {
            body["labelSize"] = size
        }
        if let unit = layout.labelUnit {
            body["labelUnit"] = unit.rawValue
        }
        encode(["kind": "upsertMultiview", "layout": body])
    }

    static func deleteMultiview(_ id: UInt64) -> String {
        encode(["kind": "deleteMultiview", "id": NSNumber(value: id)])
    }

    static func setSettings(_ session: MixerSessionData) -> String {
        let encoder = JSONEncoder()
        guard let settings = try? encoder.encode(session.settings),
              let settingsObj = try? JSONSerialization.jsonObject(with: settings),
              let outputs = try? encoder.encode(session.outputs),
              let outputsObj = try? JSONSerialization.jsonObject(with: outputs),
              let buses = try? encoder.encode(session.buses),
              let busesObj = try? JSONSerialization.jsonObject(with: buses)
        else { return "{}" }
        return encode([
            "kind": "setSettings",
            "settings": settingsObj,
            "outputs": outputsObj,
            "buses": busesObj,
            "headphoneCopyMaster": session.headphoneCopyMaster,
            "nextOutputId": NSNumber(value: session.nextOutputId),
            "nextBusId": NSNumber(value: session.nextBusId)
        ])
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

enum RemoteEndpoint {
    static let defaultHost = "127.0.0.1"
    static let defaultPort: UInt32 = 9400

    static func split(_ text: String) -> (host: String, port: UInt32) {
        var raw = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if raw.isEmpty {
            return (defaultHost, defaultPort)
        }
        if !raw.contains("://") {
            raw = "ws://" + raw
        }
        guard let url = URL(string: raw), let host = url.host, !host.isEmpty else {
            return (defaultHost, defaultPort)
        }
        let port = url.port.map { UInt32(clamping: $0) } ?? defaultPort
        return (host, port == 0 ? defaultPort : min(port, 65535))
    }

    static func format(host: String, port: UInt32) -> String? {
        let trimmed = host.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let safePort = min(max(port, 1), 65535)
        if trimmed.contains(":") && !trimmed.hasPrefix("[") {
            return "ws://[\(trimmed)]:\(safePort)"
        }
        return "ws://\(trimmed):\(safePort)"
    }

    static func display(_ text: String) -> String {
        let parts = split(text)
        if parts.host.contains(":") && !parts.host.hasPrefix("[") {
            return "[\(parts.host)]:\(parts.port)"
        }
        return "\(parts.host):\(parts.port)"
    }
}

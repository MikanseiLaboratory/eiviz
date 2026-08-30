import SwiftUI

enum EivizTheme {
    static let background = Color(red: 26 / 255, green: 26 / 255, blue: 26 / 255)
    static let chrome = Color(red: 37 / 255, green: 37 / 255, blue: 37 / 255)
    static let panel = Color(red: 22 / 255, green: 22 / 255, blue: 22 / 255)
    static let list = Color(red: 34 / 255, green: 34 / 255, blue: 34 / 255)
    static let statusBar = Color(red: 17 / 255, green: 17 / 255, blue: 17 / 255)
    static let videoBar = Color(red: 20 / 255, green: 20 / 255, blue: 20 / 255)
    static let dialog = Color(red: 43 / 255, green: 43 / 255, blue: 43 / 255)
    static let dialogSide = Color(red: 30 / 255, green: 30 / 255, blue: 30 / 255)
    static let text = Color(red: 238 / 255, green: 238 / 255, blue: 238 / 255)
    static let dim = Color(red: 170 / 255, green: 170 / 255, blue: 170 / 255)
    static let status = Color(red: 124 / 255, green: 252 / 255, blue: 124 / 255)
    static let warn = Color(red: 1, green: 183 / 255, blue: 77 / 255)
    static let hud = Color(red: 156 / 255, green: 204 / 255, blue: 101 / 255)
    static let preview = Color(red: 232 / 255, green: 119 / 255, blue: 34 / 255)
    static let program = Color(red: 46 / 255, green: 125 / 255, blue: 50 / 255)
    static let button = Color(red: 58 / 255, green: 58 / 255, blue: 58 / 255)
    static let stroke = Color(white: 0.27)
}

struct MixerButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(EivizTheme.text)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(EivizTheme.button)
            .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}

extension View {
    func mixerField() -> some View {
        self
            .textFieldStyle(.plain)
            .padding(6)
            .background(Color(white: 0.16))
            .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
            .foregroundStyle(EivizTheme.text)
    }
}

func parseUInt32(_ text: String) -> UInt32? {
    UInt32(text.filter(\.isNumber))
}

func mixerUintField(_ value: Binding<UInt32>, minimum: UInt32 = 1) -> some View {
    TextField("", text: Binding(
        get: { String(value.wrappedValue) },
        set: { text in
            if let n = parseUInt32(text), n >= minimum {
                value.wrappedValue = n
            }
        }
    ))
    .mixerField()
}

func mixerInt32Field(_ value: Binding<Int32>) -> some View {
    TextField("", text: Binding(
        get: { String(value.wrappedValue) },
        set: { text in
            let filtered = text.filter { $0.isNumber || $0 == "-" }
            if let n = Int32(filtered) {
                value.wrappedValue = n
            }
        }
    ))
    .mixerField()
}

import Foundation

enum L10n {
    static func t(_ key: String) -> String {
        let language = AppPrefs.shared.language
        let table = language == .ja ? ja : en
        return table[key] ?? en[key] ?? key
    }

    static func error(_ action: String, _ code: Int32) -> String {
        let reason: String
        switch code {
        case 1: reason = t("error.alreadyCreated")
        case 2: reason = t("error.notCreated")
        case 3: reason = t("error.invalidArgument")
        case 4: reason = t("error.device")
        case 5: reason = t("error.io")
        default: reason = t("error.unknown").replacingOccurrences(of: "{0}", with: "\(code)")
        }
        return t("error.format")
            .replacingOccurrences(of: "{0}", with: t("action.\(action)"))
            .replacingOccurrences(of: "{1}", with: reason)
    }

    private static let en: [String: String] = [
        "chrome.mixingUnit": "Mixing Unit",
        "chrome.add": "Add",
        "chrome.edit": "Edit",
        "chrome.delete": "Delete",
        "chrome.open": "Open",
        "chrome.openRecent": "Open recent",
        "chrome.save": "Save",
        "chrome.load": "Load",
        "chrome.overlay": "Overlay",
        "chrome.multiview": "Multiview",
        "chrome.resources": "Resources",
        "chrome.settings": "Settings",
        "chrome.preferences": "Preferences",
        "chrome.newMultiview": "New Multiview",
        "prefs.language": "Language",
        "prefs.english": "English",
        "prefs.japanese": "日本語",
        "prefs.theme": "Theme",
        "prefs.themeDark": "Dark",
        "prefs.themeLight": "Light",
        "prefs.themeSystem": "Follow OS Setting",
        "about.blurb": "eiviz is an experimental software switcher developed and maintained by Mikansei Laboratory.",
        "about.author": "Shugo Kawamura",
        "about.openSource": "Open source",
        "about.license": "eiviz original source is PolyForm Shield License 1.0.0. Third-party crates stay MIT / Apache-2.0 / Zlib. NDI® is a trademark of Vizrt NDI AB.",
        "settings.alwaysOnTop": "Always on top",
        "settings.labelPosition": "Label position",
        "mv.top": "Top",
        "mv.bottom": "Bottom",
        "dialog.ok": "OK",
        "dialog.cancel": "Cancel",
        "error.format": "{0}: {1}",
        "error.alreadyCreated": "Already created",
        "error.notCreated": "Not created",
        "error.invalidArgument": "Invalid argument",
        "error.device": "Device error",
        "error.io": "I/O error",
        "error.unknown": "Error ({0})",
        "action.Metal mixer initialization": "Metal mixer initialization",
        "action.Create Mixing Unit": "Create Mixing Unit",
        "action.Configure Mixing Unit": "Configure Mixing Unit",
        "action.Set frame buffer": "Set frame buffer",
        "action.Set ReBAR optimization": "Set ReBAR optimization",
        "action.Set NDI GPU upload": "Set NDI GPU upload",
        "action.Bind Multiview": "Bind Multiview",
        "action.Load session": "Load session",
        "action.Save session": "Save session",
        "action.Still load": "Still load",
        "action.Add output": "Add output",
        "action.Set Mixing Unit state": "Set Mixing Unit state",
        "action.Preview scene": "Preview scene",
        "action.CUT": "CUT",
        "action.AUTO": "AUTO",
        "action.TAKE": "TAKE",
        "action.T-bar": "T-bar",
        "action.Overlays": "Overlays",
        "action.Audio link": "Audio link"
    ]

    private static let ja: [String: String] = [
        "chrome.mixingUnit": "Mixing Unit",
        "chrome.add": "追加",
        "chrome.edit": "編集",
        "chrome.delete": "削除",
        "chrome.open": "開く",
        "chrome.openRecent": "最近使ったファイル",
        "chrome.save": "保存",
        "chrome.load": "読み込み",
        "chrome.overlay": "Overlay",
        "chrome.multiview": "Multiview",
        "chrome.resources": "リソース",
        "chrome.settings": "設定",
        "chrome.preferences": "環境設定",
        "chrome.newMultiview": "新規 Multiview",
        "prefs.language": "言語",
        "prefs.english": "English",
        "prefs.japanese": "日本語",
        "prefs.theme": "テーマ",
        "prefs.themeDark": "ダーク",
        "prefs.themeLight": "ライト",
        "prefs.themeSystem": "OS の設定に従う",
        "about.blurb": "eiviz は Mikansei Laboratory が開発・維持している実験的なソフトウェアスイッチャーです。",
        "about.author": "川村柊吾",
        "about.openSource": "オープンソース",
        "about.license": "eiviz 本体は PolyForm Shield License 1.0.0 です。第三者クレートは MIT / Apache-2.0 / Zlib のままです。NDI® は Vizrt NDI AB の商標です。",
        "settings.alwaysOnTop": "常に手前に表示",
        "settings.labelPosition": "ラベル位置",
        "mv.top": "上",
        "mv.bottom": "下",
        "dialog.ok": "OK",
        "dialog.cancel": "キャンセル",
        "error.format": "{0}: {1}",
        "error.alreadyCreated": "すでに作成済みです",
        "error.notCreated": "まだ作成されていません",
        "error.invalidArgument": "引数が正しくありません",
        "error.device": "デバイスエラーです",
        "error.io": "入出力エラーです",
        "error.unknown": "エラー ({0})",
        "action.Metal mixer initialization": "Metal ミキサー初期化",
        "action.Create Mixing Unit": "Mixing Unit の作成",
        "action.Configure Mixing Unit": "Mixing Unit の設定",
        "action.Set frame buffer": "フレームバッファ設定",
        "action.Set ReBAR optimization": "ReBAR 最適化の設定",
        "action.Set NDI GPU upload": "NDI GPU アップロードの設定",
        "action.Bind Multiview": "Multiview のバインド",
        "action.Load session": "セッションの読み込み",
        "action.Save session": "セッションの保存",
        "action.Still load": "静止画の読み込み",
        "action.Add output": "出力の追加",
        "action.Set Mixing Unit state": "Mixing Unit 状態の設定",
        "action.Preview scene": "シーンのプレビュー",
        "action.CUT": "CUT",
        "action.AUTO": "AUTO",
        "action.TAKE": "TAKE",
        "action.T-bar": "T-bar",
        "action.Overlays": "Overlay",
        "action.Audio link": "音声リンク"
    ]
}

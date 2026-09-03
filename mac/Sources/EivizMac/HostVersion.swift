import Foundation

enum HostVersion {
    static var display: String {
        if let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
           !version.isEmpty
        {
            return version
        }
        return "0.2.0-beta.3"
    }
}

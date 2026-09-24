import Darwin
import Foundation
import Security
import WidgetKit

private func checkSigning() throws {
    guard ProcessInfo.processInfo.operatingSystemVersion.majorVersion >= 14 else {
        throw WidgetDataError.unsupportedSystem
    }
    guard let group = Bundle.main.object(forInfoDictionaryKey: "TrueNorthWidgetAppGroup") as? String else {
        throw WidgetDataError.missingAppGroup
    }
    var code: SecCode?
    var staticCode: SecStaticCode?
    var information: CFDictionary?
    guard SecCodeCopySelf([], &code) == errSecSuccess, let code,
          SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode,
          SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &information)
            == errSecSuccess,
          let info = information as? [String: Any],
          let team = info[kSecCodeInfoTeamIdentifier as String] as? String,
          group.hasPrefix(team + "."),
          let entitlements = info[kSecCodeInfoEntitlementsDict as String] as? [String: Any],
          let groups = entitlements["com.apple.security.application-groups"] as? [String],
          groups.contains(group) else {
        throw WidgetDataError.missingAppGroup
    }
}

@_cdecl("truenorth_widget_check")
public func checkWidget() -> UnsafeMutablePointer<CChar>? {
    do {
        try checkSigning()
        return nil
    } catch {
        return strdup(error.localizedDescription)
    }
}

@_cdecl("truenorth_widget_update")
public func updateWidget(_ json: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    do {
        try checkSigning()
        let store = try WidgetSnapshotStore.shared()
        if let json {
            try store.write(Data(String(cString: json).utf8))
        } else {
            try store.clear()
        }
        WidgetCenter.shared.reloadTimelines(ofKind: netWorthWidgetKind)
        return nil
    } catch {
        return strdup(error.localizedDescription)
    }
}

@_cdecl("truenorth_widget_free")
public func freeWidgetError(_ pointer: UnsafeMutablePointer<CChar>?) {
    free(pointer)
}

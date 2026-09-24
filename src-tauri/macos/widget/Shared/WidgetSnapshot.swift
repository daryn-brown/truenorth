import Foundation

let netWorthWidgetKind = "TrueNorthNetWorth"

enum WidgetDataError: LocalizedError {
    case invalidSnapshot
    case missingAppGroup
    case inaccessibleContainer
    case unsupportedSystem

    var errorDescription: String? {
        switch self {
        case .invalidSnapshot:
            return "The widget snapshot is invalid. Open TrueNorth to refresh it."
        case .missingAppGroup:
            return "This app is not signed for the TrueNorth widget app group."
        case .inaccessibleContainer:
            return "macOS denied access to the widget's shared container."
        case .unsupportedSystem:
            return "TrueNorth desktop widgets require macOS 14 or later."
        }
    }
}

struct WidgetSnapshot: Decodable {
    enum State: String, Decodable {
        case ready
        case noAccounts = "no_accounts"
        case missingBalances = "missing_balances"
        case missingRates = "missing_rates"
    }

    let schemaVersion: Int
    let updatedAt: Date
    let state: State
    let totalUSD: Double?
    let totalCAD: Double?
    let asOf: String?

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case updatedAt = "updated_at"
        case state
        case totalUSD = "total_usd"
        case totalCAD = "total_cad"
        case asOf = "as_of"
    }

    var dataDate: Date? {
        guard let asOf else { return nil }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd"
        formatter.isLenient = false
        guard let date = formatter.date(from: asOf),
              formatter.string(from: date) == asOf else { return nil }
        return date
    }

    func isStale(at date: Date) -> Bool {
        date.timeIntervalSince(updatedAt) >= 24 * 60 * 60
    }

    static func decode(_ data: Data) throws -> WidgetSnapshot {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let snapshot = try decoder.decode(WidgetSnapshot.self, from: data)
        guard snapshot.schemaVersion == 1 else { throw WidgetDataError.invalidSnapshot }
        if snapshot.state == .ready {
            guard let usd = snapshot.totalUSD, usd.isFinite,
                  let cad = snapshot.totalCAD, cad.isFinite,
                  snapshot.dataDate != nil else { throw WidgetDataError.invalidSnapshot }
        } else if snapshot.totalUSD != nil || snapshot.totalCAD != nil || snapshot.asOf != nil {
            throw WidgetDataError.invalidSnapshot
        }
        return snapshot
    }

    static var preview: WidgetSnapshot {
        WidgetSnapshot(
            schemaVersion: 1, updatedAt: Date(), state: .ready,
            totalUSD: 124_500, totalCAD: 168_075,
            asOf: ISO8601DateFormatter().string(from: Date()).prefix(10).description
        )
    }
}

struct WidgetSnapshotStore {
    let directory: URL
    var file: URL { directory.appendingPathComponent("net-worth.json") }

    static func shared() throws -> WidgetSnapshotStore {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "TrueNorthWidgetAppGroup") as? String,
              !group.isEmpty else { throw WidgetDataError.missingAppGroup }
        guard let container = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: group
        ) else { throw WidgetDataError.inaccessibleContainer }
        return WidgetSnapshotStore(directory: container.appendingPathComponent("Widget", isDirectory: true))
    }

    func read() throws -> WidgetSnapshot? {
        do {
            return try WidgetSnapshot.decode(Data(contentsOf: file))
        } catch CocoaError.fileReadNoSuchFile {
            return nil
        }
    }

    func write(_ data: Data) throws {
        _ = try WidgetSnapshot.decode(data)
        let manager = FileManager.default
        try manager.createDirectory(
            at: directory, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        try data.write(to: file, options: .atomic)
        try manager.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
    }

    func clear() throws {
        do {
            try FileManager.default.removeItem(at: file)
        } catch CocoaError.fileNoSuchFile {
            return
        }
    }
}

import Foundation
import Darwin

private struct CheckFailure: Error, CustomStringConvertible {
    let description: String
}

private func expect(_ condition: @autoclosure () throws -> Bool, _ message: String) throws {
    if try !condition() { throw CheckFailure(description: message) }
}

private func expectFailure(_ operation: () throws -> Void) throws {
    do {
        try operation()
    } catch {
        return
    }
    throw CheckFailure(description: "Expected the operation to reject invalid or inaccessible data.")
}

@main
struct WidgetSnapshotChecks {
    private static func data(_ overrides: [String: Any] = [:]) throws -> Data {
        var values: [String: Any] = [
            "schema_version": 1, "updated_at": "2026-09-24T12:00:00Z",
            "state": "ready", "total_usd": -100.0, "total_cad": -125.0,
            "as_of": "2026-09-23",
        ]
        values.merge(overrides) { _, new in new }
        return try JSONSerialization.data(withJSONObject: values)
    }

    private static func roundTrip(_ store: WidgetSnapshotStore) throws {
        try expect(store.read() == nil, "Missing snapshot must mean sharing is off.")
        try store.write(data())
        guard let snapshot = try store.read() else { throw CheckFailure(description: "Missing written snapshot.") }
        try expect(snapshot.totalUSD == -100, "Negative USD net worth must be preserved.")
        try expect(snapshot.totalCAD == -125, "Negative CAD net worth must be preserved.")
        try expect(snapshot.asOf == "2026-09-23", "Underlying data date must be preserved.")
        try expect(snapshot.state == .ready, "Valid snapshot must be ready.")
        let attributes = try FileManager.default.attributesOfItem(atPath: store.file.path)
        try expect((attributes[.posixPermissions] as? NSNumber)?.intValue == 0o600, "Snapshot must be owner-only.")
        try store.write(data(["total_usd": 200.0]))
        try expect(store.read()?.totalUSD == 200, "An atomic replacement must expose the new total.")
        try store.clear()
        try expect(store.read() == nil, "Opt-out must delete shared totals.")
        try store.clear()
    }

    private static func invalidSnapshots() throws {
        for invalid in [
            try data(["schema_version": 2]),
            try data(["total_usd": NSNull()]),
            try data(["as_of": "2026-02-30"]),
            try data(["updated_at": "not a date"]),
            try data(["state": "unknown"]),
            Data("not JSON".utf8),
        ] {
            try expectFailure { _ = try WidgetSnapshot.decode(invalid) }
        }
    }

    private static func emptyStates() throws {
        try expectFailure { _ = try WidgetSnapshot.decode(data(["state": "missing_rates"])) }
        let empty = try data([
            "state": "no_accounts", "total_usd": NSNull(),
            "total_cad": NSNull(), "as_of": NSNull(),
        ])
        try expect(WidgetSnapshot.decode(empty).state == .noAccounts, "Empty state must not become zero net worth.")
    }

    private static func freshness() throws {
        let snapshot = try WidgetSnapshot.decode(data())
        try expect(!snapshot.isStale(at: snapshot.updatedAt.addingTimeInterval(86_399)), "Snapshot should not expire early.")
        try expect(snapshot.isStale(at: snapshot.updatedAt.addingTimeInterval(86_400)), "Snapshot must expire at 24 hours.")
    }

    private static func readErrors(_ store: WidgetSnapshotStore) throws {
        try FileManager.default.createDirectory(at: store.file, withIntermediateDirectories: false)
        try expectFailure { _ = try store.read() }
    }

    static func main() {
        do {
            let directory = FileManager.default.temporaryDirectory
                .appendingPathComponent("truenorth-widget-tests-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
            let store = WidgetSnapshotStore(directory: directory)
            do {
                let checks: [(String, () throws -> Void)] = [
                    ("snapshot round trip, permissions, and opt-out", { try roundTrip(store) }),
                    ("corrupt and incompatible snapshot rejection", invalidSnapshots),
                    ("empty states cannot carry financial amounts", emptyStates),
                    ("exact 24-hour freshness boundary", freshness),
                    ("read failures are not treated as sharing disabled", { try readErrors(store) }),
                ]
                for (name, check) in checks {
                    try check()
                    print("ok - \(name)")
                }
            } catch {
                try FileManager.default.removeItem(at: directory)
                throw error
            }
            try FileManager.default.removeItem(at: directory)
        } catch {
            FileHandle.standardError.write(Data("Widget snapshot check failed: \(error)\n".utf8))
            exit(1)
        }
    }
}

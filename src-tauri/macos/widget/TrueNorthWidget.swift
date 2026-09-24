import Foundation
import OSLog
import SwiftUI
import WidgetKit

private let logger = Logger(subsystem: "com.darynbrown.finance-second-brain.widget", category: "snapshot")

struct NetWorthEntry: TimelineEntry {
    let date: Date
    let snapshot: WidgetSnapshot?
    var failed = false
    var placeholder = false
}

struct NetWorthProvider: TimelineProvider {
    func placeholder(in context: Context) -> NetWorthEntry {
        NetWorthEntry(date: Date(), snapshot: .preview, placeholder: true)
    }

    func getSnapshot(in context: Context, completion: @escaping (NetWorthEntry) -> Void) {
        completion(context.isPreview ? NetWorthEntry(date: Date(), snapshot: .preview) : entry())
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<NetWorthEntry>) -> Void) {
        let current = entry()
        completion(Timeline(entries: [current], policy: .after(current.date.addingTimeInterval(15 * 60))))
    }

    private func entry() -> NetWorthEntry {
        do {
            return NetWorthEntry(date: Date(), snapshot: try WidgetSnapshotStore.shared().read())
        } catch {
            logger.error("Cannot read widget snapshot: \(error.localizedDescription, privacy: .public)")
            return NetWorthEntry(date: Date(), snapshot: nil, failed: true)
        }
    }
}

struct NetWorthWidgetView: View {
    let entry: NetWorthEntry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 5) {
                Image(systemName: "location.north.circle.fill")
                    .foregroundStyle(.purple)
                Text("TrueNorth").fontWeight(.semibold)
                Spacer(minLength: 0)
                if family == .systemMedium {
                    Text("NET WORTH").font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            .font(.caption)

            if let snapshot = entry.snapshot, snapshot.state == .ready,
               let usd = snapshot.totalUSD, let cad = snapshot.totalCAD {
                if family == .systemMedium {
                    Spacer(minLength: 0)
                    HStack(spacing: 16) {
                        amount(usd, currency: "USD")
                        Divider()
                        amount(cad, currency: "CAD")
                    }
                    Spacer(minLength: 0)
                } else {
                    amount(usd, currency: "USD")
                    amount(cad, currency: "CAD")
                    Spacer(minLength: 0)
                }
                if snapshot.isStale(at: entry.date) {
                    Label("Open app to refresh", systemImage: "clock")
                        .font(.system(size: 10)).foregroundStyle(.secondary)
                } else if let date = snapshot.dataDate {
                    Text("Data from \(date, format: Date.FormatStyle(date: .abbreviated, time: .omitted, timeZone: .gmt))")
                        .font(.system(size: 10)).foregroundStyle(.secondary)
                        .lineLimit(1).minimumScaleFactor(0.8)
                }
            } else {
                Spacer(minLength: 0)
                Text(emptyTitle).font(.headline)
                Text(emptyMessage).font(.caption).foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        .redacted(reason: entry.placeholder ? .placeholder : [])
        .containerBackground(for: .widget) {
            LinearGradient(
                colors: [Color.purple.opacity(0.22), Color.indigo.opacity(0.08)],
                startPoint: .topLeading, endPoint: .bottomTrailing
            )
        }
    }

    private func amount(_ value: Double, currency: String) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(currency).font(.system(size: 9, weight: .medium)).foregroundStyle(.secondary)
            Text(value, format: .currency(code: currency).precision(.fractionLength(0)))
                .font(.system(size: family == .systemMedium ? 25 : 21, weight: .semibold, design: .rounded))
                .monospacedDigit()
                .lineLimit(1)
                .minimumScaleFactor(0.55)
                .privacySensitive()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var emptyTitle: String {
        if entry.failed { return "Widget unavailable" }
        switch entry.snapshot?.state {
        case .noAccounts: return "No accounts yet"
        case .missingBalances: return "Balances needed"
        case .missingRates: return "Exchange rates needed"
        default: return "Share your net worth"
        }
    }

    private var emptyMessage: String {
        if entry.failed { return "Open TrueNorth to refresh the widget." }
        switch entry.snapshot?.state {
        case .noAccounts: return "Add an account in TrueNorth."
        case .missingBalances: return "Update or sync your balances in TrueNorth."
        case .missingRates: return "Refresh FX in TrueNorth to calculate both totals."
        default: return "Open TrueNorth, then enable sharing under Widgets."
        }
    }
}

@main
struct TrueNorthWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: netWorthWidgetKind, provider: NetWorthProvider()) { entry in
            NetWorthWidgetView(entry: entry)
        }
        .configurationDisplayName("Net worth")
        .description("Your USD and CAD net worth, shared locally from TrueNorth.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

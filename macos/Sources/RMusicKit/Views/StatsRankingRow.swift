import SwiftUI

struct StatsRankingRow: View {
    let item: JSONValue
    let index: Int
    let kind: String
    let maximum: Double
    let store: MusicStore
    private var title: String { item[kind == "tracks" ? "title" : kind == "albums" ? "album" : "artist"].text }
    var body: some View {
        HStack(spacing: 14) {
            Text(String(index + 1)).font(.caption.monospacedDigit()).foregroundStyle(.secondary).frame(width: 22)
            if kind == "tracks" {
                ArtworkView(track: Track(id: item["track_id"].text, title: title, artist: item["artist"].text, album: "", duration: 0, format: "", sample_rate: 0, bit_depth: 0, cover_url: item["cover_url"].string), store: store, radius: 7).frame(width: 44, height: 44)
            } else {
                Image(systemName: kind == "albums" ? "square.stack" : "person.crop.circle").font(.title2).foregroundStyle(MusicStyle.accent).frame(width: 44, height: 44).background(MusicStyle.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
            }
            VStack(alignment: .leading, spacing: 7) {
                Group { if kind == "tracks" { Text(title.isEmpty ? "未命名" : title) } else { CatalogTextLink(track: catalogSummary(item), store: store, album: kind == "albums") } }.font(.system(size: 12, weight: .medium)).lineLimit(1)
                if kind != "artists" { CatalogTextLink(track: catalogSummary(item), store: store).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                GeometryReader { geometry in Capsule().fill(MusicStyle.accent.opacity(0.5)).frame(width: geometry.size.width * min(1, max(0, item["listened_seconds"].number / maximum))) }.frame(maxWidth: 260).frame(height: 3)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 5) { Text(listeningTime(item["listened_seconds"].number)).font(.system(size: 12, weight: .medium)).monospacedDigit(); Text("\(item["play_count"].integer) 次有效播放").font(.caption2).foregroundStyle(.secondary) }
            if kind == "tracks" {
                Button("播放 \(title)", systemImage: "play.fill") { Task { await store.perform { let track = try await store.api("/library/tracks/\(item["track_id"].text.pathComponentEncoded)").decode(Track.self); await store.play(track) } } }.labelStyle(.iconOnly).buttonStyle(MusicControlStyle()).disabled(!store.canPlay)
            }
        }.padding(.vertical, 12).contentShape(Rectangle()).contextMenu { if kind == "tracks" { TrackActions(track: catalogSummary(item), store: store) } }
    }
}

struct ListeningHistoryView: View {
    let store: MusicStore
    let timezone: String
    @StoredState private var resource = ResourceStore()
    @StoredState private var cursors = [""]
    @StoredState private var retry = UUID()
    var body: some View {
        VStack(spacing: 12) {
            ResourceNotice(resource: resource)
            if resource.error != nil { Button("重试记录") { retry = UUID() }.disabled(resource.loading) }
            if resource.value["items"].array.isEmpty && !resource.loading { Text("暂无聆听记录").foregroundStyle(.secondary).padding(20) }
            ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
                HStack(spacing: 12) {
                    Image(systemName: item["counted"].boolean ? "checkmark.circle" : "clock").foregroundStyle(MusicStyle.accent)
                    VStack(alignment: .leading, spacing: 5) {
                        Text(item["title"].text).font(.system(size: 12, weight: .medium))
                        HStack { CatalogTextLink(track: catalogSummary(item), store: store); CatalogTextLink(track: catalogSummary(item), store: store, album: true) }.font(.caption).foregroundStyle(.secondary)
                        Text("\(listeningHistoryDate(milliseconds: item["started_at_ms"].number, timezone: timezone)) · \(listeningFinishReason(item["finish_reason"].text))").font(.caption2).foregroundStyle(.secondary)
                    }
                    Spacer()
                    VStack(alignment: .trailing, spacing: 5) { Text(listeningTime(item["listened_seconds"].number)).font(.callout).monospacedDigit(); Text(item["counted"].boolean ? "有效播放" : "短时聆听").font(.caption2).foregroundStyle(.secondary) }
                    Button("播放 \(item["title"].text)", systemImage: "play.fill") { Task { await store.perform { let track = try await store.api("/library/tracks/\(item["track_id"].text.pathComponentEncoded)").decode(Track.self); await store.play(track) } } }.labelStyle(.iconOnly).buttonStyle(MusicControlStyle()).disabled(!store.canPlay)

                }.padding(.vertical, 10).contentShape(Rectangle()).contextMenu { TrackActions(track: catalogSummary(item), store: store) }
            }
            HStack { Button("上一页") { cursors.removeLast() }.disabled(cursors.count == 1 || resource.loading); Spacer(); Text("第 \(cursors.count) 页").font(.caption); Spacer(); Button("下一页") { cursors.append(resource.value["next_cursor"].text) }.disabled(resource.value["next_cursor"].text.isEmpty || resource.loading) }
        }.padding(.top, 16).task(id: "\(store.sessionID):\(cursors.count):\(retry)") { await resource.load(statsHistoryPath(cursor: cursors.last), store: store) }
        .onChange(of: store.sessionID) { cursors = [""] }
    }
}

func statsHistoryPath(cursor: String?) -> String {
    var query = ["limit": "20"]
    if let cursor, !cursor.isEmpty { query["cursor"] = cursor }
    return apiQuery("/stats/history", query)
}

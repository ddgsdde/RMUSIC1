import SwiftUI

/// Selection is scoped to the displayed page; only explicit selection exposes batch download.
struct SelectableTrackList: View {
    let tracks: [Track]
    let store: MusicStore
    var startIndex = 0
    var playlist: Playlist?
    var showCatalogDetails = false
    var serverSorted = false
    var recommendationScores: [String: JSONValue] = [:]
    @StoredState private var order = "default"
    private var displayedTracks: [Track] { sortedTracks(tracks, by: order) }
    @StoredState private var selecting = false
    @StoredState private var selected: Set<String> = []
    @StoredState private var sorting = false
    @StoredState private var downloading = false
    private var available: Set<String> { Set(tracks.filter { $0.available != false }.map(\.id)) }
    var body: some View {
        VStack(spacing: 12) {
            HStack(spacing: 12) {
                if selecting {
                    Text("已选 \(selected.count) 首").font(.caption).foregroundStyle(.secondary)
                    Button("全选本页") { selected = available }
                    Button(selected == available ? "全部下载（\(selected.count) 首）" : "下载所选（\(selected.count) 首）", systemImage: "arrow.down.to.line") {
                        let ids = tracks.filter { selected.contains($0.id) }.map(\.id)
                        downloading = true
                        Task { await store.perform { try await store.downloads.queue(ids, store: store) }; downloading = false }
                    }.disabled(selected.isEmpty || downloading || !store.connected)
                    Spacer()
                    Button("完成") { selecting = false; selected = [] }
                } else {
                    if !serverSorted { CatalogSortPicker(title: "本页排序", value: $order, options: trackSortOptions) }
                    Spacer()
                    if !recommendationScores.isEmpty { Text("偏好分").font(.caption).foregroundStyle(.secondary) }
                    Button("多选", systemImage: "checklist") { selecting = true }.disabled(tracks.isEmpty)
                }
            }.controlSize(.small)
            LazyVStack(spacing: 2) {
                ForEach(Array(displayedTracks.enumerated()), id: \.offset) { index, track in
                    HStack(spacing: 6) {
                        if let playlist, playlist.smart_query == nil, reorderable(playlist) {
                            TrackOrderHandle(scope: "playlist:\(store.sessionID):\(playlist.id)", ids: playlist.track_ids, index: startIndex + index).disabled(sorting)
                        }
                        if selecting {
                            Toggle("选择 \(track.title)", isOn: Binding(get: { selected.contains(track.id) }, set: { wanted in
                                if wanted { selected.insert(track.id) } else { selected.remove(track.id) }
                            })).labelsHidden().toggleStyle(.checkbox).disabled(track.available == false)
                        }
                        MusicTrackRow(track: track, index: startIndex + index, store: store, play: { Task { await store.playList(displayedTracks, starting: track.id) } }, playlist: playlist, showCatalogDetails: showCatalogDetails)
                        if !recommendationScores.isEmpty {
                            let score = recommendationScores[track.id].map { $0.number.formatted(.number.precision(.fractionLength(0...1))) }
                            Text(score ?? "–").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                                .frame(width: 52, alignment: .trailing).padding(.trailing, 8)
                                .accessibilityLabel(score.map { "偏好分 \($0)" } ?? "暂无偏好分")
                        }
                    }.modifier(TrackOrderDrop(scope: "playlist:\(store.sessionID):\(playlist?.id ?? "")", ids: playlist?.track_ids ?? [], index: startIndex + index, enabled: !sorting && playlist.map(reorderable) == true) { from, to in
                        if let playlist { move(playlist, from: from, to: to) }
                    })
                }
            }
        }.onChange(of: tracks.map(\.id)) { selected = selected.intersection(available) }
            .onChange(of: store.sessionID) { selected = []; selecting = false }
    }
    private func reorderable(_ playlist: Playlist) -> Bool {
        playlist.id != store.collection.favoriteID && order == "default" && (!serverSorted || store.collection.sort == "default") && playlist.smart_query == nil && Array(playlist.track_ids.dropFirst(startIndex).prefix(tracks.count)) == tracks.map { track in playlist.track_ids.contains(track.id) ? track.id : (track.provider_data ?? .null)["cached_from"]["track_id"].string ?? track.id }
    }
    private func move(_ playlist: Playlist, from: Int, to: Int) {
        sorting = true
        Task {
            await store.perform {
                let epoch = store.sessionID
                let reordered = movedTrackIDs(playlist.track_ids, from: from, to: to)
                _ = try await store.api("/playlists/\(playlist.id.pathComponentEncoded)", method: "PUT", body: .object([
                    "track_ids": .array(reordered.map(JSONValue.string)),
                    "expected": .object(["name": .string(playlist.name), "track_ids": .array(playlist.track_ids.map(JSONValue.string))]),
                ]))
                guard epoch == store.sessionID else { return }
                if store.collection.selectedID == playlist.id && store.collection.page * 100 == startIndex {
                    let visible = Dictionary(tracks.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
                    let next = reordered.dropFirst(startIndex).prefix(tracks.count).compactMap { visible[$0] }
                    if next.count == tracks.count { withAnimation(.easeInOut(duration: 0.18)) { store.collection.tracks = next } }
                }
                try await store.collection.refresh(store)
            }
            sorting = false
        }
    }
}

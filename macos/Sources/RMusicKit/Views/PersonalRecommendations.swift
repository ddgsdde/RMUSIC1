import SwiftUI

@MainActor @Observable
final class SimilarSongsStore {
    struct Notice {
        let id = UUID()
        let seed: Track
        let automatic: Bool
        var tracks: [Track] = []
        var status = "正在查找…"
        var expanded: Bool
    }
    private(set) var notice: Notice?
    @ObservationIgnored private var task: Task<Void, Never>?
    private var request: Notice?

    func find(_ track: Track, store: MusicStore, automatic: Bool = false) {
        if automatic, request?.automatic == false { return }
        close()
        let owner = store.sessionID
        let pending = Notice(seed: track, automatic: automatic, expanded: !automatic)
        request = pending
        if !automatic { notice = pending }
        task = Task {
            var next = pending
            do {
                let response = try await store.api("/recommendations/similar", method: "POST", body: .object(["track_id": .string(track.id)]))
                next.tracks = try response["items"].decode([Track].self)
                next.status = next.tracks.isEmpty ? "暂无匹配曲目" : ""
            } catch { next.status = "相似歌曲暂时无法读取" }
            guard !Task.isCancelled, owner == store.sessionID, request?.id == pending.id else { return }
            if automatic && next.tracks.isEmpty { close() } else { notice = next }
        }
    }
    func unliked(_ track: Track) {
        if request?.automatic == true, request?.seed.id == track.id { close() }
    }
    func expand() { notice?.expanded = true }
    func close() { task?.cancel(); task = nil; request = nil; notice = nil }
}

struct SimilarSongsNotice: View {
    let store: MusicStore
    var body: some View {
        if let notice = store.similarSongs.notice {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text("《\(notice.seed.title)》的相似歌曲").font(.headline).lineLimit(1)
                    Spacer()
                    Button("关闭相似歌曲", systemImage: "xmark") { store.similarSongs.close() }.labelStyle(.iconOnly)
                }
                if !notice.status.isEmpty { Text(notice.status).font(.caption).foregroundStyle(.secondary) }
                ForEach(Array(notice.tracks.prefix(notice.expanded ? 8 : 1))) { track in
                    Button {
                        store.similarSongs.close()
                        Task { await store.playList(notice.tracks, starting: track.id) }
                    } label: {
                        HStack { Text(track.title); Spacer(); Text(track.artist).foregroundStyle(.secondary) }.lineLimit(1)
                    }.disabled(!store.canPlay)
                }
                if !notice.expanded && notice.tracks.count > 1 { Button("查看全部 \(notice.tracks.count) 首") { store.similarSongs.expand() } }
            }.buttonStyle(.plain).padding(14).frame(width: 320).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
                .task(id: notice.id) {
                    guard notice.automatic else { return }
                    do { try await Task.sleep(for: .seconds(12)) } catch { return }
                    if store.similarSongs.notice?.id == notice.id, store.similarSongs.notice?.expanded == false { store.similarSongs.close() }
                }
        }
    }
}

struct DailyPlaylistSnapshot {
    let date: String
    let tracks: [Track]
    let scores: [String: JSONValue]
}

struct DailyPlaylistSection: View {
    let store: MusicStore
    var coverSize: CGFloat = 142
    let open: (DailyPlaylistSnapshot) -> Void
    @StoredState private var daily = ResourceStore()
    @Environment(\.scenePhase) private var phase

    var body: some View {
        let tracks = (try? daily.value["items"].decode([Track].self)) ?? []
        VStack(alignment: .leading, spacing: 15) {
            HStack {
                Text("每日歌单").font(.system(size: 16, weight: .semibold))
                Spacer()
                if !tracks.isEmpty {
                    Button("查看全部 \(tracks.count) 首", systemImage: "arrow.up.right") {
                        open(DailyPlaylistSnapshot(date: daily.value["date"].text, tracks: tracks, scores: daily.value["scores"].object))
                    }
                        .font(.system(size: 11)).foregroundStyle(.secondary).buttonStyle(MusicControlStyle())
                } else if daily.error != nil {
                    Button("重试") { Task { await load() } }.buttonStyle(MusicControlStyle())
                }
            }
            if tracks.isEmpty {
                Text(daily.error != nil ? "暂时无法读取" : daily.value == .null ? "正在选曲…" : "暂无可用曲目")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                ScrollView(.horizontal) {
                    LazyHStack(alignment: .top, spacing: 16) {
                        ForEach(tracks) { track in
                            VStack(alignment: .leading, spacing: 8) {
                                HomeShelfArtwork(track: track, title: track.title, store: store) {
                                    Task { await store.playList(tracks, starting: track.id) }
                                }.disabled(!store.canPlay).accessibilityLabel("播放 \(track.title)")
                                CatalogTextLink(track: track, store: store)
                                    .font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1)
                            }.frame(width: coverSize, alignment: .leading)
                                .contextMenu { TrackActions(track: track, store: store) }
                        }
                    }.padding(.vertical, 5)
                }.scrollIndicators(.visible)
            }
        }
        .task(id: "\(store.sessionID):\(phase == .active)") {
            guard phase == .active else { return }
            while !Task.isCancelled {
                await load()
                do { try await Task.sleep(for: .seconds(1800)) } catch { return }
            }
        }
    }
    private func load() async { await daily.load("/recommendations/daily", store: store, reportConnectionFailure: false) }
}

struct DailyPlaylistPage: View {
    let store: MusicStore
    let snapshot: DailyPlaylistSnapshot
    let backLabel: String
    let back: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                Button(backLabel, systemImage: "chevron.left", action: back)
                    .buttonStyle(MusicControlStyle())
                    .keyboardShortcut(.cancelAction)
                HStack(spacing: 22) {
                    ArtworkView(track: snapshot.tracks.first(where: { $0.cover_url?.isEmpty == false }) ?? snapshot.tracks.first,
                                store: store, radius: 20).frame(width: 130, height: 130)
                    VStack(alignment: .leading, spacing: 12) {
                        Text("每日歌单").font(.system(size: 30, weight: .bold)).accessibilityAddTraits(.isHeader)
                        Text(([snapshot.date].filter { !$0.isEmpty } + ["\(snapshot.tracks.count) 首歌曲"]).joined(separator: " · "))
                            .font(.callout).foregroundStyle(.secondary)
                        Button("播放全部", systemImage: "play.fill") {
                            Task { await store.playList(snapshot.tracks) }
                        }.buttonStyle(WorkspacePrimaryButton()).disabled(!store.canPlay || snapshot.tracks.isEmpty)
                    }
                    Spacer()
                }
                SelectableTrackList(tracks: snapshot.tracks, store: store, serverSorted: true, recommendationScores: snapshot.scores)
            }.padding(30)
        }
    }
}

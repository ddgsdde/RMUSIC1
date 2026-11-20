import SwiftUI

/// Album-led home. All artwork and counters come from the current Core account.
struct MineradioHomeView: View {
    let store: MusicStore
    let navigate: (String) -> Void
    let openPlayer: () -> Void
    let openDaily: (DailyPlaylistSnapshot) -> Void
    @StoredState private var home = ResourceStore()
    @StoredState private var stats = ResourceStore()
    private var tracks: [Track] { Array(((try? home.value["tracks"].decode([Track].self)) ?? []).prefix(12)) }
    private var albums: [Track] { home.value["albums"].array.prefix(12).map { item in var track = catalogSummary(item); track.cover_url = item["cover_url"].string; return track } }
    private var featured: Track? { store.zone?.track ?? tracks.first ?? albums.first }
    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 22) {
                    heading
                    HStack(spacing: 18) {
                        MineradioHomeHero(track: featured, covers: albums, store: store, action: resume, ready: store.canPlay, current: store.zone?.track != nil).frame(maxWidth: .infinity)
                        if geometry.size.width >= 850 {
                        VStack(spacing: 14) {
                            MineradioHomeShortcut(title: "发现音乐", caption: "DAILY MIX", symbol: "sparkles", artwork: albums.last, store: store) { navigate("discover") }
                            MineradioHomeShortcut(title: "播放列表", caption: "\(store.collection.playlists.count) 个歌单", symbol: "music.note.list", artwork: albums.dropFirst().first, store: store) { navigate("playlists") }
                        }.frame(width: max(220, min(280, geometry.size.width * 0.25)))
                        }
                    }.frame(height: geometry.size.width < 850 ? 210 : 230)
                    DailyPlaylistSection(store: store, coverSize: coverSize(geometry.size.width), open: openDaily)
                    if !albums.isEmpty { albumShelf(width: geometry.size.width) }
                    if !tracks.isEmpty { frequent }
                    if let error = home.error { HStack { Text(error).font(.caption).foregroundStyle(.secondary); Button("重试") { Task { await load() } } } }
                }.padding(.bottom, 20)
            }.scrollIndicators(.hidden).rememberPageScroll(store, key: "mineradio-home")
        }.task(id: store.sessionID) { await home.load("/library/home", store: store) }
            .task(id: store.listeningStatsRefreshID) { await stats.followListeningStats(apiQuery("/stats", ["period_days": "1"]), store: store) }
    }
    private var heading: some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 5) {
                Text("聆听").font(.system(size: 24, weight: .semibold))
                Text(Date.now.formatted(.dateTime.month(.wide).day().weekday(.wide))).font(.system(size: 11)).foregroundStyle(.secondary)
            }
            Spacer()
            Button { navigate("stats") } label: {
                HStack(spacing: 10) { Image(systemName: "chart.bar.xaxis").foregroundStyle(MusicStyle.accent); Text(stats.error == nil && stats.value["summary"]["listened_seconds"].number > 0 ? "今日 \(Int(stats.value["summary"]["listened_seconds"].number / 60)) 分钟" : "聆听统计").font(.system(size: 12)); Image(systemName: "chevron.right").font(.system(size: 9)) }.padding(.horizontal, 14).padding(.vertical, 10).background(.white.opacity(0.045), in: Capsule())
            }.buttonStyle(.plain)
        }
    }
    private func columns(_ width: CGFloat) -> Int { max(3, min(6, Int(width / 155))) }
    private func coverSize(_ width: CGFloat) -> CGFloat {
        let count = columns(width)
        return max(1, (width - CGFloat(count - 1) * 16) / CGFloat(count))
    }
    private func albumShelf(width: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 15) {
            HStack { Text("最近专辑").font(.system(size: 16, weight: .semibold)); Spacer(); Button { navigate("browse") } label: { HStack(spacing: 6) { Text("浏览音乐库"); Image(systemName: "arrow.up.right") } }.font(.system(size: 11)).foregroundStyle(.secondary).buttonStyle(MusicControlStyle()) }
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 16, alignment: .top), count: columns(width)), alignment: .leading, spacing: 18) {
                ForEach(albums) { album in
                    VStack(alignment: .leading, spacing: 8) {
                        HomeShelfArtwork(track: album, title: album.title, store: store) {
                            Task { await store.openAlbum(album) }
                        }.help("打开专辑 \(album.album)")
                        CatalogTextLink(track: album, store: store).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }.padding(.vertical, 5)
        }
    }
    private var frequent: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack { Text("常听曲目").font(.system(size: 16, weight: .semibold)); Spacer(); Button("音乐空间") { navigate("library") }.font(.system(size: 11)).foregroundStyle(.secondary).buttonStyle(MusicControlStyle()) }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 260), spacing: 14)], spacing: 14) {
                ForEach(tracks) { track in
                    HStack(spacing: 14) {
                        Button { playHome(track) } label: { ArtworkView(track: track, store: store, radius: 8).frame(width: 46, height: 46) }.buttonStyle(.plain).accessibilityLabel("播放 \(track.title)")
                        VStack(alignment: .leading, spacing: 5) {
                            Button(track.title) { playHome(track) }.font(.system(size: 12, weight: .medium)).lineLimit(1).buttonStyle(MusicControlStyle())
                            CatalogTextLink(track: track, store: store).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1)
                        }
                        Spacer()
                        Button { playHome(track) } label: { Image(systemName: "play.fill").font(.system(size: 11)).foregroundStyle(MusicStyle.accent) }.buttonStyle(MusicControlStyle()).accessibilityLabel("播放 \(track.title)")
                    }.padding(14).background(.white.opacity(0.035), in: RoundedRectangle(cornerRadius: 14))

                }
            }
        }
    }
    private func playHome(_ track: Track) {
        if store.canPlay { Task { await store.playList(tracks, starting: track.id) } } else { navigate("outputs") }
    }
    private func resume() {
        guard store.canPlay else { navigate("outputs"); return }
        if store.zone?.track != nil || !(store.zone?.queue.isEmpty ?? true) { Task { await store.transport("play") }; openPlayer() }
        else if let track = tracks.first { Task { await store.playList(tracks, starting: track.id) }; openPlayer() }
        else { navigate("browse") }
    }
    private func load() async {
        async let library = home.load("/library/home", store: store)
        async let listening = stats.load(apiQuery("/stats", ["period_days": "1"]), store: store)
        _ = await (library, listening)
    }
}

struct HomeShelfArtwork: View {
    let track: Track
    let title: String
    let store: MusicStore
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 8) {
                ArtworkView(track: track, store: store, radius: 9).aspectRatio(1, contentMode: .fit)
                Text(title).font(.system(size: 12, weight: .medium)).lineLimit(1)
            }.frame(maxWidth: .infinity, alignment: .leading)
        }.buttonStyle(MusicButtonStyle())
    }
}

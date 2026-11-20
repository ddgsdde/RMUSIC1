import SwiftUI

struct LibraryView: View {
    @Bindable var store: MusicStore
    let openDaily: (DailyPlaylistSnapshot) -> Void
    @SceneStorage("rmusic.library-space.section") private var section = "albums"
    var body: some View {
        if let id = store.artistID ?? store.albumID {
            CatalogDetailView(store: store, id: id, kind: store.artistID != nil ? "artist" : "album").id(id)
        } else { library }
    }
    private var library: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 30) {
                if store.albumID != nil || store.artistID != nil {
                    Button("全部音乐", systemImage: "chevron.left") { Task { await store.closeAlbum() } }.buttonStyle(MusicControlStyle()).font(.system(size: 11)).foregroundStyle(.secondary)
                    if let anchor = store.catalogAnchor ?? store.tracks.first { RelatedCatalogLinks(track: anchor, store: store) }
                    Text(store.artistName ?? store.albumName ?? "专辑").font(.system(size: 38, weight: .bold)).tracking(-1)
                } else if store.query.isEmpty { EmptyView() }
                else { Text("“\(store.query)” 的搜索结果").font(.headline) }
                if !store.connected {
                    Button("连接音乐库", systemImage: "arrow.right") { store.showConnection = true }.buttonStyle(WorkspacePrimaryButton())
                    Text("尚未连接 Core").font(.system(size: 12)).foregroundStyle(.secondary).padding(.vertical, 40)
                } else {
                    HStack(alignment: .firstTextBaseline, spacing: 24) {
                        Text(store.artistID != nil ? "歌手曲目" : store.albumID == nil ? "音乐空间" : "专辑曲目").font(.system(size: 19, weight: .semibold))
                        if store.albumID == nil && store.artistID == nil { segment("唱片", value: "albums"); segment("艺术家", value: "artists"); segment("歌曲", value: "tracks") }
                        Spacer()
                        Text("\(store.total) 首").font(.system(size: 10)).foregroundStyle(.tertiary)
                    }
                    if section == "albums" && store.query.isEmpty {
                        if AppearanceStore.shared.skin != .mineradio { DailyPlaylistSection(store: store, open: openDaily) }
                        if !store.tracks.isEmpty { albumShelf }
                    }
                    if section == "artists" { MusicSpaceArtistsView(store: store) }
                    else if store.tracks.isEmpty && !store.loading { Text("暂无歌曲").font(.system(size: 14)).foregroundStyle(.secondary).padding(.vertical, 60) }
                    else {
                        HStack { Text("歌曲").frame(maxWidth: .infinity, alignment: .leading); Text("专辑").frame(width: 150, alignment: .leading); Text("格式").frame(width: 50); Text("时长").frame(width: 88, alignment: .trailing) }.font(.system(size: 9)).foregroundStyle(.tertiary).padding(.horizontal, 30)
                        SelectableTrackList(tracks: store.tracks, store: store, startIndex: store.page * 100)
                        PageControls(page: store.page, total: store.total, loading: store.loading) { page in store.page = page; Task { await store.perform { try await store.loadLibrary() } } }
                    }
                }
            }.padding(.horizontal, 34).padding(.top, 10).padding(.bottom, 30)
        }.scrollIndicators(.hidden).rememberPageScroll(store, key: "library:\(section):\(store.page):\(store.query)")
    }
    private func segment(_ label: String, value: String) -> some View {
        Button { section = value } label: { VStack(spacing: 7) { Text(label).font(.system(size: 11, weight: section == value ? .semibold : .regular)).foregroundStyle(section == value ? Color.primary : Color.secondary); Capsule().fill(section == value ? MusicStyle.accent : .clear).frame(width: 14, height: 3) }.padding(.horizontal, 7).padding(.vertical, 4).frame(minHeight: 32).contentShape(Rectangle()) }.buttonStyle(.plain)
    }
    private var albumShelf: some View {
        ScrollView(.horizontal) { HStack(alignment: .top, spacing: 22) { ForEach(albumTracks) { track in AlbumCard(track: track, store: store) } }.padding(.bottom, 6) }.scrollIndicators(.hidden)
    }
    private var albumTracks: [Track] {
        var seen = Set<String>()
        return store.tracks.filter { seen.insert("\($0.source ?? ""):\($0.album_artist ?? $0.artist):\($0.album)").inserted }.prefix(12).map { $0 }
    }
}
private struct AlbumCard: View {
    let track: Track
    let store: MusicStore
    @StoredState private var hovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var coverSize: CGFloat { AppearanceStore.shared.skin == .mineradio ? 182 : 142 }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Button { Task { await store.openAlbum(track) } } label: {
                VStack(alignment: .leading, spacing: 10) {
                ArtworkView(track: track, store: store, radius: AppearanceStore.shared.skin == .mineradio ? 3 : 10).frame(width: coverSize, height: coverSize)
                    .rotation3DEffect(.degrees(AppearanceStore.shared.skin == .mineradio && !reduceMotion ? (hovering ? -4 : -16) : 0), axis: (x: 0, y: 1, z: 0), perspective: 0.5)
                    .overlay(alignment: .bottomTrailing) { Image(systemName: "arrow.up.right").font(.system(size: 13)).foregroundStyle(MusicStyle.onAccent).padding(11).background(MusicStyle.accent, in: Circle()).padding(9).opacity(hovering ? 1 : 0) }
                    .shadow(color: .black.opacity(hovering ? 0.13 : 0.05), radius: hovering ? 12 : 5, y: 5)
                Text(track.album.isEmpty ? track.title : track.album).font(.system(size: 12, weight: .semibold)).lineLimit(1)
                }
            }.buttonStyle(MusicButtonStyle()).help("打开专辑 \(track.album)")
            CatalogTextLink(track: track, store: store).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1)
        }.frame(width: coverSize).offset(y: hovering && !reduceMotion ? -4 : 0).onHover { hovering = $0 }.contextMenu { TrackActions(track: track, store: store) }.animation(reduceMotion ? nil : .spring(response: 0.35, dampingFraction: 0.8), value: hovering)
    }
}

import SwiftUI

struct CollectionView: View {
    @Bindable var store: MusicStore
    @StoredState private var binding = false
    @StoredState private var editing = false
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 26) {
                HStack(spacing: 22) {
                    ZStack {
                        RoundedRectangle(cornerRadius: 20).fill(LinearGradient(colors: [.mint.opacity(0.3), .teal.opacity(0.07)], startPoint: .topLeading, endPoint: .bottomTrailing))
                        Image(systemName: store.collection.selectedID == store.collection.favoriteID ? "heart.fill" : "music.note.list").font(.system(size: 45, weight: .light)).foregroundStyle(MusicStyle.accent)
                    }.frame(width: 130, height: 130)
                    VStack(alignment: .leading, spacing: 12) {
                        Text(store.collection.name.isEmpty ? "歌单" : store.collection.name).font(.system(size: 30, weight: .bold)).lineLimit(2)
                        Text("\(store.collection.total) 首 · 同步至你的 Core").font(.callout).foregroundStyle(.secondary)
                    }
                    Spacer()
                    if store.collection.selectedID == store.collection.favoriteID { Button("云端绑定") { binding = true } }
                    if let id = store.collection.selectedID { PlaylistExportButton(store: store, id: id, name: store.collection.name) }
                    if store.collection.selectedID != store.collection.favoriteID,
                       store.collection.playlists.first(where: { $0.id == store.collection.selectedID })?.smart_query == nil {
                        Button("编辑歌单", systemImage: "pencil") { editing = true }
                    }
                }
                CatalogSortPicker(title: "排序整个歌单", value: Binding(get: { store.collection.sort }, set: { value in store.collection.sort = value; if let id = store.collection.selectedID { Task { await store.perform { try await store.collection.open(id, store: store, page: 0) } } } }), options: trackSortOptions, defaultLabel: store.collection.selectedID == store.collection.favoriteID ? "最近加入我喜欢" : nil)
                if let id = store.collection.selectedID { PlaylistSyncStatusView(store: store, id: id) }
                if store.collection.loading && store.collection.tracks.isEmpty { ProgressView().frame(maxWidth: .infinity).padding(40) }
                else if store.collection.tracks.isEmpty { ContentUnavailableView("歌单为空", systemImage: "heart", description: Text("在歌曲旁点亮爱心，或通过右键菜单添加到歌单。")) }
                SelectableTrackList(tracks: store.collection.tracks, store: store, startIndex: store.collection.page * 100, playlist: store.collection.selectedPlaylist, serverSorted: true)
                PageControls(page: store.collection.page, total: store.collection.total, loading: store.collection.loading) { page in
                    if let id = store.collection.selectedID { Task { await store.perform { try await store.collection.open(id, store: store, page: page) } } }
                }
            }.padding(30)
        }.rememberPageScroll(store, key: "collection:\(store.collection.selectedID ?? ""):\(store.collection.page)")
        .scrollIndicators(.hidden).sheet(isPresented: $binding) { FavoritesBindingView(store: store, close: { binding = false }) }
        .sheet(isPresented: $editing) { if let id = store.collection.selectedID { PlaylistEditorView(store: store, id: id, close: { editing = false }) } }
    }
}

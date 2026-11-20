import SwiftUI

/// Shared by rows, statistics and playlists. Destructive actions require explicit playlist context.
struct TrackActions: View {
    let track: Track
    let store: MusicStore
    var playlist: Playlist?
    @StoredState private var resolved: Track?
    @StoredState private var artists: [CatalogReference] = []
    private var song: Track { resolved ?? track }
    var body: some View {
        Group {
            Button("播放", systemImage: "play.fill") { Task { await store.play(song) } }.disabled(!store.canPlay || song.available == false)
            if store.zone?.track?.id == song.id {
                Button("找相似", systemImage: "music.note.magnifyingglass") { store.similarSongs.find(song, store: store) }
            }
            Button("启动电台", systemImage: "dot.radiowaves.left.and.right") { Task { await store.startRadio(song) } }.disabled(!store.canPlay || song.available == false)
            Button("添加到队列", systemImage: "text.badge.plus") { Task { await store.enqueue(song) } }.disabled(!store.canPlay || song.available == false)
            Button("下一首播放", systemImage: "text.line.first.and.arrowtriangle.forward") { Task { await store.playNext(song) } }.disabled(!store.canPlay || song.available == false)
            Divider()
            Button("转到专辑", systemImage: "opticaldisc") { Task { await store.openAlbum(song) } }.disabled(!song.hasAlbum)
            Menu("转到歌手", systemImage: "person.crop.square") {
                if !artists.isEmpty {
                    ForEach(artists) { artist in Button(artist.name) { Task { await store.openCatalogReference(artist, track: song, album: false) } } }
                } else {
                    ForEach(Array(song.creditNames.enumerated()), id: \.offset) { index, name in
                        Button(name) { Task { await store.perform { let refs = try await store.catalogReferences(song, album: false, artistIndex: index); guard refs.count == 1 else { throw RuntimeFailure(message: "歌手资料仍在读取，请稍后重新打开菜单。") }; await store.openCatalogReference(refs[0], track: song, album: false) } } }
                    }
                }
            }
            Button("歌曲资料与评论", systemImage: "info.circle") { store.detailTrack = song }
            Divider()
            Button(store.collection.favorites.contains(song.id) ? "取消喜欢" : "喜欢", systemImage: "heart") { Task { await store.perform { try await store.collection.like(song, store: store) } } }
            Menu("添加到歌单") {
                ForEach(store.collection.playlists.filter { $0.smart_query == nil }) { list in Button(list.name) { Task { await store.perform { try await store.collection.change(song, playlist: list, store: store) } } } }
                if store.collection.playlists.filter({ $0.smart_query == nil }).isEmpty { Text("暂无可编辑歌单") }
            }
            if let playlist, playlist.smart_query == nil {
                Button("从此歌单移除", role: .destructive) { Task { await store.perform { try await store.collection.change(song, playlist: playlist, remove: true, store: store) } } }
            }
            Divider()
            Button("下载到指定目录", systemImage: "folder.badge.plus") { Task { await store.perform { try await store.downloads.queue([song.id], store: store) } } }.disabled(song.available == false)
            Button(store.downloads.state(song.id), systemImage: "arrow.down.circle") { Task { await store.perform { try await store.downloads.start(song.id, store: store) } } }.disabled(song.available == false)
        }.task(id: "\(store.sessionID):\(track.id)") {
            if track.source == nil || track.album.isEmpty { if let full = try? await store.api("/library/tracks/\(track.id.pathComponentEncoded)").decode(Track.self), !Task.isCancelled { resolved = full } }
            if let values = try? await store.catalogArtistReferences(song), !Task.isCancelled { artists = values }

        }
    }
}

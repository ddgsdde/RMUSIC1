import SwiftUI

/// Local copies and all account-bound cloud directory pages share one workspace.
struct PlaylistsView: View {
    let store: MusicStore
    let create: () -> Void
    let rename: (Playlist) -> Void
    let delete: (Playlist) -> Void
    @StoredState private var catalog = PlaylistCatalog()
    @StoredState private var source = "all"
    @StoredState private var selected: PlaylistEntry?
    @AppStorage("playlistLayout") private var grid = true
    @StoredState private var busy = false
    @StoredState private var editingSmart = false
    @StoredState private var importing = false
    @StoredState private var smartPlaylist: Playlist?
    @StoredState private var refresh = UUID()
    var body: some View {
        Group {
            if let selected {
                if selected.cloud { ProviderLibraryView(store: store, providerID: selected.provider, playlist: selected.value, close: { self.selected = nil }) }
                else {
                    VStack(alignment: .leading, spacing: 0) {
                        HStack { Button("全部播放列表", systemImage: "chevron.left") { self.selected = nil }; Spacer(); Button("播放全部", systemImage: "play.fill") { Task { await store.playPlaylist(selected.value["id"].text) } }.disabled(!store.canPlay || busy) }.padding(.horizontal, 30).padding(.top, 20)
                        CollectionView(store: store)
                    }
                }
            } else { catalogView }
        }.sheet(isPresented: $importing) { PlaylistImportView(store: store, close: { importing = false }, saved: { refresh = UUID() }) }
        .sheet(isPresented: $editingSmart) {
            SmartPlaylistEditor(store: store, playlist: smartPlaylist, close: { editingSmart = false }, saved: { refresh = UUID() })
        }.task(id: "\(store.sessionID):\(refresh)") {
            try? await store.collection.refresh(store)
            repeat {
                await catalog.load(store)
                do { try await Task.sleep(for: .seconds(5)) } catch { return }
            } while !Task.isCancelled && store.connected
        }
    }
    private var catalogView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 14) { sourcePicker; Spacer(); catalogActions }
                    VStack(alignment: .leading, spacing: 14) { sourcePicker; HStack { catalogActions; Spacer(minLength: 0) } }
                }
                if let error = catalog.error { Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.orange).font(.caption) }
                let entries = catalog.entries(source: source)
                Text("\(entries.count) 个播放列表").font(.caption).foregroundStyle(.secondary)
                if entries.isEmpty && !catalog.loading { ContentUnavailableView("暂无播放列表", systemImage: "music.note.list", description: Text("可创建本地歌单，或在 Core 登录音源后刷新云端目录。")) }
                if grid {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 200, maximum: 270), spacing: 20)], alignment: .leading, spacing: 20) {
                        ForEach(entries) { entry in playlistCard(entry) }
                    }
                } else {
                    LazyVStack(spacing: 8) { ForEach(entries) { entry in playlistRow(entry) } }
                }
            }.padding(28)
        }
    }
    private var sourcePicker: some View {
        ThemePicker("来源", selection: $source) {
            Text("全部歌单").tag("all"); Text("本地").tag("local")
            ForEach(catalog.providers, id: \.providerKey) { provider in Text(provider["name"].text).tag(provider["id"].text) }
        }.frame(width: 230)
    }
    private var catalogActions: some View {
        HStack(spacing: 12) {
            if catalog.loading { ProgressView().controlSize(.small) }
            Button(grid ? "列表视图" : "卡片视图", systemImage: grid ? "list.bullet" : "square.grid.2x2") { grid.toggle() }.labelStyle(.iconOnly)
            Button("刷新云端目录", systemImage: "arrow.clockwise") { Task { await refreshCloud() } }.labelStyle(.iconOnly).disabled(busy)
            Button("导入", systemImage: "square.and.arrow.down") { importing = true }.help("导入播放列表")
            Button("智能歌单", systemImage: "sparkles") { smartPlaylist = nil; editingSmart = true }
            Button("创建", systemImage: "plus", action: create).help("创建播放列表")
        }.fixedSize()
    }
    private func playlistCard(_ entry: PlaylistEntry) -> some View {
        FeatureCard {
            Button { open(entry) } label: {
                VStack(alignment: .leading, spacing: 12) {
                    cover(entry).aspectRatio(1, contentMode: .fit)
                    Text(entry.value["name"].text).font(.headline).lineLimit(2).frame(height: 36, alignment: .topLeading)
                    Text(subtitle(entry)).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
            }.buttonStyle(.plain)
            HStack { action(entry); Spacer(minLength: 4); management(entry) }.controlSize(.small)
        }
    }
    private func playlistRow(_ entry: PlaylistEntry) -> some View {
        HStack(spacing: 14) {
            Button { open(entry) } label: { cover(entry).frame(width: 56, height: 56) }.buttonStyle(.plain)
            Button { open(entry) } label: { VStack(alignment: .leading, spacing: 6) { Text(entry.value["name"].text).font(.headline); Text(subtitle(entry)).font(.caption).foregroundStyle(.secondary) }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle()) }.buttonStyle(.plain)
            action(entry); management(entry)
        }.padding(12).background(.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 12))
    }
    private func cover(_ entry: PlaylistEntry) -> some View {
        ZStack {
            RoundedRectangle(cornerRadius: 12).fill(MusicStyle.accent.opacity(entry.cloud ? 0.08 : 0.15))
            if let artwork = entry.artwork {
                ArtworkView(track: artwork, store: store)
            } else { Image(systemName: entry.cloud ? "cloud" : "music.note.list").font(.system(size: 32, weight: .light)).foregroundStyle(MusicStyle.accent) }
        }.clipped()
    }
    @ViewBuilder private func action(_ entry: PlaylistEntry) -> some View {
        if entry.cloud { Button("同步到本地", systemImage: "arrow.down.circle") { Task { await mutate("/playlists/bind", body: ["provider": .string(entry.provider), "playlist_id": entry.value["id"], "account_context": entry.value["account_context"]]) } }.disabled(busy) }
        else { Button("播放", systemImage: "play.fill") { Task { busy = true; defer { busy = false }; await store.playPlaylist(entry.value["id"].text) } }.disabled(!store.canPlay || busy) }
    }
    private func management(_ entry: PlaylistEntry) -> some View {
        Menu("管理", systemImage: "ellipsis") {
            Button("打开歌单") { open(entry) }
            if !entry.cloud {
                if !entry.provider.isEmpty {
                    Button("立即同步") { Task { await mutate("/playlists/\(entry.value["id"].text.pathComponentEncoded)/sync") } }
                    Toggle("每小时拉取", isOn: Binding(get: { entry.value["sync"]["auto_sync"].boolean }, set: { value in Task { await mutate("/playlists/\(entry.value["id"].text.pathComponentEncoded)/sync", body: ["auto_sync": .bool(value)]) } }))
                }
                if let playlist = try? entry.value.decode(Playlist.self), playlist.id != store.collection.favoriteID {
                    Button("编辑智能规则") { smartPlaylist = playlist; editingSmart = true }
                    Button("重命名") { rename(playlist) }
                    Button("删除歌单", role: .destructive) { delete(playlist) }
                }
            }
        }.menuStyle(.borderlessButton).fixedSize().disabled(busy)
    }
    private func subtitle(_ entry: PlaylistEntry) -> String {
        let count = entry.cloud ? entry.value["track_count"].integer : entry.value["track_ids"].array.count
        let status = entry.value["sync"]["status"].text
        let labels = ["pending": "等待同步", "synced": "已同步", "partial": "部分未同步", "conflict": "同步冲突", "failed": "同步失败"]
        return (entry.value["smart_query"].object.isEmpty ? "\(catalog.name(entry.provider)) · \(count) 首" : "智能 · 动态匹配") + (entry.cloud ? entry.value["writable"].boolean ? " · 云端歌单" : " · 只读收藏" : status.isEmpty ? "" : " · \(labels[status] ?? status)")
    }
    private func open(_ entry: PlaylistEntry) {
        if entry.cloud { selected = entry }
        else { Task { busy = true; defer { busy = false }; await store.perform { try await store.collection.open(entry.value["id"].text, store: store); selected = entry } } }
    }
    private func refreshCloud() async {
        await mutate("/playlist-directory/refresh")
    }
    private func mutate(_ path: String, body: [String: JSONValue] = [:]) async {
        guard !busy else { return }; busy = true; defer { busy = false }
        await store.perform {
            _ = try await store.api(path, method: "POST", body: .object(body))
            await catalog.load(store); try await store.collection.refresh(store)
            store.notice = "同步任务已排队，可在 Core 后台任务查看。"
        }
    }
}
private extension JSONValue { var providerKey: String { self["id"].text } }

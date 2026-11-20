import SwiftUI

/// A single paged collection; changing location releases the previous page.
struct LibraryBrowserView: View {
    let store: MusicStore
    @StoredState private var filters: [String:String] = [:]
    @StoredState private var manageFiles = false
    @StoredState private var storage = ""
    @StoredState private var kind = "albums"
    @StoredState private var selectedID = ""
    @StoredState private var selectedName = ""
    @StoredState private var page = 0
    @StoredState private var resource = ResourceStore()
    @StoredState private var refreshID = UUID()
    @StoredState private var playback = WorkflowAction()
    private var showingTracks: Bool { kind == "tracks" || !selectedID.isEmpty }
    private var path: String {
        apiQuery("/library/\(kind == "folders" && selectedID.isEmpty ? "folders" : showingTracks ? "tracks" : kind)", [
            "storage": storage, "limit": "50", "offset": String(page * 50),
            "album_id": kind == "albums" ? selectedID : "", "artist_id": kind == "artists" ? selectedID : "",
            "folder": kind == "folders" ? selectedID : ""
        ].merging(filters.filter { !$0.value.isEmpty }) { _, value in value })
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                FeatureHeader(title: "音乐库", loading: resource.loading) { refreshID = UUID() }
                ThemePicker("存储来源", selection: $storage) { Text("全部来源").tag(""); Text("本地曲库").tag("local"); Text("Core 缓存库").tag("cache") }.pickerStyle(.segmented).frame(width: 400)
                ThemePicker("浏览方式", selection: $kind) { Text("专辑").tag("albums"); Text("艺术家").tag("artists"); Text("歌曲").tag("tracks"); if !storage.isEmpty && store.user["role"].text != "external" { Text("文件夹").tag("folders") } }.pickerStyle(.segmented).frame(width: 400)
                if !storage.isEmpty && store.user["role"].text == "admin" { Button("编辑与清理文件", systemImage: "slider.horizontal.3") { manageFiles = true } }
                if kind != "folders" { LibraryQueryControls(store: store, values: $filters, grouped: !showingTracks) }
                ResourceNotice(resource: resource)
                if !selectedID.isEmpty {
                    HStack { Button("返回全部", systemImage: "chevron.left") { resetLocation() }; Text(selectedName).font(.title2.bold()).lineLimit(2); Spacer() }
                }
                if showingTracks && !selectedID.isEmpty, let anchor = tracks.first { RelatedCatalogLinks(track: anchor, store: store) }
                HStack {
                    Text("\(resource.value["total"].integer) 项").foregroundStyle(.secondary)
                    if kind == "albums" && selectedID.isEmpty && resource.value["unassigned_total"].integer > 0 {
                        Button("未归属专辑 · \(resource.value["unassigned_total"].integer) 首") { kind = "tracks"; resetLocation(); filters["album_status"] = "unassigned" }.buttonStyle(MusicControlStyle()).foregroundStyle(MusicStyle.accent)
                    }
                }.font(.caption)
                if showingTracks {
                    HStack {
                        Button(playback.busy ? "正在准备…" : "播放全部", systemImage: "play.fill") {
                            let query = path
                            playback.run(store: store, operation: { try await store.playLibraryQuery(query) })
                        }.buttonStyle(WorkspacePrimaryButton()).disabled(tracks.isEmpty || !store.canPlay || playback.busy)
                        if playback.busy { Button("取消") { playback.cancel() } }
                        if let error = playback.error { Text(error).font(.caption).foregroundStyle(.secondary) }
                    }
                    SelectableTrackList(tracks: tracks, store: store, startIndex: page * 50, showCatalogDetails: true)
                } else if kind == "folders" {
                    LazyVStack(spacing: 12) { ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
                        Button { selectedID = item["path"].text; selectedName = item["name"].text; page = 0 } label: {
                            FeatureCard { HStack { Image(systemName: "folder").font(.title2); Text(item["name"].text); Spacer(); Text("\(item["track_count"].integer) 首").foregroundStyle(.secondary); Image(systemName: "chevron.right") } }
                        }.buttonStyle(.plain)
                    } }
                } else {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 150, maximum: 190), spacing: 22)], alignment: .leading, spacing: 24) {
                        ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
                            Button { selectedID = item["id"].text; selectedName = item[kind == "albums" ? "title" : "name"].text; page = 0 } label: {
                                VStack(alignment: .leading, spacing: 9) {
                                    ArtworkView(track: artwork(item), store: store, radius: kind == "artists" ? 75 : 12).frame(width: 150, height: 150)
                                    Text(item[kind == "albums" ? "title" : "name"].text).font(.headline).lineLimit(2)
                                    if kind == "albums" { HStack { Text(item["release_date"].text.isEmpty ? "发行日期未知" : item["release_date"].text); NewMusicBadge(date: item["release_date"].text, album: true) }.font(.caption2).foregroundStyle(.secondary) }
                                    HStack(spacing: 4) { if !item["artist"].text.isEmpty { CatalogTextLink(track: catalogSummary(item), store: store); Text("·") }; Text("\(item["track_count"].integer) 首") }.font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                }.frame(width: 150)
                            }.buttonStyle(MusicButtonStyle())
                        }
                    }
                }
                if resource.value["items"].array.isEmpty && !resource.loading { ContentUnavailableView("这里还没有音乐", systemImage: "music.note", description: Text("添加曲库目录或连接在线音源后再来看看。")) }
                HStack { Button("上一页") { page -= 1 }.disabled(page == 0 || resource.loading); Spacer(); Text("第 \(page + 1) 页").font(.caption); Spacer(); Button("下一页") { page += 1 }.disabled((page + 1) * 50 >= resource.value["total"].integer || resource.loading) }
            }.padding(30)
        }.task(id: "\(store.sessionID):\(path):\(refreshID)") { await load() }
        .sheet(isPresented: $manageFiles) { LibraryFilesView(store: store, storage: storage) }
        .onChange(of: storage) { if storage.isEmpty && kind == "folders" { kind = "albums" }; resetLocation() }
        .onChange(of: kind) { resetLocation(); if kind != "tracks" { filters.removeValue(forKey: "album_status") } }.onChange(of: filters) { page = 0 }
        .onChange(of: path) { playback.cancel() }.onDisappear { playback.cancel() }
    }
    private var tracks: [Track] { (try? resource.value["items"].decode([Track].self)) ?? [] }
    private func load() async { await resource.load(path, store: store) }
    private func resetLocation() { selectedID = ""; selectedName = ""; page = 0 }
    private func artwork(_ item: JSONValue) -> Track {
        Track(id: item["id"].text, title: item[kind == "albums" ? "title" : "name"].text, artist: item["artist"].text, album: item["title"].text, duration: 0, format: "", sample_rate: 0, bit_depth: 0, cover_url: item[kind == "albums" ? "cover_url" : "image_url"].string)
    }
}

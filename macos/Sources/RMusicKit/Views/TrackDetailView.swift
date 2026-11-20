import SwiftUI

struct TrackDetailView: View {
    let track: Track
    let store: MusicStore
    let close: () -> Void
    @StoredState private var editing = false
    @StoredState private var tab = "info"
    @StoredState private var resource = ResourceStore()
    @StoredState private var refreshID = UUID()
    @StoredState private var forceRefresh = false
    @StoredState private var audio = ResourceStore()
    @StoredState private var qualities = ResourceStore()
    private var displayedTrack: Track {
        var result = track
        if let image = resource.value["background"]["image_url"].string { result.cover_url = image.isEmpty ? nil : image }
        return result
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                Button("返回", systemImage: "chevron.left", action: close).buttonStyle(MusicControlStyle())
                HStack(spacing: 22) {
                    ArtworkView(track: displayedTrack, store: store, resolveMissing: true).frame(width: 124, height: 124)
                    VStack(alignment: .leading, spacing: 10) { Text(track.title).font(.title.bold()); CatalogTextLink(track: track, store: store).foregroundStyle(.secondary); CatalogTextLink(track: track, store: store, album: true).foregroundStyle(.secondary); Text(track.quality).font(.caption) }
                    Spacer()
                    if store.user["role"].text == "admin" { Button("编辑资料", systemImage: "pencil") { editing = true } }
                    Menu("歌曲操作") { TrackActions(track: track, store: store) }
                }
                RelatedCatalogLinks(track: track, store: store)
                ThemePicker("内容", selection: $tab) { Text("歌曲资料").tag("info"); Text("版本与录音").tag("versions"); if store.user["role"].text != "external" { Text("音乐身份").tag("identity") }; Text("乐评与评论").tag("comments") }.pickerStyle(.segmented).frame(maxWidth: 420)
                if tab == "comments" { TrackCommentsView(track: track, store: store) }
                else if tab == "identity" { MusicIdentityView(track: track, store: store).id(track.id) }
                else if tab == "versions" { MusicVersionsView(track: track, store: store).id(track.id) }
                else {
                    FeatureHeader(title: "关于这首歌", loading: resource.loading) { forceRefresh = true; refreshID = UUID() }
                    ResourceNotice(resource: resource)
                    FeatureCard {
                        Fact(name: "文件格式", value: track.quality)
                        Fact(name: "文件时长", value: listeningTime(track.duration))
                        if audio.value["channels"].integer > 0 { Fact(name: "声道", value: String(audio.value["channels"].integer)) }
                        Fact(name: "实际音源", value: audio.value["quality_label"].text)
                        Fact(name: "可用音质", value: qualities.value["items"].array.map { $0["label"].text }.joined(separator: " · "))
                        let info = resource.value["background"]
                        ForEach(["release_date", "genre", "language", "label"], id: \.self) { key in Fact(name: ["release_date": "发行时间", "genre": "流派", "language": "语言", "label": "唱片公司"][key]!, value: info[key].text) }
                        if resource.value["matched"].boolean == false { Text("暂无可确认的扩展资料，保留文件原始信息。").foregroundStyle(.secondary) }
                        if !info["description"].text.isEmpty { Text(info["description"].text).textSelection(.enabled) }
                        if !info["album_description"].text.isEmpty { Text(info["album_description"].text).textSelection(.enabled) }
                        ForEach(Array(info["credits"].array.enumerated()), id: \.offset) { _, credit in Fact(name: credit["role"].text, value: credit["names"].array.map(\.text).joined(separator: "、")) }
                        ForEach(Array(resource.value["artists"].array.enumerated()), id: \.offset) { _, artist in Text(artist["name"].text).font(.headline); Text(artist["description"].text).foregroundStyle(.secondary).textSelection(.enabled) }
                        Fact(name: "资料来源", value: resource.value["source"].text)
                    }
                }
            }.padding(30)
        }.sheet(isPresented: $editing) { MediaProfileEditor(store: store, kind: "track", id: track.id, close: { editing = false }, saved: { refreshID = UUID() }) }.task(id: "\(store.sessionID):\(track.id):\(refreshID)") { let force = forceRefresh; forceRefresh = false; await load(refresh: force) }
    }
    private func load(refresh: Bool) async {
        let path = "/library/tracks/\(track.id.pathComponentEncoded)"
        async let background = resource.load(path + "/background?refresh=\(refresh)", store: store, identity: path + "/background")
        async let format = audio.load(path + "/audio-info", store: store)
        _ = await (background, format)
        if resource.status == 400 && resource.error?.contains("尚无已启用的资料连接器") == true { resource.error = nil }

        if let source = track.source, source != "local" { await qualities.load("/providers/\(source.pathComponentEncoded)/tracks/\(track.id.pathComponentEncoded)/qualities", store: store) }
    }
}

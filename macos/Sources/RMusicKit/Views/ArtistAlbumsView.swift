import SwiftUI

/// Discography pages are loaded only while the artist's album tab is visible.
struct ArtistAlbumsView: View {
    let store: MusicStore
    let artistID: String
    let anchor: Track
    @StoredState private var resource = ResourceStore()
    @StoredState private var sort = "default"
    @StoredState private var localSort = "added"
    @StoredState private var page = 1
    @StoredState private var provider = ""
    @StoredState private var revision = UUID()
    @StoredState private var refresh = false
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            FeatureHeader(title: "专辑", loading: resource.loading) { refresh = true; revision = UUID() }
            ResourceNotice(resource: resource)
            let platforms = resource.value["platforms"].array
            ForEach(Array(platforms.enumerated()), id: \.offset) { _, platform in
                albumSection(platform, title: platform["source"].text == "qqmusic" ? "QQ 音乐" : platform["source"].text, source: platform["source"].text)
            }
            let local = resource.value["local"]
            if local["total"].integer > 0 {
                DisclosureGroup("已收录专辑 · \(local["total"].integer) 张") { albumSection(local, title: "已收录专辑", source: "local").padding(.top, 16) }
            }
            if !resource.loading && resource.error == nil && platforms.allSatisfy({ $0["items"].array.isEmpty }) && local["items"].array.isEmpty {
                Text("暂无可确认的专辑").foregroundStyle(.secondary)
            }
        }.onChange(of: sort) { page = 1 }.onChange(of: localSort) { page = 1 }.task(id: "\(store.sessionID):\(artistID):\(provider):\(page):\(revision):\(sort):\(localSort)") {
            let force = refresh; refresh = false
            await resource.load(apiQuery("/library/catalog/artist/\(artistID.pathComponentEncoded)/albums", ["page": String(page), "provider": provider, "refresh": String(force),"sort": sort,"local_sort": localSort]), store: store, identity: artistID)
        }
    }
    private func albumSection(_ value: JSONValue, title: String, source: String) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(title).font(.title3.weight(.semibold)); Spacer(); CatalogSortPicker(title: source == "local" ? "本地专辑排序" : "平台专辑排序", value: source == "local" ? $localSort : $sort, options: source == "local" ? ["added","title","title_desc","release_new","release_old","plays"] : value["sorts"].array.isEmpty ? ["default"] : value["sorts"].array.map(\.text)); if value["total"].number > 0 { Text("\(value["total"].integer) 张").font(.caption).foregroundStyle(.secondary) } }

            if let notice = value["notice"].string { Text(notice).font(.callout).foregroundStyle(.secondary) }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 146, maximum: 210), spacing: 22)], alignment: .leading, spacing: 24) {
                ForEach(Array(value["items"].array.enumerated()), id: \.offset) { _, album in
                    Button { Task { await store.openCatalogReference(CatalogReference(id: album["id"].text, name: album["title"].text, detail: album["source"].text), track: anchor, album: true) } } label: {
                        VStack(alignment: .leading, spacing: 9) {
                            ArtworkView(track: picture(album), store: store, radius: 12).aspectRatio(1, contentMode: .fit)
                            Text(album["title"].text).font(.system(size: 13, weight: .semibold)).lineLimit(2).frame(height: 34, alignment: .topLeading)
                            HStack { Text(album["release_date"].text); NewMusicBadge(date: album["release_date"].text, album: true) }.font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                    }.buttonStyle(.plain).help("打开专辑 \(album["title"].text)")
                }
            }
            let current = max(1, value["page"].integer)
            if current > 1 || value["next_page"].integer > 0 {
                HStack { Button("上一页") { provider = source; page = current - 1 }.disabled(current == 1); Spacer(); Text("第 \(current) 页").font(.caption); Spacer(); Button("下一页") { provider = source; page = value["next_page"].integer }.disabled(value["next_page"].integer == 0) }.disabled(resource.loading)
            }
        }
    }
    private func picture(_ album: JSONValue) -> Track {
        var result = anchor; result.cover_url = album["cover_url"].string
        return result
    }
}

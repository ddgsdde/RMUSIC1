import SwiftUI

/// A single entity owns its biography, library and paged platform catalog.
struct CatalogDetailView: View {
    let store: MusicStore
    let id: String
    let kind: String
    @StoredState private var resource = ResourceStore()
    @StoredState private var sort = "default"
    @StoredState private var platformSort = "default"
    @StoredState private var page = 1
    @StoredState private var localPage = 1
    @StoredState private var provider = ""
    @StoredState private var section = "all"
    @StoredState private var expanded = false
    @StoredState private var refresh = UUID()
    @StoredState private var forceRefresh = false
    @StoredState private var editing = false
    private var path: String { apiQuery("/library/catalog/\(kind)/\(id.pathComponentEncoded)", ["page": String(page), "local_page": String(localPage), "provider": section == "library" ? "" : provider,"sort": sort,"platform_sort": platformSort,"remote": section == "library" ? "false" : "true"]) }
    private var hasData: Bool { if case .null = resource.value { return false }; return true }
    private var platforms: [JSONValue] { resource.value["platforms"].array }
    private var local: [Track] { (try? resource.value["local"]["items"].decode([Track].self)) ?? [] }
    private var visibleLocal: [Track] {
        guard section == "all" else { return local }
        let ids = Set(platforms.flatMap { $0["items"].array.map { $0["id"].text } })
        return local.filter { !ids.contains($0.id) }
    }
    private var remoteSorting: Bool { section != "library" && !platforms.isEmpty }
    private var sortOptions: [String] {
        if remoteSorting { let options = platforms.first?["sorts"].array.map(\.text) ?? []; return options.isEmpty ? ["default"] : options }
        return ["default","title","title_desc","release_new","release_old","duration","duration_desc","plays"]
    }
    private var title: String { if kind == "album", let track = try? resource.value["anchor"].decode(Track.self), !track.hasAlbum { return "未归属专辑" }; return resource.value["name"].string ?? (kind == "artist" ? store.artistName : store.albumName) ?? "音乐资料" }
    private var profile: JSONValue { resource.value["profile"] }
    private var artwork: Track? {
        guard var track = (try? resource.value["anchor"].decode(Track.self)) ?? store.catalogAnchor ?? store.tracks.first else { return nil }
        if let image = profile["image_url"].string { track.cover_url = image }
        else if kind == "artist" { return nil }
        return track
    }
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 24) {
                Button("全部音乐", systemImage: "chevron.left") { Task { await store.closeAlbum() } }.buttonStyle(MusicControlStyle()).foregroundStyle(.secondary)
                header
                CatalogSourcesView(store: store, kind: kind, id: id, detail: resource.value) { refresh = UUID() }
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 14) { scopeButtons; Spacer(minLength: 14); actions }
                    VStack(alignment: .leading, spacing: 12) { ScrollView(.horizontal) { scopeButtons }.scrollIndicators(.hidden).rememberPageScroll(store, key: "catalog:\(kind):\(id):\(section)"); HStack { Spacer(); actions } }
                }
                ResourceNotice(resource: resource)
                if section == "albums", let anchor = (try? resource.value["anchor"].decode(Track.self)) ?? store.catalogAnchor {
                    ArtistAlbumsView(store: store, artistID: id, anchor: anchor)
                } else {
                if section != "library" {
                    ForEach(Array(platforms.enumerated()), id: \.offset) { _, platform in platformSection(platform) }
                    if platforms.isEmpty && !resource.loading && resource.error == nil { Text("暂无可确认的平台目录").foregroundStyle(.secondary) }
                }
                if section == "library" || (section == "all" && !resource.loading && (!visibleLocal.isEmpty || resource.value["local"]["next_page"].integer > 0)) {
                    Text("本地曲库").font(.title3.weight(.semibold))
                    if visibleLocal.isEmpty { Text("本地曲库暂无曲目").foregroundStyle(.secondary) }
                    ForEach(Array(visibleLocal.enumerated()), id: \.element.id) { index, track in MusicTrackRow(track: track, index: (localPage - 1) * 50 + index, store: store, play: { Task { await store.playList(visibleLocal, starting: track.id) } }, showCatalogDetails: true) }
                    if resource.value["local"]["next_page"].integer > 0 || localPage > 1 {
                        HStack { Button("上一页") { localPage -= 1 }.disabled(localPage == 1); Text("第 \(localPage) 页").font(.caption); Button("下一页") { localPage += 1 }.disabled(resource.value["local"]["next_page"].integer == 0) }.disabled(resource.loading)
                    }
                }
                }

            }.padding(30)
        }.scrollIndicators(.hidden).onChange(of: sort) { localPage = 1 }.onChange(of: platformSort) { page = 1 }.sheet(isPresented: $editing) { MediaProfileEditor(store: store, kind: kind, id: id, close: { editing = false }, saved: { forceRefresh = false; refresh = UUID() }) }
            .task(id: "\(store.sessionID):\(path):\(refresh)") {
                let forced = forceRefresh; forceRefresh = false
                if (section == "library" && !forced) || !hasData { _ = await resource.load(path.replacingOccurrences(of: "remote=true", with: "remote=false"), store: store, identity: "\(kind):\(id)") }
                if !Task.isCancelled && (section != "library" || forced) {
                    let target = forced ? path.replacingOccurrences(of: "remote=false", with: "remote=true") : path
                    _ = await resource.load(target + "&refresh=\(forced)", store: store, identity: "\(kind):\(id)")
                }
            }
    }
    private var scopeButtons: some View {
        HStack(spacing: 10) {
            sectionButton("全部曲目", "all")
            if kind == "artist" { sectionButton("专辑", "albums") }
            sectionButton("本地曲库 · \(resource.value["local"]["total"].integer)", "library")
        }.fixedSize()
    }
    private var actions: some View {
        HStack(spacing: 12) {
            if section != "albums" {
                CatalogSortPicker(title: remoteSorting ? "平台曲目排序" : "本地曲目排序", value: Binding(get: { remoteSorting ? platformSort : sort }, set: { value in
                    if remoteSorting { platformSort = value; page = 1 }
                    if value != "hot" { sort = value; localPage = 1 }
                }), options: sortOptions, compact: true).frame(width: 175).disabled(resource.loading)
            }
            if resource.loading { ProgressView().controlSize(.small) }
            if store.user["role"].text == "admin" { Button("编辑资料", systemImage: "pencil") { editing = true }.buttonStyle(MusicControlStyle()) }
            Button { forceRefresh = true; refresh = UUID() } label: { Image(systemName: "arrow.clockwise") }.buttonStyle(MusicControlStyle()).help("更新资料").accessibilityLabel("更新资料")
        }.fixedSize()
    }
    private var header: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(alignment: .top, spacing: 24) {
                if kind == "artist" && artwork == nil {
                    Image(systemName: "person.fill").font(.system(size: 46)).foregroundStyle(MusicStyle.accent).frame(width: 150, height: 150).background(MusicStyle.accent.opacity(0.09), in: Circle())
                } else { ArtworkView(track: artwork, store: store, radius: kind == "artist" ? 75 : 14).frame(width: 150, height: 150) }
                VStack(alignment: .leading, spacing: 12) {
                    Text(kind == "artist" ? "艺术家" : "专辑").font(.caption.weight(.medium)).foregroundStyle(MusicStyle.accent)
                    Text(title).font(.system(size: 32, weight: .bold)).textSelection(.enabled)
                    if kind == "album", let track = (try? resource.value["anchor"].decode(Track.self)) ?? store.catalogAnchor ?? store.tracks.first { CatalogTextLink(track: track, store: store).font(.callout) }
                    let seed = local.first ?? (try? resource.value["anchor"].decode(Track.self))
                    if let seed {
                        Button("启动电台", systemImage: "dot.radiowaves.left.and.right") { Task { await store.startRadio(seed) } }.buttonStyle(MusicControlStyle()).disabled(!store.canPlay || seed.available == false)
                    }
                    facts
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            if let description = profile["description"].string, !description.isEmpty {
                VStack(alignment: .leading, spacing: 10) {
                    Text(description).font(.system(size: 13)).lineSpacing(6).lineLimit(expanded ? nil : 4).textSelection(.enabled)
                    Button(expanded ? "收起简介" : "展开完整简介") { expanded.toggle() }.buttonStyle(MusicControlStyle()).foregroundStyle(MusicStyle.accent).font(.caption)
                    if let source = resource.value["provenance"]["description"].string { Text("资料来源 · \(source)").font(.caption2).foregroundStyle(.secondary) }
                }.padding(18).frame(maxWidth: .infinity, alignment: .leading).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 14))
            }
        }
    }
    private var facts: some View {
        VStack(alignment: .leading, spacing: 5) {
            ForEach([("release_date", "发行"), ("country", "地区"), ("genre", "风格"), ("language", "语言"), ("label", "唱片公司"), ("birthday", "生日"), ("birthplace", "出生地"), ("alias", "别名"), ("company", "公司"), ("foreign_name", "外文名"), ("identity", "身份"), ("instrument", "乐器"), ("enter", "出道"), ("tag", "标签"), ("area", "地区")], id: \.0) { key, label in
                if let value = profile[key].string, !value.isEmpty { Text("\(label) · \(value)").font(.caption).foregroundStyle(.secondary).textSelection(.enabled) }
            }
        }
    }
    private func sectionButton(_ title: String, _ value: String) -> some View {
        Button { section = value } label: { Text(title).font(.system(size: 12, weight: .medium)).padding(.horizontal, 15).padding(.vertical, 9).contentShape(Capsule()).background(section == value ? MusicStyle.accent.opacity(0.18) : .primary.opacity(0.035), in: Capsule()).foregroundStyle(section == value ? MusicStyle.accent : Color.secondary) }.buttonStyle(.plain)
    }
    @ViewBuilder private func platformSection(_ platform: JSONValue) -> some View {
        let tracks = (try? platform["items"].decode([Track].self)) ?? []
        VStack(alignment: .leading, spacing: 14) {
            HStack { Text(platform["name"].string ?? platform["source"].text).font(.title3.weight(.semibold)); Text("\(platform["total"].integer) 首").font(.caption).foregroundStyle(.secondary); Spacer() }
            if !platform["notice"].text.isEmpty { Text(platform["notice"].text).font(.caption).foregroundStyle(.secondary) }
            ForEach(Array(tracks.enumerated()), id: \.element.id) { index, track in MusicTrackRow(track: track, index: (max(1, platform["page"].integer) - 1) * 30 + index, store: store, play: { Task { await store.playList(tracks, starting: track.id) } }, showCatalogDetails: true) }
            if platform["next_page"].integer > 0 || platform["page"].integer > 1 {
                HStack { Button("上一页") { provider = platform["source"].text; page = max(1, platform["page"].integer - 1) }.disabled(platform["page"].integer <= 1); Text("第 \(max(1, platform["page"].integer)) 页").font(.caption); Button("下一页") { provider = platform["source"].text; page = platform["next_page"].integer }.disabled(platform["next_page"].integer == 0) }.disabled(resource.loading)
            }
        }
    }
}

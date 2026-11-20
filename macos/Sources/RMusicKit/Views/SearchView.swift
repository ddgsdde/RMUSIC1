import SwiftUI

/// The indexed local results stay available while the combined search runs.
struct SearchView: View {
    let store: MusicStore
    @StoredState private var local = ResourceStore()
    @StoredState private var combined = ResourceStore()
    @StoredState private var localOnly = false
    private var resource: ResourceStore { localOnly || combined.value == .null ? local : combined }
    private var busy: Bool { local.loading || (!localOnly && combined.loading) }
    @StoredState private var category = "all"
    @StoredState private var limit = 24
    private var tracks: [Track] { (try? resource.value["items"].decode([Track].self)) ?? [] }
    private var artistCount: Int { resource.value["artists"].array.filter { $0["relevance"].integer > 0 }.count }
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 24) {
                HStack(alignment: .firstTextBaseline) {
                    Text(store.query.isEmpty ? "搜索音乐" : "“\(store.query)”").font(.system(size: 28, weight: .bold))
                    Spacer()
                    if busy { ProgressView().controlSize(.small).accessibilityLabel(localOnly ? "正在搜索" : "正在搜索全部音源") }
                }
                HStack(spacing: 8) {
                    ForEach([("all", "综合"), ("songs", "歌曲"), ("albums", "专辑"), ("artists", "艺术家")], id: \.0) { key, title in
                        Button { category = key; limit = 24 } label: {
                            Text(title).font(.headline).padding(.horizontal, 20).padding(.vertical, 10)
                                .background(category == key ? MusicStyle.accent.opacity(0.22) : Color.primary.opacity(0.04), in: Capsule())
                                .foregroundStyle(category == key ? MusicStyle.accent : .primary)
                        }.buttonStyle(.plain)
                    }
                }
                Picker("搜索范围", selection: $localOnly) {
                    Text("全部音源").tag(false)
                    Text("本地音源").tag(true)
                }.pickerStyle(.segmented).labelsHidden().frame(maxWidth: 240)
                ResourceNotice(resource: resource)
                if !localOnly, let error = combined.error, resource !== combined {
                    Text(error).font(.callout).foregroundStyle(.secondary)
                }
                if let notice = resource.value["notice"].string { Text(notice).font(.callout).foregroundStyle(.secondary) }
                if !store.query.isEmpty && !busy && resource.error == nil && tracks.isEmpty {
                    ContentUnavailableView.search(text: store.query)
                }
                if category == "all", !tracks.isEmpty { bestMatch }
                if category == "all" || category == "artists" { catalog("artists", title: "艺术家", count: category == "all" ? 6 : limit) }
                if category == "all" || category == "albums" { catalog("albums", title: "专辑", count: category == "all" ? 6 : limit) }
                if category == "all" || category == "songs" {
                    if !tracks.isEmpty { heading("歌曲", key: "songs") }
                    ForEach(Array(tracks.prefix(category == "all" ? 8 : limit).enumerated()), id: \.element.id) { index, track in
                        MusicTrackRow(track: track, index: index, store: store, play: { Task { await store.playList(tracks, starting: track.id) } })
                    }
                }
                if category == "artists" && artistCount == 0 && !busy && !tracks.isEmpty { Text("没有名称匹配的艺术家").foregroundStyle(.secondary) }
                if category != "all" && limit < (category == "songs" ? tracks.count : category == "artists" ? artistCount : resource.value[category].array.count) {
                    Button("显示更多") { limit += 24 }.buttonStyle(.bordered)
                }
            }.padding(30)
        }
        .task(id: "\(store.sessionID):\(store.query)") {
            local = ResourceStore(); limit = 24
            let term = store.query.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !term.isEmpty else { return }
            do { try await Task.sleep(for: .milliseconds(120)); try Task.checkCancellation() }
            catch { return }
            let resource = local
            await resource.load(apiQuery("/library/search", ["q": term]), store: store)
            if resource.status == 404, !Task.isCancelled {
                await resource.load(apiQuery("/library/tracks", ["q": term, "source": "local", "limit": "100"]), store: store)
            }
        }
        .task(id: "\(store.sessionID):\(store.query):\(localOnly)") {
            combined = ResourceStore(); limit = 24
            let term = store.query.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !localOnly, !term.isEmpty else { return }
            do { try await Task.sleep(for: .milliseconds(300)); try Task.checkCancellation() }
            catch { return }
            await combined.load(apiQuery("/search", ["q": term]), store: store)
        }
    }
    @ViewBuilder private var bestMatch: some View {
        let query = store.query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if let artist = resource.value["artists"].array.first(where: { $0["name"].text.lowercased() == query }), let track = try? artist["track"].decode(Track.self) {
            Button { Task { await store.openCatalogReference(CatalogReference(id: artist["id"].text, name: artist["name"].text, detail: artist["source"].text), track: track, album: false) } } label: {
                HStack(spacing: 18) {
                    if let url = artist["image_url"].string {
                        ArtworkView(track: Track(id: "artist:" + artist["id"].text, title: artist["name"].text, artist: artist["name"].text, album: "", duration: 0, format: "", sample_rate: 0, bit_depth: 0, cover_url: url), store: store, radius: 32).frame(width: 64, height: 64)
                    } else { Image(systemName: "person.crop.circle").font(.system(size: 46, weight: .light)).foregroundStyle(MusicStyle.accent) }
                    VStack(alignment: .leading, spacing: 7) { Text("最相关 · 艺术家").font(.caption).foregroundStyle(.secondary); Text(artist["name"].text).font(.title2.bold()) }
                    Spacer(); Image(systemName: "arrow.up.right").foregroundStyle(.secondary)
                }.padding(24).background(MusicStyle.accent.opacity(0.10), in: RoundedRectangle(cornerRadius: 18))
            }.buttonStyle(.plain)
        } else if let track = tracks.first {
            VStack(alignment: .leading, spacing: 12) { Text("最相关 · 歌曲").font(.headline); MusicTrackRow(track: track, index: 0, store: store) }.padding(16).background(MusicStyle.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 18))
        }
    }
    private func heading(_ title: String, key: String) -> some View {
        HStack { Text(title).font(.title2.bold()); Spacer(); if category == "all" { Button("查看全部", systemImage: "arrow.up.right") { category = key }.buttonStyle(MusicControlStyle()).foregroundStyle(.secondary) } }
    }
    @ViewBuilder private func catalog(_ key: String, title: String, count: Int) -> some View {
        let rows = resource.value[key].array.filter { category != "artists" || $0["relevance"].integer > 0 }
        if !rows.isEmpty {
            heading(title, key: key)
            if category == "all" {
                ScrollView(.horizontal) { LazyHStack(alignment: .top, spacing: 18) {
                    ForEach(Array(rows.prefix(count).enumerated()), id: \.offset) { _, item in SearchCatalogCard(item: item, album: key == "albums", store: store).frame(width: 168) }
                } }.scrollIndicators(.hidden)
            } else {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 150, maximum: 220), spacing: 20)], alignment: .leading, spacing: 22) {
                    ForEach(Array(rows.prefix(count).enumerated()), id: \.offset) { _, item in SearchCatalogCard(item: item, album: key == "albums", store: store) }
                }
            }
        }
    }
}
private struct SearchCatalogCard: View {
    let item: JSONValue
    let album: Bool
    let store: MusicStore
    var body: some View {
        if let track = try? item["track"].decode(Track.self) {
            VStack(alignment: .leading, spacing: 10) {
                Button { Task { await store.openCatalogReference(CatalogReference(id: item["id"].text, name: item[album ? "title" : "name"].text, detail: item["source"].text), track: track, album: album) } } label: {
                    VStack(alignment: .leading, spacing: 12) {
                        if album { Color.clear.aspectRatio(1, contentMode: .fit).overlay { ArtworkView(track: track, store: store) } }
                        else if let url = item["image_url"].string { Color.clear.aspectRatio(1, contentMode: .fit).overlay { ArtworkView(track: Track(id: "artist:" + item["id"].text, title: item["name"].text, artist: item["name"].text, album: "", duration: 0, format: "", sample_rate: 0, bit_depth: 0, cover_url: url), store: store, radius: 100) } }
                        else { Image(systemName: "person.fill").font(.system(size: 46, weight: .light)).foregroundStyle(MusicStyle.accent.opacity(0.8)).frame(maxWidth: .infinity).aspectRatio(1, contentMode: .fit).background(MusicStyle.accent.opacity(0.09), in: Circle()).padding(12) }
                        Text(item[album ? "title" : "name"].text).font(.headline).lineLimit(2).frame(maxWidth: .infinity, alignment: .leading)
                    }
                }.buttonStyle(MusicButtonStyle())
                if album { CatalogTextLink(track: track, store: store).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                Text(item["source"].text == "qqmusic" ? "QQ 音乐" : item["source"].text == "local" ? "本地音源" : item["source"].text).font(.caption2).foregroundStyle(.tertiary)
            }.padding(14).background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 18))
        }
    }
}

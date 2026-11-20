import SwiftUI

struct DiscoverView: View {
    let store: MusicStore
    @StoredState private var refreshID = UUID()
    @StoredState private var forceRefresh = false
    @StoredState private var platformTab = false
    @StoredState private var platform = ResourceStore()
    @StoredState private var platformRefresh = 0
    @StoredState private var providerPlaylist: JSONValue?
    @StoredState private var provider = ""
    private var feed: ResourceStore { store.discoveryFeed.recommendations }

    var body: some View {
        if let providerPlaylist {
            ProviderLibraryView(store: store, providerID: provider, playlist: providerPlaylist) { self.providerPlaylist = nil }
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 22) {
                    FeatureHeader(title: "发现音乐", loading: platformTab ? platform.loading : store.discoveryFeed.updating) {
                        if platformTab { platformRefresh += 1 } else { forceRefresh = true }
                        refreshID = UUID()
                    }
                    Picker("发现栏目", selection: $platformTab) {
                        Text("为你推荐").tag(false)
                        Text("平台推荐").tag(true)
                    }.pickerStyle(.segmented).frame(maxWidth: 310)
                    if platformTab {
                        ResourceNotice(resource: platform)
                        let capability = store.user["role"].text == "external" ? "public_recommendations_v1" : "recommendations"
                        let entries = platform.value["items"].array.filter {
                            $0["enabled"].boolean && $0["capabilities"].array.contains { $0.text == capability }
                        }
                        if entries.isEmpty && !platform.loading { Text("暂无可用的推荐平台。").foregroundStyle(.secondary) }
                        ForEach(Array(entries.enumerated()), id: \.offset) { _, item in
                            ProviderRecommendationPanel(store: store, descriptor: item, refreshID: platformRefresh) { section, list in
                                provider = section["source"].text
                                providerPlaylist = list
                            }.id(platformKey(item))
                        }
                    } else {
                        ResourceNotice(resource: feed)
                        let result = store.discoveryFeed
                        if result.items.isEmpty && feed.error == nil && (feed.value.object.isEmpty || result.updating) {
                            DiscoveryLoadingRail()
                        } else if result.items.isEmpty && feed.error == nil {
                            Text("暂时没有可推荐的歌曲。")
                                .foregroundStyle(.secondary)
                        } else {
                            RecommendationTrackShelf(title: "为你推荐", store: store, tracks: result.items, reasons: feed.value["reasons"], personal: true)
                        }
                        ForEach(result.shelves) { section in
                            RecommendationTrackShelf(title: section.title, store: store, tracks: section.tracks, reasons: feed.value["reasons"], personal: true)
                        }
                    }
                }.padding(30)
            }.task(id: "\(store.sessionID):\(platformTab):\(refreshID)") {
                let force = forceRefresh
                forceRefresh = false
                if platformTab { _ = await platform.load("/plugins", store: store) }
                else { await store.discoveryFeed.load(store, refresh: force) }
            }.task(id: store.sessionID) {
                _ = await platform.load("/plugins", store: store)
                guard !Task.isCancelled, store.user["role"].text != "external",
                      let daily = platform.value["items"].array.first(where: { item in
                          item["enabled"].boolean && item["capabilities"].array.contains { $0.text == "recommendations" }
                          && (item["id"].text == "qqmusic" || item["capabilities"].array.contains { $0.text == "daily_recommendations_v1" })
                      }) else { return }
                let resource = ResourceStore()
                await resource.load(platformURL(daily, page: 1, refresh: false), store: store, snapshot: platformKey(daily) + "/1", reportConnectionFailure: false)
            }
        }
    }
}

private func recommendationTracks(_ value: JSONValue) -> [Track] { (try? value.decode([Track].self)) ?? [] }

private struct RecommendationTrackShelf: View {
    let title: String
    let store: MusicStore
    let tracks: [Track]
    var reasons = JSONValue.null
    var personal = false
    @StoredState private var reduced = Set<String>()

    var body: some View {
        if !tracks.isEmpty {
            Text(title).font(.title2.bold())
            ScrollView(.horizontal) {
                LazyHStack(alignment: .top, spacing: 16) {
                    ForEach(tracks) { track in
                        VStack(alignment: .leading, spacing: 6) {
                            ZStack(alignment: .topTrailing) {
                                Button { Task { await store.playList(tracks, starting: track.id) } } label: {
                                    ArtworkView(track: track, store: store).frame(width: 142, height: 142)
                                }.buttonStyle(.plain).disabled(!store.canPlay)
                                if personal {
                                    Menu {
                                        Button("启动电台") { Task { await store.startRadio(track) } }.disabled(!store.canPlay)
                                        Button("加入队列") { Task { await store.enqueue(track) } }.disabled(!store.canPlay)
                                        Button(reduced.contains(track.id) ? "撤销减少推荐" : "少推荐这首") { Task { await feedback(track) } }
                                    } label: {
                                        Image(systemName: "ellipsis.circle.fill")
                                            .font(.system(size: 21))
                                            .symbolRenderingMode(.palette)
                                            .foregroundStyle(.white, .black.opacity(0.75))
                                            .padding(7)
                                    }.menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().accessibilityLabel("推荐操作")
                                }
                            }
                            Button { store.detailTrack = track } label: {
                                Text(track.title).font(.headline).lineLimit(1)
                            }.buttonStyle(.plain)
                            CatalogTextLink(track: track, store: store).font(.caption).lineLimit(1)
                            if personal {
                                if let reason = reasons[track.id].string {
                                    Text(reason).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                                }
                                Text(track.source == "local" ? "本地" : "云端").font(.caption2.weight(.medium)).foregroundStyle(MusicStyle.accent)
                            }
                        }.frame(width: 142).contextMenu { TrackActions(track: track, store: store) }
                    }
                }
            }.scrollIndicators(.hidden)
        }
    }

    private func feedback(_ track: Track) async {
        await store.perform {
            let less = !reduced.contains(track.id)
            _ = try await store.api("/recommendations/feedback/\(track.id.pathComponentEncoded)", method: "PUT", body: .object(["less": .bool(less)]))
            if less { reduced.insert(track.id) } else { reduced.remove(track.id) }
        }
    }
}

private func platformKey(_ item: JSONValue) -> String {
    "/platform-recommendations/" + ["id", "version", "account_context"].map { item[$0].text.pathComponentEncoded }.joined(separator: "/")
}
private func platformURL(_ item: JSONValue, page: Int, refresh: Bool) -> String {
    "/providers/\(item["id"].text.pathComponentEncoded)/recommendations?page=\(page)&refresh=\(refresh)"
}

private struct ProviderRecommendationPanel: View {
    let store: MusicStore
    let descriptor: JSONValue
    let refreshID: Int
    let open: (JSONValue, JSONValue) -> Void
    @StoredState private var expanded = true
    @StoredState private var resource = ResourceStore()
    @StoredState private var sections = [JSONValue]()
    private var firstPage: JSONValue {
        store.pageSnapshots.value(platformKey(descriptor) + "/1", owner: store.sessionID) ?? .null
    }

    var body: some View {
        DisclosureGroup(descriptor["name"].text, isExpanded: $expanded) {
            VStack(alignment: .leading, spacing: 16) {
                ResourceNotice(resource: resource)
                ForEach(Array((sections.isEmpty ? firstPage["sections"].array : sections).enumerated()), id: \.offset) { _, section in
                    if section["kind"].text == "albums" {
                        Text(section["title"].text).font(.title2.bold())
                        ScrollView(.horizontal) {
                            LazyHStack(spacing: 16) {
                                ForEach(recommendationTracks(section["items"])) { track in
                                    Button { Task { await store.openAlbum(track) } } label: {
                                        VStack(alignment: .leading) {
                                            ArtworkView(track: track, store: store).frame(width: 142, height: 142)
                                            Text(track.album).lineLimit(1)
                                        }.frame(width: 142)
                                    }.buttonStyle(.plain)
                                }
                            }
                        }.scrollIndicators(.hidden)
                    } else {
                        RecommendationTrackShelf(title: section["title"].text, store: store, tracks: recommendationTracks(section["items"]))
                    }
                    if !section["playlists"].array.isEmpty {
                        Text(section["title"].text).font(.title2.bold())
                        ScrollView(.horizontal) {
                            LazyHStack(spacing: 16) {
                                ForEach(Array(section["playlists"].array.enumerated()), id: \.offset) { _, list in
                                    Button { open(section, list) } label: {
                                        VStack(alignment: .leading) {
                                            ArtworkView(track: catalogSummary(list), store: store).frame(width: 142, height: 142)
                                            Text(list["name"].text).lineLimit(2)
                                        }.frame(width: 142)
                                    }.buttonStyle(.plain)
                                }
                            }
                        }.scrollIndicators(.hidden)
                    }
                }
                let page = (resource.value == .null ? firstPage : resource.value)["next_page"].integer
                if page > 0 {
                    Button("加载更多") { Task { await load(page, refresh: false) } }.disabled(resource.loading)
                }
            }.padding(.top, 12)
        }.task(id: "\(store.sessionID):\(descriptor["account_context"].text):\(descriptor["version"].text):\(refreshID):\(expanded)") {
            if expanded { await load(1, refresh: refreshID > 0) }
        }
    }

    private func load(_ page: Int, refresh: Bool) async {
        let url = platformURL(descriptor, page: page, refresh: refresh)
        if await resource.load(url, store: store, snapshot: platformKey(descriptor) + "/\(page)", reportConnectionFailure: false), !Task.isCancelled {
            let next = resource.value["sections"].array
            sections = page == 1 ? next : (sections.isEmpty ? firstPage["sections"].array : sections) + next
        }
    }
}

struct TrackRail: View {
    let tracks: [Track]
    let store: MusicStore
    var body: some View {
        ScrollView(.horizontal) {
            LazyHStack(alignment: .top, spacing: 20) {
                ForEach(tracks.prefix(12)) { track in
                    VStack(alignment: .leading, spacing: 10) {
                        Button { Task { await store.playList(tracks, starting: track.id) } } label: { ArtworkView(track: track, store: store).frame(width: 155, height: 155).overlay(alignment: .bottomTrailing) { Image(systemName: "play.fill").padding(10).background(.regularMaterial, in: Circle()).padding(8) } }.buttonStyle(MusicButtonStyle()).disabled(!store.canPlay)
                        Button { store.detailTrack = track } label: { Text(track.title).font(.headline).lineLimit(2) }.buttonStyle(.plain)
                        CatalogTextLink(track: track, store: store).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }.frame(width: 155).contextMenu { TrackActions(track: track, store: store) }
                }
            }.padding(.vertical, 6)
        }.scrollIndicators(.hidden)
    }
}

private struct DiscoveryLoadingRail: View {
    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 20) {
                ForEach(0..<6) { _ in
                    VStack(alignment: .leading, spacing: 12) {
                        RoundedRectangle(cornerRadius: 12).fill(.quaternary).frame(width: 155, height: 155)
                        RoundedRectangle(cornerRadius: 3).fill(.quaternary).frame(width: 120, height: 12)
                        RoundedRectangle(cornerRadius: 3).fill(.quaternary).frame(width: 80, height: 10)
                    }
                }
            }.padding(.vertical, 6)
        }.scrollIndicators(.hidden).accessibilityElement(children: .ignore).accessibilityLabel("正在加载推荐音乐")
    }
}

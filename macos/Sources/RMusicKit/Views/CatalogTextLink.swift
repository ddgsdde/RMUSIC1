import SwiftUI

/// Navigation stays separate from row playback, including compact player captions.
struct CatalogTextLink: View {
    let track: Track
    let store: MusicStore
    var album = false
    @StoredState private var choosing = false
    @StoredState private var loading = false
    @StoredState private var matches: [CatalogReference] = []
    @StoredState private var resolvedCredits: [CatalogReference] = []
    @StoredState private var task: Task<Void, Never>?
    private var names: [String] { album ? [track.album] : resolvedCredits.isEmpty ? track.creditNames : resolvedCredits.map(\.name) }
    var body: some View {
        Group {
            if album && !track.hasAlbum { Text("未归属专辑").foregroundStyle(.secondary) }
            else if names.count > 3 {
                ScrollView(.horizontal) { captions.fixedSize(horizontal: true, vertical: false) }
                    .scrollIndicators(.hidden).fixedSize(horizontal: false, vertical: true)
            } else { captions }
        }
            .popover(isPresented: $choosing) {
                VStack(alignment: .leading, spacing: 6) {
                    if matches.isEmpty { ForEach(Array(names.enumerated()), id: \.offset) { index, name in
                        Button(name) { choosing = false; open(index) }.buttonStyle(.plain).padding(8)
                    } } else { ForEach(matches) { item in Button("\(item.name) · \(item.detail)") { choosing = false; task = Task { await store.openCatalogReference(item, track: track, album: album) } }.buttonStyle(.plain).padding(8) } }
                }.padding(8)
            }
            .opacity(loading ? 0.6 : 1)
            .task(id: "\(store.sessionID):\(track.id):\(track.artist)") {
                resolvedCredits = []
                if !album && track.creditNames.count == 1 && track.artist.contains(" / ") {
                    if let refs = try? await store.catalogReferences(track, album: false, artistIndex: 0), !Task.isCancelled, refs.count > 1, Set(refs.map(\.name)).count == refs.count { resolvedCredits = refs }
                }
            }
            .onDisappear { task?.cancel(); task = nil }
    }
    private var captions: some View {
        HStack(spacing: 0) {
            ForEach(Array(names.enumerated()), id: \.offset) { index, name in
                if index > 0 { Text(" / ").foregroundStyle(.secondary) }
                CatalogCaption(name: name, album: album) { open(index) }
            }
        }
    }
    private func open(_ index: Int) {
        guard !loading, names.first?.isEmpty == false else { return }
        task = Task {
            loading = true; defer { loading = false }
            let owner = store.sessionID
            await store.perform {
                let refs = resolvedCredits.indices.contains(index) ? [resolvedCredits[index]] : try await store.catalogReferences(track, album: album, artistIndex: index)
                try Task.checkCancellation(); guard owner == store.sessionID else { return }
                if refs.count == 1 { await store.openCatalogReference(refs[0], track: track, album: album) }
                else if refs.count > 1 { if !album && Set(refs.map(\.name)).count == refs.count { resolvedCredits = refs } else { matches = refs; choosing = true } }
                else { throw RuntimeFailure(message: "没有可确认身份的曲库资料，请查看歌曲关联目录。") }
            }
        }
    }
}
private struct CatalogCaption: View {
    let name: String
    let album: Bool
    let action: () -> Void
    @StoredState private var hovering = false
    var body: some View {
        Text(name).foregroundStyle(hovering ? MusicStyle.accent : Color.secondary)
            .padding(.horizontal, 3).padding(.vertical, 3).frame(minHeight: 28)
            .background(hovering ? MusicStyle.accent.opacity(0.08) : Color.clear, in: RoundedRectangle(cornerRadius: 5))
            .contentShape(Rectangle())
            .highPriorityGesture(TapGesture().onEnded(action))
            .onHover { hovering = $0 }
            .accessibilityAddTraits(.isButton).accessibilityAction { action() }
            .help("打开\(album ? "专辑" : "歌手") \(name)")
    }
}
extension Track {
    var hasAlbum: Bool {
        let name = album.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let data = provider_data ?? .null
        return !name.isEmpty && (!["none","null","unknown","unknown album","未知专辑","未提供专辑信息"].contains(name) || !data["album_mid"].text.isEmpty || !data["release_id"].text.isEmpty)
    }
    var creditNames: [String] {
        let data = provider_data ?? .null
        let rows = data["artist_credits"].array
        if !rows.isEmpty && rows.count <= 64 && rows.allSatisfy({ !($0["name"].string ?? "").trimmingCharacters(in: .whitespaces).isEmpty }) { return rows.map { $0["name"].text } }
        let provider = source == "local" ? data["cached_from"]["source"].string ?? source : source
        for key in ["provider_metadata", "qqmusic_metadata"] {
            let metadata = data[key], people = data[key]["artists"].array
            if metadata["matched"].boolean, metadata["source"].string == provider, metadata["track"]["artist"].string == artist,
               !people.isEmpty, people.count <= 64, people.allSatisfy({ !($0["id"].string ?? $0["mid"].string ?? "").isEmpty && !$0["name"].text.isEmpty }) {
                let names = people.map { $0["name"].text }
                if names.joined(separator: " / ") == artist { return names }
            }
        }
        let names = artist.components(separatedBy: " / "), ids = data["artist_mids"].array
        if (source == "qqmusic" || data["cached_from"]["source"].string == "qqmusic"), ids.count == names.count, ids.count <= 64, ids.allSatisfy({ !($0.string ?? "").isEmpty }) { return names }
        return [artist]
    }
}

func catalogSummary(_ item: JSONValue) -> Track {
    Track(id: item["track_id"].string ?? item["id"].text, title: item["title"].text, artist: item["artist"].text, album: item["album"].string ?? item["title"].text, duration: 0, format: "", sample_rate: 0, bit_depth: 0, album_artist: item["album_artist"].string, source: item["source"].string, provider_data: item["provider_data"])
}

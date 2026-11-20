import SwiftUI

struct ProviderLibraryView: View {
    let store: MusicStore
    let providerID: String
    var playlist: JSONValue?
    var close: (() -> Void)?
    @StoredState private var resource = ResourceStore()
    @StoredState private var selected: JSONValue?
    @StoredState private var page = 1
    @StoredState private var refresh = 0
    @StoredState private var collection = "all"
    private struct BulkRequest: Equatable { let id = UUID(); let kind: String }
    @StoredState private var action: BulkRequest?
    @StoredState private var progress = ""
    private var pagination: ProviderPagination? { try? ProviderPagination(page: page, response: resource.value) }
    private var active: JSONValue? { selected ?? playlist }
    private var path: String {
        let suffix = active.map { "/\($0["id"].text.pathComponentEncoded)/tracks" } ?? ""
        return apiQuery("/providers/\(providerID.pathComponentEncoded)/playlists" + suffix, ["page": String(page), "collection": collection])
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if close != nil || selected != nil { Button("返回", systemImage: "chevron.left") { if selected != nil { selected = nil; page = 1 } else { close?() } }.buttonStyle(MusicControlStyle()) }
                FeatureHeader(title: active?["name"].text ?? "在线歌单", loading: resource.loading) { refresh += 1 }
                ResourceNotice(resource: resource)
                if active == nil { ThemePicker("云端歌单分类", selection: $collection) { Text("全部").tag("all"); Text("自建歌单").tag("created"); Text("收藏歌单").tag("collected") }.pickerStyle(.segmented).frame(maxWidth: 360) }
                if let active {
                    HStack {
                        Button("播放全部", systemImage: "play.fill") { action = BulkRequest(kind: "play") }.disabled(!store.canPlay || action != nil)
                        Button("全部入队", systemImage: "text.badge.plus") { action = BulkRequest(kind: "append") }.disabled(!store.canPlay || action != nil)
                        if action != nil { ProgressView().controlSize(.small); Text(progress).font(.caption); Button("取消") { action = nil } }
                        Spacer()
                    }
                    paginationControls
                    HStack {
                        Text("共 \(resource.value["total"].integer) 首 · 本页 \(tracks.count) 首").foregroundStyle(.secondary)
                        Spacer()
                        Button("导入并绑定歌单", systemImage: "arrow.down.circle") { Task { await bind(active) } }
                    }
                    SelectableTrackList(tracks: tracks, store: store, startIndex: (page - 1) * 100)
                } else {
                    ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
                        Button { selected = item; page = 1 } label: { HStack { Image(systemName: "music.note.list"); Text(item["name"].text); Spacer(); Text("\(item["track_count"].integer) 首").foregroundStyle(.secondary); Image(systemName: "chevron.right") }.padding(18) }.buttonStyle(.plain)
                    }
                }
                if !resource.loading && resource.error == nil && resource.value["items"].array.isEmpty { ContentUnavailableView("此页没有内容", systemImage: "music.note.list") }
                paginationControls
            }.padding(30)
        }.onChange(of: collection) { page = 1 }
        .task(id: "\(store.sessionID):\(path):\(refresh)") { await resource.load(path, store: store) }
        .task(id: action?.id) { if let action, let active { await bulk(action, playlist: active["id"].text) } }
        .onChange(of: store.sessionID) { action = nil; selected = nil; page = 1 }
    }
    private var paginationControls: some View {
        HStack { Button("上一页") { page -= 1 }.disabled(page <= 1 || resource.loading); Spacer(); Text("第 \(page) 页"); Spacer(); Button("下一页") { if let next = pagination?.next { page = next } }.disabled(pagination?.next == nil || resource.loading) }
    }
    private var tracks: [Track] { (try? resource.value["items"].decode([Track].self)) ?? [] }
    private func bulk(_ request: BulkRequest, playlist: String) async {
        defer { if action?.id == request.id { action = nil; progress = "" } }
        let action = request.kind
        await store.perform {
            let session = store.sessionID
            guard let zone = store.zone else { throw RuntimeFailure(message: "先选择播放音区。") }
            try await store.requireUngrouped(zone.id)
            let ids = try await collectProviderPlaylistIDs(providerID, playlist: playlist, store: store) { count, total in if self.action?.id == request.id { progress = "已读取 \(count) / \(total) 首" } }
            try Task.checkCancellation()
            guard session == store.sessionID, store.selectedZoneID == zone.id else { throw CancellationError() }
            guard !ids.isEmpty else { throw RuntimeFailure(message: "歌单中还没有歌曲。") }
            _ = try await store.api("/zones/\(zone.id.pathComponentEncoded)/queue", method: action == "play" ? "PUT" : "POST", body: .object(["track_ids": .array(ids)]))
            try Task.checkCancellation()
            guard session == store.sessionID, store.selectedZoneID == zone.id else { throw CancellationError() }
            if action == "play" { _ = try await store.api("/zones/\(zone.id.pathComponentEncoded)/play", method: "POST", body: .object(["queue_index": .number(0)])) }
            try await store.refreshZones()
        }
    }
    private func bind(_ playlist: JSONValue) async {
        await store.perform {
            _ = try await store.api("/playlists/bind", method: "POST", body: .object(["provider": .string(providerID), "playlist_id": playlist["id"], "auto_sync": .bool(true)]))
            try await store.collection.refresh(store)
            store.notice = "歌单已绑定，后台同步进度可在任务中查看。"
        }
    }
}

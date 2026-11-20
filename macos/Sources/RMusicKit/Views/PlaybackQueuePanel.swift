import SwiftUI

struct PlaybackQueuePanel: View {
    let store: MusicStore
    let close: () -> Void
    @StoredState private var order = "default"
    @StoredState private var radio = [Track]()
    @StoredState private var radioOpen = false
    @StoredState private var radioStatus = ""
    var body: some View {
        let queue = store.zone?.queue ?? []
        let ids = queue.map(\.id)
        let scope = "queue:\(store.sessionID):\(store.zone?.id ?? ""):\(store.zone?.playback_id ?? "")"
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text("播放队列（\(store.zone?.queue.count ?? 0)）").font(.headline); Spacer(); Button("清空") { Task { await clear() } }.disabled(store.zone?.queue.isEmpty != false); Button("关闭队列", systemImage: "xmark", action: close).labelStyle(.iconOnly).buttonStyle(MusicControlStyle()) }
            ThemePicker("音区", selection: Binding(get: { store.selectedZoneID }, set: { store.selectPlaybackZone($0) })) { ForEach(store.playbackZones) { Text(store.playbackOutputLabel($0)).tag($0.id) } }
            CatalogSortPicker(title: "显示排序", value: $order, options: trackSortOptions)
            if let zone = store.zone {
                VStack(alignment: .leading, spacing: 6) {
                    Toggle(isOn: Binding(get: { zone.autoplay?["enabled"].boolean ?? true }, set: { enabled in
                        Task { await store.perform {
                            var config = zone.autoplay?.object ?? ["source": .string("mixed"), "fallback_local": .bool(true)]
                            config["enabled"] = .bool(enabled)
                            _ = try await store.api("/zones/\(zone.id.pathComponentEncoded)", method: "PATCH", body: .object(["autoplay": .object(config)]))
                            try await store.refreshZones()
                        }}
                    })) {
                        Label("队列电台", systemImage: "dot.radiowaves.left.and.right")
                    }.toggleStyle(.switch).controlSize(.small)
                    if zone.autoplay?["enabled"].boolean != false {
                        DisclosureGroup("续播推荐", isExpanded: $radioOpen) {
                            if let next = radio.first {
                                HStack {
                                    VStack(alignment: .leading, spacing: 3) {
                                        Text(next.title).lineLimit(1)
                                        Text(next.source == "local" ? "本地 · 延续队列风格" : "云端 · 延续队列风格").font(.caption).foregroundStyle(.secondary)
                                    }
                                    Spacer()
                                    Button("跳过") { Task { await loadRadio(zone, skip: next.id) } }
                                }
                            } else {
                                Text(radioStatus == "waiting" || radioStatus == "unavailable" ? "暂无可播候选，电台会重试。" : radioStatus == "preparing" ? "正在准备下一批歌曲…" : "接近队列末尾时准备相似歌曲。")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }.task(id: "\(store.sessionID):\(radioOpen):\(zone.id):\(zone.playback_id ?? ""):\(zone.autoplay?["enabled"].boolean ?? true):\(zone.queue.count):\(store.radioRevision)") {
                    if radioOpen, zone.autoplay?["enabled"].boolean != false { await loadRadio(zone) } else { radio = []; radioStatus = "" }
                }
            }
            Divider()
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 4) {
                        ForEach(trackOrder(queue, by: order), id: \.self) { index in
                            let track = queue[index]
                            HStack(spacing: 2) {
                            if order == "default" { TrackOrderHandle(scope: scope, ids: ids, index: index) }
                            Button { Task { await store.transport("play", body: ["queue_index": .number(Double(index))]) } } label: {
                                HStack(spacing: 10) {
                                    ArtworkView(track: track, store: store, radius: 7).frame(width: 40, height: 40)
                                    VStack(alignment: .leading, spacing: 5) { Text(track.title).font(.system(size: 12, weight: .medium)).lineLimit(1); CatalogTextLink(track: track, store: store).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1); CatalogTextLink(track: track, store: store, album: true).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1) }
                                    Spacer(minLength: 0)
                                    if store.zone?.queue_index == index { Image(systemName: store.zone?.status == "playing" ? "waveform" : "pause.fill").foregroundStyle(MusicStyle.accent) }
                                    else { Text(timeLabel(track.duration)).font(.system(size: 9).monospacedDigit()).foregroundStyle(.secondary) }
                                }.padding(9).background(store.zone?.queue_index == index ? MusicStyle.accent.opacity(0.1) : .clear, in: RoundedRectangle(cornerRadius: 10))
                            }.buttonStyle(.plain).disabled(!store.canPlay)
                            }.contextMenu { TrackActions(track: track, store: store) }.modifier(TrackOrderDrop(scope: scope, ids: ids, index: index, enabled: order == "default") { from, to in
                                if let original = store.zone { Task { await store.reorderQueue(original, from: from, to: to) } }
                            }).id(index)
                        }
                    }
                    if store.zone?.queue.isEmpty != false { ContentUnavailableView("队列为空", systemImage: "music.note.list") }
                }.task(id: store.zone?.id) { if let index = store.zone?.queue_index { proxy.scrollTo(index, anchor: .center) } }
            }
        }.padding(18)
    }
    private func loadRadio(_ zone: Zone, skip: String? = nil) async {
        let owner = store.sessionID
        let path = "/zones/\(zone.id.pathComponentEncoded)/radio"
        let value = try? await store.api(path, method: skip == nil ? "GET" : "POST", body: skip.map { .object(["track_id": .string($0)]) })
        guard !Task.isCancelled, store.sessionID == owner, store.zone?.id == zone.id else { return }
        radio = (try? value?["items"].decode([Track].self)) ?? []
        radioStatus = value?["status"].text ?? ""
    }
    private func clear() async { await store.perform { guard let zone = store.zone else { return }; try await store.requireUngrouped(zone.id); _ = try await store.api("/zones/\(zone.id.pathComponentEncoded)/queue", method: "DELETE"); try await store.refreshZones() } }
}

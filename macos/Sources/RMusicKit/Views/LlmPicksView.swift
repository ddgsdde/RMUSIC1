import SwiftUI

struct LlmPicksView: View {
    let store: MusicStore
    let candidates: [Track]
    @StoredState private var open = false
    @StoredState private var preference = ""
    @StoredState private var playlistName = "LLM 精选"
    @StoredState private var result: JSONValue = .null
    @StoredState private var action = WorkflowAction()
    @StoredState private var saving = WorkflowAction()
    var body: some View {
        FeatureCard {
            DisclosureGroup("LLM 帮我选歌", isExpanded: $open) {
                VStack(alignment: .leading, spacing: 14) {
                    Text("仅主动请求时发送候选歌曲资料与偏好。服务由 Core 管理员配置。").font(.caption).foregroundStyle(.secondary)
                    TextField("这次想听什么？", text: $preference)
                    HStack {
                        Text("从当前推荐中选择，最多 50 首候选").font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button("生成推荐", systemImage: "sparkles") {
                            var seen = Set<String>()
                            let ids = candidates.filter { seen.insert($0.id).inserted && $0.available != false }.prefix(50).map { JSONValue.string($0.id) }
                            action.send("/llm/recommend", body: .object(["track_ids": .array(ids), "preference": .string(preference)]), store: store) { value in result = value; action.notice = nil }
                        }.disabled(action.busy || candidates.isEmpty || preference.utf8.count > 1000)
                    }
                    WorkflowNotice(action: action)
                    if !result["items"].array.isEmpty {
                        ForEach(Array(result["items"].array.enumerated()), id: \.offset) { _, value in
                            if let track = try? value.decode(Track.self) {
                                HStack {
                                    VStack(alignment: .leading, spacing: 4) { Text(track.title); CatalogTextLink(track: track, store: store).font(.caption).foregroundStyle(.secondary); if let reason = result["reasons"][track.id].string { Text(reason).font(.caption).foregroundStyle(.secondary).textSelection(.enabled) } }
                                    Spacer()
                                    Menu("操作") { TrackActions(track: track, store: store) }
                                    Button("播放", systemImage: "play.fill") { Task { await store.playList((try? result["items"].decode([Track].self)) ?? [track], starting: track.id) } }.disabled(!store.canPlay)
                                }
                            }
                        }
                        HStack {
                            TextField("保存为歌单", text: $playlistName)
                            Button("保存歌单") {
                                saving.send("/playlists", body: .object(["name": .string(playlistName), "track_ids": .array(result["items"].array.map { $0["id"] })]), store: store) { _ in store.notice = "推荐已保存，可在播放列表中查看。" }
                            }.disabled(saving.busy || playlistName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        }
                        WorkflowNotice(action: saving)
                    }
                }.padding(.top, 14)
            }
        }.onDisappear { action.cancel(); saving.cancel() }
    }
}

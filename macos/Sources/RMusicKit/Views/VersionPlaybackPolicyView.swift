import SwiftUI

struct VersionPlaybackPolicyView: View {
    let track: Track
    let store: MusicStore
    let preferred: Track?
    @StoredState private var resource = ResourceStore()
    @StoredState private var action = WorkflowAction()
    @StoredState private var usePreferred = false
    @StoredState private var fallback = false
    @StoredState private var sameRelease = true
    @StoredState private var lossless = true
    @StoredState private var clearPreferred = false
    @StoredState private var refresh = UUID()
    private var path: String { "/library/tracks/\(track.id.pathComponentEncoded)/versions/policy" }
    var body: some View {
        DisclosureGroup("播放版本策略") {
            VStack(alignment: .leading, spacing: 12) {
                ResourceNotice(resource: resource); WorkflowNotice(action: action)
                if resource.value["identity_resolved"].boolean {
                    Text("播放首选 · \(clearPreferred ? "按所选歌曲播放" : preferred?.title ?? resource.value["preferred_track"]["title"].string ?? "按所选歌曲播放")\(preferred == nil ? "" : " · 待保存")").font(.callout)
                    Toggle("新点播使用播放首选", isOn: $usePreferred)
                    Toggle("原版本不可用时自动回退", isOn: $fallback)
                    Toggle("回退仅限相同发行", isOn: $sameRelease)
                    Toggle("回退版本必须无损且不低于原音质", isOn: $lossless)
                    Text("自动回退默认关闭，仅选已确认的同一录音；先尝试恢复原音源。已有播放和队列不会自动换版。").font(.caption).foregroundStyle(.secondary)
                    Button("清除播放首选") { clearPreferred = true; usePreferred = false }
                    Button("保存播放策略") {
                        action.send(path, method: "PUT", body: .object(["expected_revision": resource.value["revision"], "policy": .object(["preferred_track_id": clearPreferred ? .null : preferred.map { .string($0.id) } ?? resource.value["policy"]["preferred_track_id"], "apply_to_new_playback": .bool(usePreferred), "automatic_fallback": .bool(fallback), "same_release": .bool(sameRelease), "lossless_only": .bool(lossless)])]), store: store) { value in refresh = UUID(); action.notice = "策略已保存，仅下次点播生效" }
                    }.disabled(action.busy || resource.loading)
                } else if !resource.loading { Text("请先唯一确认录音身份，再设置长期策略。").foregroundStyle(.secondary) }
                Button("重新读取策略") { refresh = UUID() }
            }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 12).toggleStyle(.checkbox)
        }.task(id: "\(store.sessionID):\(track.id):\(refresh)") {
            if await resource.load(path, store: store) { let p = resource.value["policy"]; usePreferred = p["apply_to_new_playback"].boolean; fallback = p["automatic_fallback"].boolean; sameRelease = p["same_release"].boolean; lossless = p["lossless_only"].boolean }
        }.onChange(of: preferred?.id) { clearPreferred = false }.onDisappear { action.cancel() }
    }
}

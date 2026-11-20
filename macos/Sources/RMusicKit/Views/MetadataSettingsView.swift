import SwiftUI

struct MetadataSettingsView: View {
    let store: MusicStore
    @StoredState private var enabled = false
    @StoredState private var action = WorkflowAction()
    var body: some View {
        ScrollView {
            FeatureCard {
                Text("资料与封面").font(.headline)
                Toggle("匿名直连 QQ 音乐公开资料", isOn: $enabled)
                    .disabled(action.busy)
                    .onChange(of: enabled) { value in
                        guard value != store.anonymousMetadata else { return }
                        action.run(store: store, operation: {
                            if store.connected { _ = try await store.runtimeCall("metadata_policy", .object(["enabled": .bool(value)])) }
                            UserDefaults.standard.set(value, forKey: "anonymousQQMetadata")
                            store.anonymousMetadata = value
                            store.artwork.reset(revalidate: true)
                            return .null
                        }, commit: { _ in action.notice = "已保存" })
                    }
                Text("封面与歌手、专辑资料由这台 Mac 匿名获取，成功后仍回填 Core 缓存；已有图片不重复上传，失败时自动通过 Core 获取。手动编辑的资料优先。").font(.caption).foregroundStyle(.secondary)
                Text("仅影响公开资料；歌词、曲目目录和歌曲播放继续通过 Core。无需 QQ 登录信息。").font(.caption).foregroundStyle(.secondary)
                WorkflowNotice(action: action)
            }.padding(24)
        }.task { enabled = store.anonymousMetadata }
        .onChange(of: action.error) { error in if error != nil { enabled = store.anonymousMetadata } }
        .onDisappear { action.cancel() }
    }
}

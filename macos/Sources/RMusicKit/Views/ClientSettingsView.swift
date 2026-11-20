import SwiftUI

struct ClientSettingsView: View {
    let store: MusicStore
    @StoredState private var tab = "appearance"
    @StoredState private var limit = 5.0
    @AppStorage("playerLowPerformance") private var lowPerformance = false
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            SettingsCategories(title: "本机设置分类", items: SettingsCategory.client, selection: $tab).padding(.horizontal, 24).padding(.top, 20)
            if tab == "outputs" { OutputsView(store: store) }
            else if tab == "streaming" { StreamingSettingsView(store: store) }
            else if tab == "metadata" { MetadataSettingsView(store: store) }
            else {
                ScrollView {
                    if tab == "appearance" { VStack(alignment: .leading, spacing: 20) { AppearanceView(); Toggle("减少播放页动画", isOn: $lowPerformance) }.padding(24) }
                    else {
                        FeatureCard {
                            Text("本机下载与播放缓存").font(.headline)
                            Fact(name: "自动缓存", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: store.downloads.used), countStyle: .file))
                            Fact(name: "手动保留", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: store.downloads.manualBytes), countStyle: .file))
                            TextField("容量上限 · GiB", value: $limit, format: .number).frame(maxWidth: 360)
                            Button("保存上限") { Task { await store.perform { try await store.downloads.setLimit(UInt64((limit * 1024 * 1024 * 1024).rounded()), store: store) } } }.disabled(!store.connected || !limit.isFinite || !(0.1...1000).contains(limit))

                        }.padding(24)
                    }
                }
            }
        }.task { limit = Double(store.downloads.limit) / (1024 * 1024 * 1024) }
    }
}

public struct ClientSettingsViewHost: View {
    private let store: MusicStore
    public init(store: MusicStore) { self.store = store }
    public var body: some View { ClientSettingsView(store: store).frame(minWidth: 680, minHeight: 520) }
}

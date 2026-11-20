import SwiftUI

struct CoreSettingsView: View {
    let store: MusicStore
    @StoredState private var tab = "resources"
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                SettingsCategories(title: "Core 设置分类", items: store.user["role"].text == "external" ? SettingsCategory.core.filter { $0.id == "account" || $0.id == "playback" } : SettingsCategory.core, selection: $tab).onAppear { if store.user["role"].text == "external" && !["account", "playback"].contains(tab) { tab = "account" } }
                Group {
                if !store.connected { Text("请先连接 Core。") }
                else if tab == "account" { AccountSettingsView(store: store) }
                else if tab == "jobs" { BackgroundJobsView(store: store) }
                else if store.user["role"].text != "admin" { Text("修改 Core 设置需要管理员权限。后台任务可供有读取权限的账号查看。") }
                else if tab == "statistics" { statisticsForm }
                else if tab == "network" { RemoteAccessSettingsView(store: store) }
                else if tab == "library" { LibraryMaintenanceView(store: store) }
                else if tab == "backups" { BackupSettingsView(store: store) }
                else if tab == "plugins" { PluginToolsView(store: store) }
                else if tab == "artwork" { ArtworkMaintenanceView(store: store) }
                else if tab == "platforms" { ConfigurationForm(store: store, title: "后台资料刮削", subtitle: "默认关闭、单并发；尊重平台限流，不下载音频，更新未变化时保留已有资料。", readPath: "/settings", savePath: "/settings", fields: CoreConfiguration.catalogScrape); MetadataPriorityView(store: store); MusicPlatformsView(store: store) }
                else if tab == "llm" { LlmSettingsView(store: store) }
                else if tab == "resources" {
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .top, spacing: 18) {
                            resourceForm.frame(minWidth: 420)
                            VStack(spacing: 18) { CoreResourcesView(store: store); CacheRepairView(store: store) }.frame(width: 300)
                        }
                        VStack(spacing: 18) { resourceForm; CoreResourcesView(store: store); CacheRepairView(store: store) }
                    }
                    CoreStorageView(store: store)
                } else if tab == "requests" {
                    ConfigurationForm(store: store, title: "音源请求限制", readPath: "/settings", savePath: "/settings", fields: CoreConfiguration.requests)
                } else {
                    ConfigurationForm(store: store, title: "Core 通用设置", readPath: "/settings", savePath: "/settings", fields: CoreConfiguration.general)
                }
                }.id("\(store.sessionID):\(tab)")
            }.padding(24)
        }
    }
    private var statisticsForm: some View { ConfigurationForm(store: store, title: "聆听统计", subtitle: "统一设置连接此核心的客户端如何划分日期。默认上海，不改写已保存的聆听时长。", readPath: "/settings", savePath: "/settings", fields: CoreConfiguration.statistics) }
    private var resourceForm: some View {
        ConfigurationForm(store: store, title: "缓存与资源", readPath: "/settings", savePath: "/settings", fields: CoreConfiguration.resources)
    }
}
private struct CoreResourcesView: View {
    let store: MusicStore
    @StoredState private var resource = ResourceStore()
    @StoredState private var refreshID = UUID()
    var body: some View {
        FeatureCard {
            FeatureHeader(title: "资源使用", loading: resource.loading) { refreshID = UUID() }
            ResourceNotice(resource: resource)
            let data = resource.value["resources"]
            Fact(name: "受控使用", value: "\(bytes(data["used_bytes"])) / \(bytes(data["budget_bytes"]))")
            Fact(name: "播放预留", value: bytes(data["playback_reserved_bytes"]))
            Fact(name: "Core 进程", value: bytes(data["process_rss_bytes"]))
            Fact(name: "运行任务", value: String(data["active_tasks"].integer))
        }.task(id: "\(store.sessionID):\(refreshID)") { await refresh() }
    }
    private func bytes(_ value: JSONValue) -> String { if case .null = value { return "未报告" }; if value.number == 0 { return "0 B" }; return ByteCountFormatter.string(fromByteCount: Int64(max(0, min(value.number, Double(Int64.max) / 2))), countStyle: .memory) }
    private func refresh() async { await resource.load("/settings", store: store) }
}

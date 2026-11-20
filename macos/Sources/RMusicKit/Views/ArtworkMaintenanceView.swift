import SwiftUI

struct ArtworkMaintenanceView: View {
    let store: MusicStore
    @StoredState private var resource = ResourceStore()
    @StoredState private var action = WorkflowAction()
    @StoredState private var selected: Set<String> = []
    @StoredState private var offset = 0
    @StoredState private var failedOnly = false
    @StoredState private var revision = UUID()
    private var pageIDs: Set<String> { Set(resource.value["items"].array.compactMap { $0["id"].string }.filter { !$0.isEmpty }) }
    private var allSelected: Bool { !pageIDs.isEmpty && pageIDs.isSubset(of: selected) }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            ConfigurationForm(store: store, title: "永久图片预算", subtitle: "独立于音频缓存；保留已有基图与人工图片，预算不足时停止新增。", readPath: "/settings", savePath: "/settings", fields: [.init(key: "artwork_limit_bytes", title: "图片预算 · MiB", kind: .number(1...1_099_511_627_776), divisor: 1_048_576)])
            FeatureHeader(title: "图片维护", loading: resource.loading) { revision = UUID() }
            ResourceNotice(resource: resource); WorkflowNotice(action: action)
            FeatureCard {
                HStack { Fact(name: "图片占用", value: bytes(resource.value["used_bytes"])); Spacer(); Fact(name: "文件数量", value: String(resource.value["file_count"].integer)) }
                HStack { Fact(name: "本次发送正文", value: bytes(resource.value["metrics"]["response_bytes"])); Spacer(); Fact(name: "本次上游正文", value: bytes(resource.value["metrics"]["upstream_bytes"])) }
                Fact(name: "内容未变响应", value: String(resource.value["metrics"]["not_modified"].integer))
            }
            HStack {
                Toggle("仅显示异常", isOn: $failedOnly).toggleStyle(.switch)
                Button(allSelected ? "取消全选" : "全选本页") { selected = allSelected ? [] : pageIDs }.disabled(resource.loading || pageIDs.isEmpty).help("选择当前分页与筛选范围内的图片，每页最多50张")
                Spacer()
                Button("检查并标记") { submit("/artwork-cache/check", body: .object([:])) }.help("只检查文件，不下载图片")
                Button("刷新所选 · \(selected.count)") { submit("/artwork-cache/refresh", body: .object(["ids": .array(selected.sorted().map(JSONValue.string))])) }.disabled(selected.isEmpty || resource.loading)
            }.disabled(action.busy || resource.loading)
            LazyVStack(spacing: 8) {
                ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
                    HStack(spacing: 12) {
                        Toggle(item["name"].text, isOn: Binding(get: { selected.contains(item["id"].text) }, set: { if $0 { selected.insert(item["id"].text) } else { selected.remove(item["id"].text) } })).toggleStyle(.checkbox).lineLimit(1).disabled(action.busy || resource.loading)
                        Spacer()
                        Text(item["host"].text).font(.caption).foregroundStyle(.secondary)
                        Text(state(item["state"].text)).font(.caption)
                        if !item["error"].text.isEmpty { Text(failure(item["error"].text)).font(.caption2).foregroundStyle(.orange).lineLimit(1) }
                        Button("刷新") { submit("/artwork-cache/refresh", body: .object(["ids": .array([item["id"]])])) }.disabled(action.busy || resource.loading)
                    }.padding(12).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 12))
                }
            }
            if !resource.loading && resource.value["items"].array.isEmpty {
                Text(failedOnly ? "没有异常图片" : "暂无图片记录").font(.callout).foregroundStyle(.secondary).frame(maxWidth: .infinity).padding(.vertical, 20)
            }
            OffsetPageControls(offset: $offset, total: resource.value["total"].integer, limit: 50, disabled: resource.loading)
            ArtworkFailureHistoryView(store: store, revision: revision)
            ForEach(Array(resource.value["jobs"].array.enumerated()), id: \.offset) { _, job in
                Text("\(JobPresentation.title(job["kind"].text)) · \(JobPresentation.status(job["status"].text)) · \(job["processed"].integer)").font(.caption).foregroundStyle(.secondary)
            }
        }.task(id: "\(store.sessionID):\(offset):\(failedOnly):\(revision)") {
            selected = []
            await resource.load(apiQuery("/artwork-cache", ["offset": String(offset), "limit": "50", "failed": String(failedOnly)]), store: store)
        }.onChange(of: failedOnly) { offset = 0 }.onDisappear { action.cancel() }
    }
    private func submit(_ path: String, body: JSONValue) { action.send(path, method: "POST", body: body, store: store) { _ in action.notice = "任务已加入后台，可刷新查看进度"; revision = UUID() } }
    private func failure(_ key: String) -> String { ["artwork_source_expired": "平台地址已失效", "artwork_rate_limited": "平台限流，等待重试", "artwork_budget_exceeded": "图片预算不足", "artwork_corrupt": "图片文件损坏", "artwork_storage_unavailable": "图片存储不可用", "not_found": "文件缺失"][key] ?? "暂时无法加载，可重试" }
    private func bytes(_ value: JSONValue) -> String { if value.number == 0 { return "0 B" }; return ByteCountFormatter.string(fromByteCount: Int64(value.number), countStyle: .file) }
    private func state(_ key: String) -> String { ["ready": "可用", "stored": "已保存", "unavailable": "平台未提供图片", "missing": "文件缺失", "corrupt": "文件损坏", "failed": "加载失败", "not_requested": "尚未请求"][key] ?? key }
}

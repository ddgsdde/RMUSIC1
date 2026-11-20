import SwiftUI

struct BackgroundJobsView: View {
    let store: MusicStore
    @StoredState private var category = "active"
    @StoredState private var page = 0
    @StoredState private var expanded = true
    @StoredState private var resource = ResourceStore()
    @StoredState private var refreshID = UUID()
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            FeatureHeader(title: "后台任务", loading: resource.loading) { refreshID = UUID() }
            ThemePicker("任务分类", selection: $category) { Text("进行中").tag("active"); Text("历史记录").tag("history") }.pickerStyle(.segmented)
            ResourceNotice(resource: resource)
            DisclosureGroup("\(category == "active" ? "进行中" : "历史记录") · \(resource.value["total"].integer)", isExpanded: $expanded) {
            if resource.value["items"].array.isEmpty { Text("暂无后台任务").foregroundStyle(.secondary).padding(.vertical, 30) }
            LazyVStack(spacing: 14) {
                ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, job in
                    FeatureCard {
                        HStack { Text(JobPresentation.title(job["kind"].text)).font(.headline); Spacer(); Text(JobPresentation.status(job["status"].text)).foregroundStyle(.secondary) }
                        if !job["target"].text.isEmpty { Text(job["target"].text).font(.caption).lineLimit(2).textSelection(.enabled) }
                        Text(job["phase"].text).font(.callout)
                        if job["total"].number > 0 { ProgressView(value: min(job["processed"].number, job["total"].number), total: job["total"].number) }
                        Text("\(job["processed"].integer) / \(job["total"].integer) · \(displayDate(job["updated_at"].number))").font(.caption).foregroundStyle(.secondary)
                        if !job["error"].text.isEmpty { Text(job["error"].text).foregroundStyle(.orange).textSelection(.enabled) }
                        JobActionButtons(store: store, job: job, finished: { await refresh() })
                    }
                }
            }
            HStack { Button("上一页") { page -= 1 }.disabled(page == 0); Spacer(); Text("\(page + 1) / \(max(1, (resource.value["total"].integer + 4) / 5))").font(.caption); Spacer(); Button("下一页") { page += 1 }.disabled((page + 1) * 5 >= resource.value["total"].integer) }
            }
        }.onChange(of: category) { page = 0 }.task(id: "\(store.sessionID):\(category):\(page):\(expanded):\(refreshID)") {
            guard expanded else { return }
            repeat { await refresh(); do { try await Task.sleep(for: .seconds(5)) } catch { return } } while !Task.isCancelled
        }
    }
    private func refresh() async { await resource.load("/jobs?status=\(category)&limit=5&offset=\(page * 5)", store: store) }
}
struct JobActionButtons: View {
    let store: MusicStore
    let job: JSONValue
    var adminOnly = false
    let finished: () async -> Void
    @StoredState private var working = false
    var body: some View {
        HStack {
            if !JobPresentation.ended(job["status"].text) {
                Button(job["status"].text == "paused" ? "继续" : "暂停") { act(job["status"].text == "paused" ? "running" : "paused") }
                Button("取消任务", role: .destructive) { act("cancelled") }
                if job["kind"].text != "cache_metadata" { Button("优先处理") { act("priority") } }
            } else if job["status"].text != "completed" { Button("重试") { act("retry") } }
        }.disabled(working || !store.coreReachable || (adminOnly && store.user["role"].text != "admin"))
    }
    private func act(_ status: String) {
        working = true
        Task {
            defer { working = false }
            await store.perform { _ = try await store.api("/jobs/\(job["id"].text.pathComponentEncoded)", method: "PATCH", body: .object(["status": .string(status)])); await finished() }
        }
    }
}

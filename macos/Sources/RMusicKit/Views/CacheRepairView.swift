import SwiftUI

struct CacheRepairView: View {
    let store: MusicStore
    @StoredState private var resource = ResourceStore()
    @StoredState private var refreshID = UUID()
    @StoredState private var concurrency = 2
    @StoredState private var working = false
    @StoredState private var confirm = false
    private var job: JSONValue { resource.value["job"] }
    private var active: Bool { !job["id"].text.isEmpty && !JobPresentation.ended(job["status"].text) }
    var body: some View {
        FeatureCard {
            FeatureHeader(title: "缺失的云端缓存", loading: resource.loading) { refreshID = UUID() }
            Text("恢复 Core 曾保存、后来丢失的云端音频文件。平时播放会按需拉取，需要批量恢复时再运行。").font(.caption).foregroundStyle(.secondary)
            if resource.status == 404 { Text("此 Core 尚不支持缺失补齐，请升级服务端。本机部署已包含此功能。").font(.callout).foregroundStyle(.secondary) }
            else { ResourceNotice(resource: resource) }
            if resource.status == nil && !resource.loading && !resource.value["enabled"].boolean { Label("当前 Core 未启用缓存，不会下载补齐。", systemImage: "externaldrive.badge.xmark").foregroundStyle(.secondary) }
            VStack(alignment: .leading, spacing: 12) {
                Stepper("补齐并发 \(concurrency)", value: $concurrency, in: 1...16).frame(width: 220)
                if active { Button("应用并发") { Task { await configure() } } }
                else { Button("补齐所有缺失", systemImage: "arrow.down.circle") { confirm = true }.buttonStyle(WorkspacePrimaryButton()).fixedSize(horizontal: true, vertical: false) }
            }.disabled(working || !store.coreReachable || !resource.value["enabled"].boolean || store.user["role"].text != "admin")
            Text("受音源限流保护").font(.caption).foregroundStyle(.secondary).help("补齐受后台并发、歌曲拉取名额、平台间隔和冷却限制，前台播放优先。")
            if !job["id"].text.isEmpty {
                HStack { Text(JobPresentation.status(job["status"].text)).font(.headline); Spacer(); Text("\(job["processed"].integer) / \(job["total"].integer)").monospacedDigit() }
                Text(job["phase"].text).font(.callout).foregroundStyle(.secondary)
                Text("已检查 \(job["options"]["checked"].integer) 项 · 跳过 \(job["options"]["skipped"].integer) 项 · 当前并发 \(job["options"]["concurrency"].integer)").font(.caption)
                if job["total"].number > 0 { ProgressView(value: min(job["processed"].number, job["total"].number), total: job["total"].number) }
                if !job["error"].text.isEmpty { Text(job["error"].text).foregroundStyle(.orange).textSelection(.enabled) }
                JobActionButtons(store: store, job: job, adminOnly: true, finished: { await refresh() })
            }
        }.confirmationDialog("补齐缺失文件会下载歌曲并占用 Core 磁盘空间。", isPresented: $confirm, titleVisibility: .visible) {
            Button("开始补齐") { Task { await start() } }
        }.task(id: "\(store.sessionID):\(refreshID)") {
            repeat {
                await refresh()
                do { try await Task.sleep(for: .seconds(5)) } catch { return }
            } while !Task.isCancelled
        }
    }
    private func refresh() async { await resource.load("/library/cache-repair", store: store) }
    private func start() async { await mutate("/library/cache-repair", "POST") }
    private func configure() async { await mutate("/library/cache-repair/\(job["id"].text.pathComponentEncoded)", "PATCH") }
    private func mutate(_ path: String, _ method: String) async {
        guard !working else { return }; working = true; defer { working = false }
        await store.perform {
            _ = try await store.api(path, method: method, body: .object(["concurrency": .number(Double(concurrency))]))
            await refresh()
        }
    }
}

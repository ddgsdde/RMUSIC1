import SwiftUI

/// Holds a single file page; destructive work is explicitly selected and rechecked by Core.
struct LibraryFilesView: View {
    let store: MusicStore
    let storage: String
    @Environment(\.dismiss) private var dismiss
    @StoredState private var resource = ResourceStore()
    @StoredState private var query = ""
    @StoredState private var sort = "play_count_asc"
    @StoredState private var cleanup = false
    @StoredState private var threshold = 2
    @StoredState private var page = 0
    @StoredState private var selected: Set<String> = []
    @StoredState private var confirming = false
    @StoredState private var working = false
    @StoredState private var job: JSONValue = .null
    private var filter: [String: JSONValue] {
        var result: [String: JSONValue] = ["storage": .string(storage), "q": .string(query), "sort": .string(sort)]
        if cleanup && storage == "cache" { result["unlisted"] = .bool(true); result["max_play_count"] = .number(Double(threshold)) }
        return result
    }
    private var path: String {
        var query = filter.mapValues { value in if case .bool(let flag) = value { return flag ? "true" : "false" }; return value.string ?? String(value.integer) }
        query["limit"] = "50"; query["offset"] = String(page * 50)
        return apiQuery("/library/files", query)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack { Text(storage == "cache" ? "缓存文件管理" : "本地文件管理").font(.title2.bold()); Spacer(); Button("完成") { dismiss() } }
            HStack {
                TextField("搜索文件", text: $query)
                ThemePicker("排序", selection: $sort) { Text("名称").tag("title"); Text("聆听次数升序").tag("play_count_asc"); Text("聆听次数降序").tag("play_count_desc"); Text("文件大小").tag("size_desc") }.frame(width: 240)
            }
            if storage == "cache" {
                HStack {
                    Toggle("专项清理：未收藏且不在任何播放列表", isOn: $cleanup).toggleStyle(.switch).controlSize(.small)
                    if cleanup { Stepper("聆听次数 ≤ \(threshold)", value: $threshold, in: 0...1_000_000) }
                }
            }
            ResourceNotice(resource: resource)
            HStack {
                Text("\(resource.value["total"].integer) 个文件 · 已选 \(selected.count)").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("选择本页") { selected = Set(resource.value["items"].array.map { $0["id"].text }) }
                Button("取消选择") { selected = [] }.disabled(selected.isEmpty)
                Button("移入回收站", role: .destructive) { confirming = true }.disabled(selected.isEmpty || working)
            }
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
                        HStack(spacing: 14) {
                            Toggle(item["title"].text, isOn: Binding(get: { selected.contains(item["id"].text) }, set: { if $0 { selected.insert(item["id"].text) } else { selected.remove(item["id"].text) } })).labelsHidden()
                            VStack(alignment: .leading, spacing: 5) { Text(item["title"].text).lineLimit(1); CatalogTextLink(track: catalogSummary(item), store: store).font(.caption).foregroundStyle(.secondary) }
                            Spacer()
                            Text("聆听 \(item["play_count"].integer) 次").font(.caption).foregroundStyle(.secondary)
                            Text(ByteCountFormatter.string(fromByteCount: Int64(item["size"].integer), countStyle: .file)).font(.caption).frame(width: 80, alignment: .trailing)
                        }.padding(.vertical, 12)
                        Divider().opacity(0.4)
                    }
                }
            }.frame(minHeight: 300)
            if !job["id"].text.isEmpty { HStack { Text("\(JobPresentation.status(job["status"].text)) · \(job["phase"].text)").font(.caption); Spacer(); JobActionButtons(store: store, job: job, finished: { await pollJob() }) } }
            HStack { Button("上一页") { page -= 1 }.disabled(page == 0); Spacer(); Text("第 \(page + 1) 页").font(.caption); Spacer(); Button("下一页") { page += 1 }.disabled((page + 1) * 50 >= resource.value["total"].integer) }
        }.padding(28).frame(width: 760, height: 650)
            .task(id: "\(store.sessionID):\(path)") { selected = []; await resource.load(path, store: store) }
            .onChange(of: query) { page = 0 }.onChange(of: cleanup) { page = 0 }.onChange(of: threshold) { page = 0 }.onChange(of: sort) { page = 0 }
            .task(id: job["id"].string) { while !Task.isCancelled && !job["id"].text.isEmpty && !JobPresentation.ended(job["status"].text) { await pollJob(); do { try await Task.sleep(for: .seconds(3)) } catch { return } } }
            .confirmationDialog("将选中的 \(selected.count) 个文件移入回收站？", isPresented: $confirming, titleVisibility: .visible) {
                Button("移入回收站", role: .destructive) { Task { await trash() } }
                Button("取消", role: .cancel) {}
            } message: { Text("回收站仍占用缓存预算。专项清理会再次检查收藏、播放列表和聆听次数；正在使用的文件跳过。") }
    }
    private func trash() async {
        working = true; defer { working = false }
        await store.perform {
            var body: [String: JSONValue] = ["operation": .string("trash"), "ids": .array(selected.sorted().map(JSONValue.string))]
            if cleanup { body["cleanup"] = .object(filter) }
            let response = try await store.api("/library/files/actions", method: "POST", body: .object(body))
            job = .object(["id": response["job_id"], "status": .string("pending")]); selected = []
        }
    }
    private func pollJob() async {
        await store.perform {
            let response = try await store.api(apiQuery("/jobs", ["id": job["id"].text, "limit": "1"]))
            if let latest = response["items"].array.first { job = latest; if JobPresentation.ended(job["status"].text) { await resource.load(path, store: store) } }
        }
    }
}

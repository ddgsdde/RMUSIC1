import SwiftUI
struct ArtworkFailureHistoryView: View {
    let store: MusicStore
    let revision: UUID
    @StoredState private var expanded = false
    @StoredState private var resource = ResourceStore()
    var body: some View {
        DisclosureGroup("最近图片失败 · 最多50条", isExpanded: $expanded) {
            VStack(alignment: .leading, spacing: 12) {
                ResourceNotice(resource: resource)
                if !resource.loading && resource.value["items"].array.isEmpty { Text("暂无失败记录").foregroundStyle(.secondary) }
                ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
                    Text("\(Date(timeIntervalSince1970: item["created_at"].number).formatted()) · \(["upstream":"平台请求","decode":"图片解码","storage":"文件保存"][item["stage"].text] ?? item["stage"].text) · \(item["code"].text) · HTTP \(item["status"].integer) · \(item["elapsed_ms"].integer) ms · \(item["source_id"].text.prefix(10))").font(.caption).textSelection(.enabled)
                }
            }.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 12)
        }.task(id: "\(store.sessionID):\(expanded):\(revision)") { if expanded { await resource.load("/artwork-cache/events", store: store) } }
    }
}

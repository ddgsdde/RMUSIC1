import SwiftUI

struct TrackCommentsView: View {
    let track: Track
    let store: MusicStore
    @StoredState private var resource = ResourceStore()
    @StoredState private var refreshID = UUID()
    @StoredState private var forceRefresh = false
    @StoredState private var kind = "recommend"
    @StoredState private var cursors = [""]
    private var page: Int { cursors.count }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                ThemePicker("排序", selection: $kind) { Text("推荐乐评").tag("recommend"); Text("热门").tag("hot"); Text("最新").tag("new") }.pickerStyle(.segmented).frame(width: 300)
                Spacer()
                Button("获取最新评论", systemImage: "arrow.clockwise") { forceRefresh = true; refreshID = UUID() }.disabled(resource.loading)
            }
            ResourceNotice(resource: resource)
            if resource.loading { ProgressView("正在更新评论…") }
            if !resource.value["notice"].text.isEmpty { Text(resource.value["notice"].text).font(.callout).foregroundStyle(.secondary) }
            Text("仅读取公开内容 · \(resource.value["total"].integer) 条 · \(displayDate(resource.value["fetched_at"].number))").font(.caption).foregroundStyle(.secondary)
            ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
                FeatureCard {
                    HStack { Text(item["author"].text).font(.headline); Spacer(); Text(displayDate(item["published_at"].number)).font(.caption).foregroundStyle(.secondary) }
                    Text(item["text"].text).textSelection(.enabled)
                    Label("\(item["likes"].integer)", systemImage: "hand.thumbsup").font(.caption).foregroundStyle(.secondary)
                    if item["reply_count"].integer > 0 {
                        DisclosureGroup("展开回复 · \(item["reply_count"].integer)") {
                            ForEach(Array(item["replies"].array.enumerated()), id: \.offset) { _, reply in
                                VStack(alignment: .leading, spacing: 8) { Text(reply["author"].text).font(.headline); if !reply["reply_to"].text.isEmpty { Text("回复 \(reply["reply_to"].text)").font(.caption).foregroundStyle(.secondary) }; Text(reply["text"].text).textSelection(.enabled) }.padding(.vertical, 8)
                            }
                            if item["replies"].array.count < item["reply_count"].integer { Text("音源仅返回 \(item["replies"].array.count) 条回复，其余内容尚未返回。").font(.caption).foregroundStyle(.secondary) }
                        }
                    }
                }
            }
            HStack {
                Button("上一页") { cursors.removeLast() }.disabled(page == 1 || resource.loading)
                Text("第 \(page) 页").monospacedDigit(); Spacer()
                Button("下一页") { cursors.append(resource.value["next_cursor"].text) }.disabled(!resource.value["has_more"].boolean || page >= 1000 || resource.loading)
            }
        }.onChange(of: kind) { cursors = [""] }.task(id: "\(store.sessionID):\(track.id):\(kind):\(page):\(refreshID)") { let force = forceRefresh; forceRefresh = false; await load(refresh: force) }
    }
    private func load(refresh: Bool) async {
        let path = "/library/tracks/\(track.id.pathComponentEncoded)/comments"
        let parameters = ["kind": kind, "page": String(page), "page_size": "20", "cursor": cursors.last ?? ""]
        await resource.load(apiQuery(path, parameters.merging(["refresh": String(refresh)]) { _, new in new }), store: store, identity: apiQuery(path, parameters))
    }
}

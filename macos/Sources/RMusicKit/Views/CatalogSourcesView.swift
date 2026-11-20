import SwiftUI

struct CatalogSourcesView: View {
    let store: MusicStore
    let kind: String
    let id: String
    let detail: JSONValue
    let changed: () -> Void
    @StoredState private var resource = ResourceStore()
    @StoredState private var action = WorkflowAction()
    @StoredState private var open = false
    @StoredState private var query = ""
    @StoredState private var refresh = UUID()
    @StoredState private var preview: JSONValue = .null
    @StoredState private var previewRevision: JSONValue = .null
    @StoredState private var link = true
    private var path: String { "/library/catalog/\(kind)/\(id.pathComponentEncoded)/links" }
    private var editable: Bool { store.user["role"].text == "admin" || store.user["scopes"].array.contains { $0.text == "library" } }
    var body: some View {
        DisclosureGroup("资料来源与已确认关联", isExpanded: $open) {
            VStack(alignment: .leading, spacing: 14) {
                ForEach(detail["field_sources"].object.keys.sorted(), id: \.self) { field in
                    DisclosureGroup("\(title(field)) · 当前采用 \(detail["provenance"][field].text == "manual" ? "人工资料" : detail["provenance"][field].text)") {
                        ForEach(Array(detail["field_sources"][field].array.enumerated()), id: \.offset) { _, row in
                            VStack(alignment: .leading, spacing: 5) { Text(row["source"].text).font(.caption.weight(.semibold)); if field == "image_url" { Text("该平台提供图片").font(.caption) } else { Text(row["value"].text).font(.caption).lineLimit(6).textSelection(.enabled) } }
                        }
                    }
                }
                ResourceNotice(resource: resource); WorkflowNotice(action: action)
                ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, person in
                    HStack { Text("\(person["name"].text) · \(person["source"].text)"); Spacer(); if editable && person["direct"].boolean { Button("预览解除") { prepare(person, linking: false) } } }
                }
                if editable {
                    TextField("查找另一平台的同一\(kind == "artist" ? "艺术家" : "专辑发行")", text: $query)
                    ForEach(Array(resource.value["candidates"].array.enumerated()), id: \.offset) { _, person in
                        HStack { Text("\(person["name"].text) · \(person["source"].text)"); Spacer(); Button("预览关联") { prepare(person, linking: true) } }
                    }
                    if !preview.object.isEmpty {
                        FeatureCard {
                            Text("\(link ? "确认同一实体" : "解除关联") · \(preview["name"].text) · \(preview["source"].text)")
                            Text("仅改变资料聚合，不改歌曲、播放列表或聆听历史。名称相同不能证明身份相同。").font(.caption).foregroundStyle(.secondary)
                            HStack { Button(link ? "确认关联" : "确认解除") { action.send(path, method: "PUT", body: payload(preview, previewOnly: false), store: store) { _ in preview = .null; refresh = UUID(); changed() } }; Button("取消") { preview = .null } }
                        }
                    }
                }
            }.padding(.top, 12).disabled(action.busy)
        }.task(id: "\(store.sessionID):\(id):\(open):\(query):\(refresh)") {
            guard open else { return }; do { try await Task.sleep(for: .milliseconds(250)) } catch { return }
            preview = .null
            await resource.load(apiQuery(path, ["q": query]), store: store)
        }.onChange(of: id) { action.cancel(); preview = .null; query = "" }.onChange(of: query) { preview = .null }.onDisappear { action.cancel() }
    }
    private func payload(_ person: JSONValue, previewOnly: Bool) -> JSONValue { .object(["other_id": person["id"], "linked": .bool(link), "expected_revision": previewOnly ? resource.value["revision"] : previewRevision, "preview_only": .bool(previewOnly)]) }
    private func prepare(_ person: JSONValue, linking: Bool) { link = linking; let revision = resource.value["revision"]; action.send(path, method: "PUT", body: payload(person, previewOnly: true), store: store) { _ in preview = person; previewRevision = revision; action.notice = "预览完成，尚未保存" } }
    private func title(_ key: String) -> String { ["description":"简介", "image_url":"图片", "genre":"风格", "language":"语言", "release_date":"发行时间", "label":"唱片公司", "alias":"别名", "country":"地区", "company":"公司", "foreign_name":"外文名"][key] ?? key }
}

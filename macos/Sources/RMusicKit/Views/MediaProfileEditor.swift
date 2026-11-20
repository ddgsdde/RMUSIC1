import SwiftUI

struct MediaProfileEditor: View {
    let store: MusicStore
    let kind: String
    let id: String
    let close: () -> Void
    let saved: () -> Void
    @StoredState private var source = ResourceStore()
    @StoredState private var values: [String: String] = [:]
    @StoredState private var locked: Set<String> = []
    @StoredState private var busy = false
    @StoredState private var saveError: String?
    private let fields = [("image_url", "封面／头像地址"), ("description", "简介"), ("genre", "风格"), ("language", "语言"), ("release_date", "发行日期"), ("label", "唱片公司"), ("country", "地区"), ("birthday", "生日"), ("birthplace", "出生地"), ("foreign_name", "外文名"), ("alias", "别名")]
    private var path: String { "/library/catalog/\(kind)/\(id.pathComponentEncoded)/profile" }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack { Text("编辑资料").font(.title2.bold()); Spacer(); Button("关闭", action: close).disabled(busy) }
            Text("修改的字段会保留在 Core。取消锁定后，恢复采用平台资料。").font(.caption).foregroundStyle(.secondary)
            ResourceNotice(resource: source)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    ForEach(fields, id: \.0) { key, name in
                        VStack(alignment: .leading, spacing: 7) {
                            HStack { Text(name).font(.callout.weight(.medium)); Spacer(); Toggle("锁定", isOn: Binding(get: { locked.contains(key) }, set: { if $0 { locked.insert(key) } else { locked.remove(key) } })).toggleStyle(.checkbox).font(.caption) }
                            let text = Binding(get: { values[key] ?? "" }, set: { values[key] = $0; locked.insert(key) })
                            if key == "description" { TextEditor(text: text).font(.body).frame(minHeight: 150).scrollContentBackground(.hidden).padding(8).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8)) }
                            else { TextField(name, text: text).textFieldStyle(.roundedBorder) }
                        }
                    }
                }
            }
            if let saveError { Text(saveError).foregroundStyle(.red).font(.callout) }
            HStack { Button("重新载入") { Task { await load() } }.disabled(busy || source.loading); Spacer(); Button(busy ? "保存中…" : "保存资料") { Task { await save() } }.buttonStyle(.borderedProminent).disabled(source.loading || busy || source.error != nil) }
        }.padding(24).frame(width: 600, height: 660)
            .task(id: store.sessionID) { await load() }
    }
    private func load() async {
        if await source.load(path, store: store) {
            values = source.value["profile"].object.compactMapValues(\.string)
            locked = Set(source.value["manual"].object.keys)
            saveError = nil
        }
    }
    private func save() async {
        busy = true; defer { busy = false }
        saveError = nil
        do {
            let fields = Dictionary(uniqueKeysWithValues: locked.map { ($0, JSONValue.string(values[$0] ?? "")) })
            let unlock = Set(source.value["manual"].object.keys).subtracting(locked)
            _ = try await store.api(path, method: "PATCH", body: .object(["expected_revision": source.value["revision"], "fields": .object(fields), "unlock": .array(unlock.map { .string($0) })]))
            saved(); close()
        } catch { saveError = error.localizedDescription }
    }
}

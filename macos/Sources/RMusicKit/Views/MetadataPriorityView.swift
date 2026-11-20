import SwiftUI

struct MetadataPriorityView: View {
    let store: MusicStore
    @StoredState private var resource = ResourceStore()
    @StoredState private var order: [String] = []
    @StoredState private var saving = false
    var body: some View {
        FeatureCard {
            Text("平台资料优先级").font(.headline)
            Text("优先使用排在前面的平台；缺失字段由身份已确认的其他平台补全。").font(.caption).foregroundStyle(.secondary)
            ResourceNotice(resource: resource)
            ForEach(Array(order.enumerated()), id: \.element) { index, id in
                HStack {
                    Text("\(index + 1)").foregroundStyle(.secondary).frame(width: 24)
                    Text(resource.value["metadata_providers"].array.first { $0["id"].text == id }?["name"].string ?? id)
                    Spacer()
                    Button { order.swapAt(index, index - 1) } label: { Image(systemName: "arrow.up") }.disabled(index == 0 || saving).help("提高优先级")
                    Button { order.swapAt(index, index + 1) } label: { Image(systemName: "arrow.down") }.disabled(index == order.count - 1 || saving).help("降低优先级")
                }.padding(.vertical, 5)
            }
            if order.isEmpty && !resource.loading { Text("尚无已启用的资料平台").foregroundStyle(.secondary) }
            Button("保存优先级") { Task { saving = true; defer { saving = false }; await store.perform { _ = try await store.api("/settings", method: "PATCH", body: .object(["metadata_provider_priority": .array(order.map { .string($0) })])) } } }.disabled(saving || resource.loading)
        }.task(id: store.sessionID) { if await resource.load("/settings", store: store) { order = resource.value["metadata_provider_priority"].array.compactMap(\.string) } }
    }
}

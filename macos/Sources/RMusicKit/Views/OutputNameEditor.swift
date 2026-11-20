import SwiftUI

struct OutputNameEditor: View {
    let id: String
    let name: String
    let store: MusicStore
    @StoredState private var draft = ""
    @StoredState private var saving = false
    var body: some View {
        HStack {
            TextField("输出名称", text: $draft)
            Button("保存名称") {
                saving = true
                Task {
                    defer { saving = false }
                    await store.perform {
                        _ = try await store.api("/outputs/\(id.pathComponentEncoded)", method: "PATCH", body: .object(["name": .string(draft)]))
                        try await store.outputs.refresh(store)
                    }
                }
            }.disabled(saving || draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || draft.utf8.count > 100)
        }.task(id: id) { draft = name }
    }
}

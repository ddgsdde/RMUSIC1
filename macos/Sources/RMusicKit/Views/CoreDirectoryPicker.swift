import SwiftUI

struct CoreDirectoryField: View {
  let store: MusicStore
  @Binding var path: String
  @StoredState private var choosing = false
  var body: some View {
    HStack {
      Text(path.isEmpty ? "未选择 Core 目录" : path).font(.caption).textSelection(.enabled).lineLimit(2)
      Spacer()
      Button("选择 Core 目录") { choosing = true }
    }
    .sheet(isPresented: $choosing) {
      CoreDirectoryPicker(
        store: store,
        selected: {
          path = $0
          choosing = false
        }, close: { choosing = false })
    }
  }
}
struct CoreDirectoryPicker: View {
  let store: MusicStore
  let selected: (String) -> Void
  let close: () -> Void
  @StoredState private var resource = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var path = ""
  @StoredState private var offset = 0
  @StoredState private var name = ""
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      HStack {
        Text("Core 目录").font(.title2.bold())
        Spacer()
        Button("取消", action: close)
      }
      Text("Docker 中显示容器内的挂载路径。").font(.caption).foregroundStyle(.secondary)
      HStack {
        Button("可用位置") { go("") }
        Button("上一级") { go(resource.value["parent"].text) }.disabled(path.isEmpty)
        Text(resource.value["path"].string ?? "可用位置").font(.caption).textSelection(.enabled)
      }
      ResourceNotice(resource: resource)
      WorkflowNotice(action: action)
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 8) {
          ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
            Button {
              go(item["path"].text)
            } label: {
              Label(item["name"].text, systemImage: "folder").frame(
                maxWidth: .infinity, alignment: .leading
              ).padding(8)
            }.buttonStyle(.plain)
          }
        }
      }.frame(minHeight: 260)
      HStack {
        Button("上一页") { offset = max(0, offset - 200) }.disabled(offset == 0)
        Spacer()
        Button("下一页") { offset += 200 }.disabled(!resource.value["more"].boolean)
      }
      if !resource.value["path"].text.isEmpty {
        HStack {
          TextField("新文件夹名称", text: $name)
          Button("新建文件夹") {
            action.send(
              "/filesystem/directories",
              body: .object(["parent": resource.value["path"], "name": .string(name)]), store: store
            ) { go($0["path"].text) }
          }.disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
        }
      }
      HStack {
        Spacer()
        Button("选择此目录") { selected(resource.value["path"].text) }.buttonStyle(.borderedProminent)
          .disabled(resource.value["path"].text.isEmpty || resource.loading || action.busy)
      }
    }.padding(24).frame(width: 640, height: 530)
      .task(id: "\(store.sessionID):\(path):\(offset)") {
        await resource.load(
          apiQuery("/filesystem/directories", ["path": path, "offset": String(offset)]),
          store: store)
      }
      .onDisappear { action.cancel() }
  }
  private func go(_ value: String) {
    path = value
    offset = 0
    name = ""
  }
}

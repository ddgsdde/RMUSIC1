import SwiftUI

struct LibraryMaintenanceView: View {
  let store: MusicStore
  @StoredState private var roots = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var path = ""
  @StoredState private var refresh = UUID()
  @StoredState private var remove: JSONValue = .null
  @StoredState private var files: String?
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      FeatureCard {
        FeatureHeader(title: "Core 音乐目录") { refresh = UUID() }
        ResourceNotice(resource: roots)
        WorkflowNotice(action: action)
        ForEach(Array(roots.value["items"].array.enumerated()), id: \.offset) { _, item in
          HStack {
            Text(item["path"].text).font(.caption).textSelection(.enabled)
            Spacer()
            Button("移除索引", role: .destructive) { remove = item }
          }
        }
        CoreDirectoryField(store: store, path: $path)
        HStack {
          Button("添加目录") {
            action.send("/library/roots", body: .object(["path": .string(path)]), store: store) {
              _ in
              path = ""
              refresh = UUID()
            }
          }.disabled(path.isEmpty)
          Button("扫描音乐库") { action.send("/library/scan", store: store) }
        }.disabled(action.busy)
      }
      FeatureCard {
        Text("文件管理").font(.headline)
        HStack {
          Button("Core 本地文件") { files = "local" }
          Button("Core 缓存文件") { files = "cache" }
        }
      }
      RecycleBinView(store: store)
    }.task(id: "\(store.sessionID):\(refresh)") { await roots.load("/library/roots", store: store) }
      .onDisappear { action.cancel() }
      .sheet(isPresented: Binding(get: { files != nil }, set: { if !$0 { files = nil } })) {
        LibraryFilesView(store: store, storage: files ?? "cache")
      }
      .confirmationDialog(
        "移除此 Core 音乐目录的索引？原文件保留。",
        isPresented: Binding(
          get: { remove["id"].string != nil }, set: { if !$0 { remove = .null } }),
        titleVisibility: .visible
      ) {
        Button("移除索引", role: .destructive) {
          action.send(
            "/library/roots/\(remove["id"].text.pathComponentEncoded)", method: "DELETE",
            store: store
          ) { _ in refresh = UUID() }
          remove = .null
        }
      }
  }
}
struct RecycleBinView: View {
  let store: MusicStore
  @StoredState private var resource = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var offset = 0
  @StoredState private var refresh = UUID()
  @StoredState private var purge: JSONValue = .null
  @StoredState private var confirmation = ""
  var body: some View {
    FeatureCard {
      FeatureHeader(title: "Core 回收站", loading: resource.loading) { refresh = UUID() }
      ResourceNotice(resource: resource)
      WorkflowNotice(action: action)
      if resource.value["items"].array.isEmpty && !resource.loading {
        Text("回收站为空").font(.caption).foregroundStyle(.secondary)
      }
      ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
        VStack(alignment: .leading, spacing: 8) {
          HStack {
            Text(item["title"].string ?? "未命名歌曲").fontWeight(.medium)
            Spacer()
            Text(item["status"].text == "trashed" ? "已移入回收站" : item["status"].text).font(.caption)
              .foregroundStyle(.secondary)
          }
          Text(item["path"].text).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
          if let error = item["error"].string {
            Text(error).font(.caption).foregroundStyle(.orange)
          }
          HStack {
            Button("恢复") {
              action.send(
                "/library/files/bin/\(item["id"].text.pathComponentEncoded)/restore", store: store
              ) { _ in refresh = UUID() }
            }
            Button("永久删除", role: .destructive) {
              purge = item
              confirmation = ""
            }.disabled(item["status"].text != "trashed")
          }.disabled(action.busy)
        }.padding(.vertical, 6)
      }
      OffsetPageControls(offset: $offset, total: resource.value["total"].integer, limit: 50)
    }.task(id: "\(store.sessionID):\(offset):\(refresh)") {
      await resource.load("/library/files/bin?limit=50&offset=\(offset)", store: store)
    }.onDisappear { action.cancel() }
      .alert(
        "永久删除所选文件？此操作无法撤销。",
        isPresented: Binding(get: { purge["id"].string != nil }, set: { if !$0 { purge = .null } })
      ) {
        TextField("输入 永久删除", text: $confirmation)
        Button("取消", role: .cancel) { purge = .null }
        Button("永久删除", role: .destructive) {
          action.send(
            "/library/files/bin/\(purge["id"].text.pathComponentEncoded)", method: "DELETE",
            body: .object(["confirmation": .string(confirmation)]), store: store
          ) { _ in refresh = UUID() }
          purge = .null
        }.disabled(confirmation != "永久删除")
      }
  }
}

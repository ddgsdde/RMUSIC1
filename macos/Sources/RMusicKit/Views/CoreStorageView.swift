import SwiftUI

/// Core paths and packaged files stay distinct from this client's cache and downloads.
struct CoreStorageView: View {
  let store: MusicStore
  @StoredState private var resource = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var export = FileExportAction()
  @StoredState private var cacheDirectory = ""
  @StoredState private var portableDirectory = ""
  @StoredState private var offset = 0
  @StoredState private var refresh = UUID()
  @StoredState private var drafted = false
  var body: some View {
    FeatureCard {
      FeatureHeader(title: "Core 缓存目录与完整文件", loading: resource.loading) {
        drafted = false
        refresh = UUID()
      }
      ResourceNotice(resource: resource)
      WorkflowNotice(action: action)
      if let error = export.error { Text(error).font(.caption).foregroundStyle(.orange) }
      if !resource.value["enabled"].boolean {
        Label("Core 缓存已关闭", systemImage: "pause.circle").font(.caption).foregroundStyle(.secondary)
      }
      Text("Core 缓存目录").font(.caption).foregroundStyle(.secondary)
      CoreDirectoryField(store: store, path: $cacheDirectory)
      Text("完整歌曲输出目录").font(.caption).foregroundStyle(.secondary)
      CoreDirectoryField(store: store, path: $portableDirectory)
      Button("保存 Core 目录") {
        action.send(
          "/cache", method: "PATCH",
          body: .object([
            "directory": .string(cacheDirectory), "portable_directory": .string(portableDirectory),
          ]), store: store
        ) { _ in refresh = UUID() }
      }.disabled(action.busy || cacheDirectory.isEmpty || portableDirectory.isEmpty)
      Divider()
      ForEach(Array(resource.value["entries"].array.enumerated()), id: \.offset) { _, item in
        VStack(alignment: .leading, spacing: 8) {
          HStack {
            Text(item["title"].string ?? item["track_id"].text).fontWeight(.medium)
            Spacer()
            Text(item["complete"].boolean ? "完整缓存" : "部分缓存").font(.caption).foregroundStyle(
              .secondary)
          }
          CatalogTextLink(track: catalogSummary(item), store: store).font(.caption).foregroundStyle(.secondary)
          if let error = item["exports"]["error"].string {
            Text(error).font(.caption).foregroundStyle(.orange)
          }
          HStack {
            if item["complete"].boolean {
              Button("完善缓存资料") {
                action.send("/cache/\(item["id"].text.pathComponentEncoded)/export", store: store) {
                  _ in refresh = UUID()
                }
              }.disabled(action.busy)
            }
            if ["ready", "complete", "completed"].contains(item["exports"]["status"].text) {
              Button("导出完整歌曲") {
                export.save(
                  "/cache/\(item["id"].text.pathComponentEncoded)/file",
                  name: (item["exports"]["path"].string as NSString?)?.lastPathComponent
                    ?? "\(item["title"].text).\(item["format"].text)", store: store)
              }.disabled(export.busy)
              Menu("导出附加资料") {
                Button("元数据 JSON") {
                  export.save(
                    "/cache/\(item["id"].text.pathComponentEncoded)/file?asset=metadata",
                    name: "metadata.json", store: store)
                }
                Button("歌词 LRC") {
                  export.save(
                    "/cache/\(item["id"].text.pathComponentEncoded)/file?asset=lyrics",
                    name: "\(item["title"].text).lrc", store: store)
                }
              }.disabled(export.busy)
            }
          }
          Divider().opacity(0.3)
        }
      }
      OffsetPageControls(offset: $offset, total: resource.value["total"].integer, limit: 50)
    }.task(id: "\(store.sessionID):\(offset):\(refresh)") {
      if await resource.load("/cache?include_partial=true&limit=50&offset=\(offset)", store: store),
        !drafted
      {
        cacheDirectory = resource.value["directory"].text
        portableDirectory = resource.value["portable_directory"].text
        drafted = true
      }
    }.onDisappear {
      action.cancel()
      export.cancel()
    }
  }
}

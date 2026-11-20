import SwiftUI
import UniformTypeIdentifiers

struct PlaylistImportView: View {
  let store: MusicStore
  let close: () -> Void
  let saved: () -> Void
  @StoredState private var model = PlaylistImportStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var choosing = false
  @StoredState private var file: URL?
  @StoredState private var name = ""
  @StoredState private var offset = 0
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      HStack {
        Text("导入播放列表").font(.title2.bold())
        Spacer()
        Button("关闭", action: close)
      }
      HStack {
        TextField("播放列表名称", text: $name)
        Button("选择播放列表文件") { choosing = true }.disabled(model.loading)
      }
      Text("只匹配 Core 已有歌曲引用；不导入音频。仅准确匹配自动勾选，同名候选需手动确认。").font(.caption).foregroundStyle(.secondary)
      if model.loading {
        HStack {
          ProgressView().controlSize(.small)
          Text("已检查 \(model.progress) 首").font(.caption)
        }
      }
      if let error = model.error {
        Text(error).font(.caption).foregroundStyle(.orange)
        Button("重新预览") {
          if let file {
            self.file = nil
            Task { self.file = file }
          }
        }
      }
      WorkflowNotice(action: action)
      if model.ready {
        Text(
          "已选 \(model.selected.count) / \(model.total) 首 · 准确匹配 \(model.exact) · 待确认 \(model.review) · 未找到 \(model.missing)"
        ).font(.caption).foregroundStyle(.secondary)
        ScrollView {
          LazyVStack(alignment: .leading, spacing: 12) {
            ForEach(Array(model.rows.enumerated()), id: \.offset) { local, item in
              let index = offset + local
              VStack(alignment: .leading, spacing: 6) {
                Text(
                  "\(index + 1). \(item["entry"]["title"].string ?? item["entry"]["location"].text)"
                ).fontWeight(.medium).lineLimit(1)
                Text("\(item["entry"]["artist"].text) · \(item["entry"]["album"].text)").font(
                  .caption
                ).foregroundStyle(.secondary)
                if let id = item["matched"]["id"].string {
                  Toggle(
                    "准确匹配 · \(item["matched"]["title"].text)",
                    isOn: Binding(
                      get: { model.selected[index] == id },
                      set: {
                        if $0 {
                          model.selected[index] = id
                        } else {
                          model.selected.removeValue(forKey: index)
                        }
                      })
                  ).toggleStyle(.checkbox)
                } else if !item["candidates"].array.isEmpty {
                  ThemePicker(
                    "手动确认",
                    selection: Binding(
                      get: { model.selected[index] ?? "" },
                      set: {
                        if $0.isEmpty {
                          model.selected.removeValue(forKey: index)
                        } else {
                          model.selected[index] = $0
                        }
                      })
                  ) {
                    Text("跳过").tag("")
                    ForEach(Array(item["candidates"].array.enumerated()), id: \.offset) {
                      _, candidate in
                      Text(
                        "\(candidate["title"].text) · \(candidate["artist"].text) · \(candidate["album"].text) · \(candidate["source"].text) · \(candidate["duration"].integer) 秒"
                      ).tag(candidate["id"].text)
                    }
                  }
                } else {
                  Text("Core 未找到，跳过").font(.caption).foregroundStyle(.secondary)
                }
              }
              Divider().opacity(0.3)
            }
          }
        }
        OffsetPageControls(offset: $offset, total: model.total, limit: 50, disabled: model.loading)
        HStack {
          Button("清空选择") { model.selected = [:] }
          Spacer()
          Button("创建播放列表（\(model.selected.count) 首）") { submit() }.buttonStyle(.borderedProminent)
            .disabled(model.selected.isEmpty || name.isEmpty || action.busy || model.loading)
        }
      } else {
        Spacer()
      }
    }.padding(24).frame(width: 780, height: 650)
      .fileImporter(
        isPresented: $choosing,
        allowedContentTypes: [
          .json, .commaSeparatedText, UTType(filenameExtension: "m3u") ?? .text,
          UTType(filenameExtension: "m3u8") ?? .text, UTType(filenameExtension: "pls") ?? .text,
        ]
      ) { result in
        if case .success(let url) = result {
          file = url
          name = url.deletingPathExtension().lastPathComponent
          offset = 0
        } else if case .failure(let error) = result {
          model.error = error.localizedDescription
        }
      }
      .task(id: file) { if let file { await model.scan(file, store: store) } }
      .task(id: offset) { if offset > 0 || model.ready { await model.page(offset, store: store) } }
      .onDisappear {
        action.cancel()
        model.reset()
      }
  }
  private func submit() {
    do {
      let payload = try model.payload(name: name)
      action.send("/playlists/import", body: payload, store: store) { _ in
        saved()
        close()
      }
    } catch { model.error = error.localizedDescription }
  }
}
struct PlaylistExportButton: View {
  let store: MusicStore
  let id: String
  let name: String
  @StoredState private var action = FileExportAction()
  var body: some View {
    VStack(alignment: .leading) {
      Menu(action.busy ? "正在导出…" : "导出播放列表") {
        ForEach(["json", "m3u8", "pls", "csv"], id: \.self) { format in
          Button(format.uppercased()) {
            action.save(
              "/playlists/\(id.pathComponentEncoded)/export?format=\(format)",
              name: "\(name).\(format)", store: store)
          }
        }
      }.disabled(action.busy)
      if let error = action.error { Text(error).font(.caption).foregroundStyle(.orange) }
    }.onDisappear { action.cancel() }
  }
}

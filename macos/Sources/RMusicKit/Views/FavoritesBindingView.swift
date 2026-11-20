import SwiftUI

struct FavoritesBindingView: View {
  let store: MusicStore
  let close: () -> Void
  @StoredState private var favorite = ResourceStore()
  @StoredState private var catalog = ResourceStore()
  @StoredState private var plugins = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var selected = ""
  @StoredState private var takingOver = false
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      HStack {
        Text("我喜欢 · 云端绑定").font(.title2.bold())
        Spacer()
        Button("关闭", action: close)
      }
      ResourceNotice(resource: favorite)
      ResourceNotice(resource: catalog)
      WorkflowNotice(action: action)
      Text(favorite.value["sync"]["provider"].string.map { "当前绑定：\($0)" } ?? "当前保存在 Core，尚未绑定云端歌单。")
        .font(.caption).foregroundStyle(.secondary)
      ThemePicker("可写云端歌单", selection: $selected) {
        Text("选择歌单").tag("")
        ForEach(Array(targets.enumerated()), id: \.offset) { _, item in
          Text("\(item["source"].text) · \(item["name"].text)").tag(key(item))
        }
      }
      Text("首次绑定会合并两边已有歌曲。喜欢／取消喜欢将排队同步；解除绑定保留两边内容。").font(.caption).foregroundStyle(.secondary)
      if targets.isEmpty {
        Text("暂无支持双向同步的歌单，请登录音乐平台并刷新云端目录。").font(.caption).foregroundStyle(.secondary)
      }
      HStack {
        if favorite.value["sync"]["provider"].string != nil {
          Button("解除绑定") {
            action.send("/favorites/binding", method: "DELETE", store: store) { _ in finish() }
          }
        }
        Spacer()
        Button("绑定并双向同步") { bind(false) }.buttonStyle(.borderedProminent).disabled(
          selected.isEmpty || action.busy)
      }
    }.padding(24).frame(width: 680, height: 330).task(id: store.sessionID) {
      await favorite.load("/favorites", store: store)
      await catalog.load("/playlist-directory", store: store)
      await plugins.load("/plugins", store: store)
    }
    .onChange(of: action.status) { if action.status == 409 { takingOver = true } }
    .onDisappear { action.cancel() }
    .confirmationDialog(
      "目标已有本地同步副本。转移绑定后，原副本及未同步修改保留为本地内容，停止云端同步。", isPresented: $takingOver,
      titleVisibility: .visible
    ) { Button("确认转移并绑定") { bind(true) } }
  }
  private var targets: [JSONValue] {
    catalog.value["items"].array.filter { item in
      item["writable"].boolean
        && ["add", "remove"].allSatisfy { cap in
          item["write_capabilities"].array.contains { $0.string == cap }
        }
        && plugins.value["items"].array.contains { plugin in
          plugin["id"].text == item["source"].text && plugin["enabled"].boolean
            && ["playlist_write_v1", "account_context_v1"].allSatisfy { cap in
              plugin["capabilities"].array.contains { $0.string == cap }
            }
        }
    }
  }
  private func key(_ item: JSONValue) -> String {
    String(
      data: (try? JSONEncoder().encode([item["source"], item["account_context"], item["id"]]))
        ?? Data(), encoding: .utf8) ?? ""
  }
  private func bind(_ takeover: Bool) {
    guard let target = targets.first(where: { key($0) == selected }) else { return }
    action.send(
      "/favorites/binding", method: "PUT",
      body: .object([
        "provider": target["source"], "playlist_id": target["id"],
        "account_context": target["account_context"], "take_over": .bool(takeover),
      ]), store: store
    ) { _ in finish() }
  }
  private func finish() {
    Task {
      await store.perform { try await store.collection.refresh(store) }
      close()
    }
  }
}
struct PlaylistSyncStatusView: View {
  let store: MusicStore
  let id: String
  @StoredState private var resource = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var resolution = ""
  @StoredState private var conflictID: String? = nil
  @StoredState private var refresh = UUID()
  var body: some View {
    Group {
      if !resource.value["sync"].object.isEmpty {
        FeatureCard {
          HStack {
            Text("云端同步").font(.headline)
            Spacer()
            Text(status).foregroundStyle(.secondary)
            Button("立即同步") { sync(.object([:])) }.disabled(action.busy)
          }
          ResourceNotice(resource: resource)
          WorkflowNotice(action: action)
          Toggle(
            "每小时拉取",
            isOn: Binding(
              get: { resource.value["sync"]["auto_sync"].boolean },
              set: { sync(.object(["auto_sync": .bool($0)])) })
          ).toggleStyle(.switch).disabled(action.busy)
          if let error = resource.value["sync"]["error"].string {
            Text(error).font(.caption).foregroundStyle(.orange)
          }
          if resource.value["sync"]["status"].text == "conflict" {
            Text(resource.value["sync"]["conflict"]["reason"].text).font(.caption).foregroundStyle(
              .orange)
            HStack {
              Text(
                "Core \(resource.value["sync"]["conflict"]["local"]["track_ids"].array.count) 首 / 云端 \(resource.value["sync"]["conflict"]["cloud"]["track_ids"].array.count) 首"
              ).font(.caption)
              Spacer()
              Button("保留 Core 版本") { choose("local") }.disabled(resource.value["sync"]["capabilities"].array.isEmpty)
              Button("采用云端版本") { choose("cloud") }
            }.disabled(action.busy)
          }
        }
      }
    }.task(id: "\(store.sessionID):\(id):\(refresh)") {
      repeat {
        await resource.load("/playlists/\(id.pathComponentEncoded)", store: store)
        do { try await Task.sleep(for: .seconds(5)) } catch { return }
      } while !Task.isCancelled
    }
    .onDisappear { action.cancel() }
    .onChange(of: action.status) { if action.status == 409 { refresh = UUID() } }
    .confirmationDialog(
      "以所选版本解决同步冲突？其他来源且未匹配的歌曲仍保留。",
      isPresented: Binding(get: { !resolution.isEmpty }, set: { if !$0 { resolution = "" } }),
      titleVisibility: .visible
    ) {
      Button("确认处理") {
        var body: [String: JSONValue] = ["resolve": .string(resolution)]
        if let conflictID { body["conflict_id"] = .string(conflictID) }
        sync(.object(body))
        resolution = ""
      }
    }
  }
  private func choose(_ option: String) {
    conflictID = resource.value["sync"]["conflict"]["id"].string
    resolution = option
  }
  private var status: String {
    ["pending": "等待同步", "synced": "已同步", "partial": "部分未同步", "conflict": "同步冲突", "failed": "同步失败"][
      resource.value["sync"]["status"].text] ?? resource.value["sync"]["status"].text
  }
  private func sync(_ body: JSONValue) {
    action.send("/playlists/\(id.pathComponentEncoded)/sync", body: body, store: store) { _ in
      refresh = UUID()
    }
  }
}

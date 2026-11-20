import AppKit
import ImageIO
import SwiftUI
import UniformTypeIdentifiers

struct MusicPlatformsView: View {
  let store: MusicStore
  @StoredState private var resource = ResourceStore()
  @StoredState private var refreshID = UUID()
  @StoredState private var importing = false
  @StoredState private var installation = WorkflowAction()
  private var installing: Bool { installation.busy }
  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      HStack {
        FeatureHeader(title: "音乐平台", loading: resource.loading) { refreshID = UUID() }
        Button("导入插件", systemImage: "shippingbox") { importing = true }.disabled(installing)
      }
      ResourceNotice(resource: resource)
      WorkflowNotice(action: installation)
      ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, plugin in
        PlatformCard(store: store, plugin: plugin, changed: { refreshID = UUID() })
      }
    }.task(id: "\(store.sessionID):\(refreshID)") { await refresh() }
      .fileImporter(isPresented: $importing, allowedContentTypes: [.zip]) { result in
        guard case .success(let url) = result else { return }
        installation.run(
          store: store,
          operation: {
            let access = url.startAccessingSecurityScopedResource()
            defer { if access { url.stopAccessingSecurityScopedResource() } }
            return try await store.runtimeCall(
              "install_plugin", .object(["path": .string(url.path)]))
          }
        ) { _ in refreshID = UUID() }

      }.onDisappear { installation.cancel() }
  }
  private func refresh() async { await resource.load("/plugins", store: store) }
}
private struct PlatformCard: View {
  let store: MusicStore
  let plugin: JSONValue
  let changed: () -> Void
  @StoredState private var health = ResourceStore()
  @StoredState private var quality = ResourceStore()
  @StoredState private var action = WorkflowAction()
  private var working: Bool { action.busy }
  @StoredState private var login: JSONValue = .null
  @StoredState private var rollback = false
  @StoredState private var healthRefresh = UUID()
  private var id: String { plugin["id"].text }
  var body: some View {
    FeatureCard {
      HStack {
        Image(systemName: "music.note.house").font(.title).foregroundStyle(MusicStyle.accent)
        VStack(alignment: .leading, spacing: 5) {
          Text(plugin["name"].text).font(.headline)
          Text("v\(plugin["version"].text) · \(plugin["enabled"].boolean ? "已启用" : "已停用")").font(
            .caption
          ).foregroundStyle(.secondary)
        }
        Spacer()
        Toggle(
          "启用",
          isOn: Binding(
            get: { plugin["enabled"].boolean },
            set: { enabled in
              act(
                "/plugins/\(id.pathComponentEncoded)", method: "PATCH",
                body: .object(["enabled": .bool(enabled)]))
            })
        ).labelsHidden().toggleStyle(.switch).controlSize(.small)
      }
      if plugin["enabled"].boolean {
        HStack {
          Label(
            health.loading ? "正在检查登录状态…" : health.error != nil ? "登录状态检查失败" : health.value["login_pending"].boolean ? "等待扫码确认" : health.value["auth_status"].text == "expired" ? "登录已过期" : health.value["authenticated"].boolean ? "已登录" : "未登录",
            systemImage: health.value["authenticated"].boolean
              ? "person.crop.circle.badge.checkmark" : "person.crop.circle")
          Spacer()
          Button("检查连接") { healthRefresh = UUID() }
          if plugin["capabilities"].array.map(\.text).contains("logout") && health.value["authenticated"].boolean {
            Button("退出音源账号") { act("/providers/\(id.pathComponentEncoded)/logout") }
          }
        }.font(.caption)
        ResourceNotice(resource: health)
        ResourceNotice(resource: quality)
        if !quality.value["quality_options"].array.isEmpty {
          ThemePicker(
            "在线音质",
            selection: Binding(
              get: { quality.value["quality"].text },
              set: { value in
                act(
                  "/providers/\(id.pathComponentEncoded)/settings", method: "PATCH",
                  body: .object(["quality": .string(value)]))
              })
          ) {
            ForEach(Array(quality.value["quality_options"].array.enumerated()), id: \.offset) {
              _, option in Text(option["label"].string ?? option["id"].text).tag(option["id"].text)
            }
          }
        }
        HStack {
          ForEach(plugin["login_types"].array.map(\.text), id: \.self) { type in
            Button(plugin["login_labels"][type].string ?? type) { startLogin(type) }
          }
          if plugin["login_types"].array.isEmpty
            && plugin["capabilities"].array.map(\.text).contains(where: {
              ["login", "qr_login", "auth"].contains($0)
            })
          {
            Button("扫码登录") { startLogin(nil) }
          }
          if plugin["capabilities"].array.map(\.text).contains(where: {
            ["sync", "playlists"].contains($0)
          }) {
            Button("同步资料") { act("/providers/\(id.pathComponentEncoded)/sync") }
          }
        }
      }
      DisclosureGroup("插件详情") {
        VStack(alignment: .leading, spacing: 10) {
          Text(plugin["capabilities"].array.map(\.text).joined(separator: " · ")).font(.caption)
            .foregroundStyle(.secondary)
          if !plugin["error"].text.isEmpty {
            Text(plugin["error"].text).font(.caption).foregroundStyle(.orange)
          }
          Button("回退上一版本") { rollback = true }
        }.padding(.top, 8)
      }
      WorkflowNotice(action: action)
    }.disabled(working)
      .task(id: "\(store.sessionID):\(id):\(plugin["enabled"].boolean):\(healthRefresh)") {
        if plugin["enabled"].boolean { await refreshState() }
      }
      .sheet(
        isPresented: Binding(
          get: { login["session_id"].string != nil }, set: { if !$0 { login = .null } })
      ) {
        PlatformLoginView(
          store: store, provider: id, initial: login,
          close: {
            login = .null
            healthRefresh = UUID()
          })
      }
      .onChange(of: store.sessionID) {
        action.cancel()
        login = .null
      }
      .onDisappear {
        action.cancel()
        login = .null
      }
      .confirmationDialog(
        "回退 \(plugin["name"].text) 至上一版本？", isPresented: $rollback, titleVisibility: .visible
      ) { Button("确认回退") { act("/plugins/\(id.pathComponentEncoded)/rollback") } }
  }
  private func refreshState() async {
    await health.load("/plugins/\(id.pathComponentEncoded)/health", store: store)
    await quality.load("/providers/\(id.pathComponentEncoded)/settings", store: store)
  }
  private func act(_ path: String, method: String = "POST", body: JSONValue = .object([:])) {
    action.send(path, method: method, body: body, store: store) { _ in
      changed()
      healthRefresh = UUID()
    }
  }
  private func startLogin(_ type: String?) {
    action.send(
      "/providers/\(id.pathComponentEncoded)/login",
      body: .object(type.map { ["type": .string($0)] } ?? [:]), store: store
    ) { value in
      login = value
      action.notice = nil
    }
  }
}
private struct PlatformLoginView: View {
  let store: MusicStore
  let provider: String
  let initial: JSONValue
  let close: () -> Void
  @StoredState private var value: JSONValue = .null
  @StoredState private var image: NSImage?
  @StoredState private var error: String?
  private var finished: Bool {
    [
      "done", "success", "authenticated", "logged_in", "expired", "error", "timeout", "refused",
      "cancelled",
    ].contains(value["status"].text)
  }
  var body: some View {
    VStack(spacing: 20) {
      HStack {
        Text(initial["login_copy"]["title"].string ?? "连接音乐平台").font(.headline)
        Spacer()
        Button("关闭", action: close)
      }
      if let image, !finished {
        Image(nsImage: image).interpolation(.none).resizable().scaledToFit().frame(
          width: 240, height: 240)
      }
      if ["done", "success", "authenticated", "logged_in"].contains(value["status"].text) {
        Label("登录成功", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
      } else {
        Text(
          value["status"].text == "scanned"
            ? initial["login_copy"]["scanned"].string ?? "请在音乐平台确认"
            : finished ? "登录已结束，可关闭后重试" : initial["login_copy"]["scan"].string ?? "请使用音乐平台扫码")
      }
      if let error { Text(error).font(.caption).foregroundStyle(.orange) }
    }.padding(28).frame(width: 400)
      .task(id: "\(store.sessionID):\(initial["session_id"].text)") {
        value = initial
        updateImage()
        while !Task.isCancelled && !finished {
          do {
            try await Task.sleep(for: .seconds(2))
            try Task.checkCancellation()
            let next = try await store.api(
              "/providers/\(provider.pathComponentEncoded)/login/\(initial["session_id"].text.pathComponentEncoded)"
            )
            try Task.checkCancellation()
            value = next
            updateImage()
          } catch is CancellationError { return } catch {
            self.error = error.localizedDescription
            return
          }
        }
        if finished { image = nil }
      }
  }
  private func updateImage() {
    guard let raw = value["qr_data_url"].string ?? initial["qr_data_url"].string,
      raw.utf8.count < 1_048_576, raw.hasPrefix("data:image/"),
      let separator = raw.firstIndex(of: ","),
      let bytes = Data(base64Encoded: String(raw[raw.index(after: separator)...])),
      let source = CGImageSourceCreateWithData(bytes as CFData, nil),
      let bitmap = CGImageSourceCreateThumbnailAtIndex(
        source, 0,
        [
          kCGImageSourceCreateThumbnailFromImageAlways: true,
          kCGImageSourceThumbnailMaxPixelSize: 512,
        ] as CFDictionary)
    else { return }
    image = NSImage(cgImage: bitmap, size: NSSize(width: bitmap.width, height: bitmap.height))
  }
}

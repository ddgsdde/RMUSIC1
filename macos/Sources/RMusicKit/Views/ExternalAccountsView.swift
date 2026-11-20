import SwiftUI

struct ExternalAccountsView: View {
  let store: MusicStore
  @StoredState private var accounts = ResourceStore()
  @StoredState private var plugins = ResourceStore()
  @StoredState private var jobs = ResourceStore()
  @StoredState private var refresh = UUID()
  @StoredState private var presentation: AccountPresentation?
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      HStack {
        Text("外部账号").font(.title2.bold())
        Spacer()
        Button("导入账号数据") { open("import", .null) }
        Button("创建外部账号") { open("create", .null) }
      }
      Text("每个账号独立保存歌单、聆听历史、统计和推荐。音源默认不授权，同账号设备可遥控和流转播放。")
        .font(.callout).foregroundStyle(.secondary)
      ResourceNotice(resource: accounts)
      ForEach(Array(accounts.value["items"].array.enumerated()), id: \.offset) { _, account in
        FeatureCard {
          HStack {
            VStack(alignment: .leading, spacing: 6) {
              Text(account["username"].text).font(.headline)
              Text("\(account["enabled"].boolean ? "已启用" : "已停用") · \(account["totp_enabled"].boolean ? "两步验证已启用" : "未设置两步验证")")
                .font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Button("管理") { open("edit", account) }
            Button("导出") { open("export", account) }
            Button("删除", role: .destructive) { open("delete", account) }
          }
        }
      }
      if !accounts.value["next_cursor"].text.isEmpty {
        Button("加载更多账号") {
          let previous = accounts.value["items"].array
          let owner = store.sessionID
          Task {
            guard await accounts.load("/auth/external-accounts?cursor=\(accounts.value["next_cursor"].text.pathComponentEncoded)", store: store), owner == store.sessionID else { return }
            var result = accounts.value.object
            result["items"] = .array(previous + accounts.value["items"].array)
            accounts.value = .object(result)
          }
        }
      }
      ForEach(Array(jobs.value["items"].array.enumerated()), id: \.offset) { _, job in
        HStack {
          Text("\(job["manifest"]["username"].text) · \(job["state"].text) · \(job["applied"].number ?? 0, specifier: "%.0f")/\(job["records"].number ?? 0, specifier: "%.0f")")
          Spacer()
          Button("查看／继续导入") { open("import", job) }
        }.font(.callout)
      }
    }
    .task(id: "\(store.sessionID):\(refresh)") {
      await accounts.load("/auth/external-accounts", store: store)
      await plugins.load("/plugins", store: store)
      await jobs.load("/auth/account-imports", store: store)
    }
    .sheet(item: $presentation) { item in
      if item.mode == "import" {
        ExternalAccountImportView(store: store, initial: item.account, accounts: accounts.value["items"].array, plugins: plugins.value["items"].array) {
          presentation = nil; refresh = UUID()
        }
      } else {
        ExternalAccountEditor(store: store, account: item.account, mode: item.mode, plugins: plugins.value["items"].array) {
          presentation = nil; refresh = UUID()
        }
      }
    }
  }
  private func open(_ mode: String, _ account: JSONValue) { presentation = AccountPresentation(mode: mode, account: account) }
}

private struct AccountPresentation: Identifiable {
  let id = UUID()
  let mode: String
  let account: JSONValue

}

struct ExternalPolicyFields: View {
  let plugins: [JSONValue]
  @Binding var providers: Set<String>
  @Binding var cache: Bool
  @Binding var library: Bool
  @Binding var bandwidth: String
  private var identifiers: [String] { Array(Set(plugins.map { $0["id"].text }).union(providers)).filter { !$0.isEmpty }.sorted() }
  var body: some View {
    GroupBox("可用音源") {
      VStack(alignment: .leading, spacing: 12) {
        ForEach(identifiers, id: \.self) { id in
          Toggle(plugins.first { $0["id"].text == id }?["name"].text ?? id, isOn: Binding(get: { providers.contains(id) }, set: { if $0 { providers.insert(id) } else { providers.remove(id) } }))
        }
        Toggle("在线平台已缓存的音乐", isOn: $cache)
        Toggle("自有本地曲库", isOn: $library)
        Text("账号总带宽（Mbps，0 为不限）").font(.caption).foregroundStyle(.secondary)
        TextField("账号总带宽（Mbps，0 为不限）", text: $bandwidth)
        Text("多台设备共用带宽额度。外部账号不参与家庭音区同步。")
          .font(.caption).foregroundStyle(.secondary)
      }.padding(8).frame(maxWidth: .infinity, alignment: .leading)
    }
  }
  static func value(_ providers: Set<String>, _ cache: Bool, _ library: Bool, _ bandwidth: String) throws -> JSONValue {
    guard let speed = Double(bandwidth), speed.isFinite, speed >= 0, speed * 125_000 <= 1_073_741_824 else { throw RuntimeFailure(message: "请输入有效的账号带宽。") }
    return .object(["providers": .array(providers.sorted().map(JSONValue.string)), "local_cache": .bool(cache), "local_library": .bool(library), "bandwidth_bytes_per_second": .number((speed * 125_000).rounded())])
  }
}

private struct ExternalAccountEditor: View {
  let store: MusicStore
  let account: JSONValue
  let mode: String
  let plugins: [JSONValue]
  let close: () -> Void
  @StoredState private var action = WorkflowAction()
  @StoredState private var name = ""
  @StoredState private var password = ""
  @StoredState private var current = ""
  @StoredState private var code = ""
  @StoredState private var confirmation = ""
  @StoredState private var enabled = true
  @StoredState private var reset = false
  @StoredState private var providers: Set<String> = []
  @StoredState private var cache = false
  @StoredState private var library = false
  @StoredState private var bandwidth = "0"
  private var title: String { ["create": "创建外部账号", "edit": "管理外部账号", "delete": "删除外部账号", "export": "导出账号数据"][mode] ?? "外部账号" }
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      HStack { Text(title).font(.title2.bold()); Spacer(); Button("关闭", action: close).disabled(action.busy) }
      ScrollView {
        VStack(alignment: .leading, spacing: 16) {
          if mode == "create" || mode == "edit" {
            TextField("用户名", text: $name)
            SecureField(mode == "edit" ? "新密码（留空保留）" : "初始密码（至少 10 位）", text: $password)
            ExternalPolicyFields(plugins: plugins, providers: $providers, cache: $cache, library: $library, bandwidth: $bandwidth)
          }
          if mode == "edit" { Toggle("启用账号", isOn: $enabled); Toggle("重置两步验证和恢复码", isOn: $reset) }
          if mode == "delete" { Text("删除 \(account["username"].text) 及其歌单、历史、统计和设备记录。"); TextField("输入完整用户名确认", text: $confirmation) }
          if mode == "export" { Text("导出歌单、偏好、历史和统计，不包含密码、两步验证密钥或平台登录凭据。") }
          SecureField("当前管理员密码", text: $current)
          TextField("两步验证码或恢复码（已启用时填写）", text: $code)
          WorkflowNotice(action: action)
          Button("确认") { submit() }.buttonStyle(.borderedProminent)
            .disabled(action.busy || current.isEmpty || mode == "delete" && confirmation != account["username"].text)
        }.padding(2)
      }
    }.padding(24).frame(minWidth: 560, idealWidth: 620, minHeight: 520, idealHeight: 680)
      .interactiveDismissDisabled(action.busy)
      .task {
        name = account["username"].text; enabled = account["enabled"].boolean || mode == "create"
        providers = Set(account["policy"]["providers"].array.map(\.text)); cache = account["policy"]["local_cache"].boolean; library = account["policy"]["local_library"].boolean
        bandwidth = String((account["policy"]["bandwidth_bytes_per_second"].number ?? 0) / 125_000)
      }
      .onChange(of: action.busy) { if !action.busy { current = ""; code = ""; password = "" } }
      .onDisappear { action.cancel(); current = ""; code = ""; password = "" }
  }
  private func submit() {
    var body: [String: JSONValue] = ["current_password": .string(current)]
    if !code.isEmpty { body["totp"] = .string(code) }
    if mode == "create" || mode == "edit" {
      guard !name.trimmingCharacters(in: .whitespaces).isEmpty, mode != "create" || password.count >= 10 else { action.error = "请输入用户名和至少 10 位密码。"; return }
      do { body["policy"] = try ExternalPolicyFields.value(providers, cache, library, bandwidth) } catch { action.error = error.localizedDescription; return }
      body["username"] = .string(name.trimmingCharacters(in: .whitespaces))
      if mode == "create" { body["password"] = .string(password) }
      if mode == "edit" { if !password.isEmpty { body["new_password"] = .string(password) }; body["enabled"] = .bool(enabled); body["reset_totp"] = .bool(reset) }
    }
    if mode == "delete" { body["confirmation"] = .string(confirmation) }
    let credentials = JSONValue.object(body)
    let path = "/auth/external-accounts/\(account["id"].text.pathComponentEncoded)"
    if mode == "export" {
      action.run(store: store, operation: {
        let ticket = try await store.api(path + "/export-ticket", method: "POST", body: credentials)
        let route = "/" + ticket["api_path"].text.replacingOccurrences(of: "api/v1/", with: "")
        try await CoreFileService.save(route, name: "rmusic-\(account["username"].text).jsonl", store: store)
        return .null
      }) { _ in close() }
    } else {
      action.send(mode == "create" ? "/auth/external-accounts" : path, method: mode == "create" ? "POST" : mode == "delete" ? "DELETE" : "PATCH", body: credentials, store: store) { _ in close() }
    }
  }
}

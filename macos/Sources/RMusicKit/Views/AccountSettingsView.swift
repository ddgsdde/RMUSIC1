import SwiftUI

struct AccountSettingsView: View {
  let store: MusicStore
  @StoredState private var sessions = ResourceStore()
  @StoredState private var tokens = ResourceStore()
  @StoredState private var users = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var refresh = UUID()
  @StoredState private var tab = "account"
  @StoredState private var securityPassword = ""
  @StoredState private var name = ""
  @StoredState private var password = ""
  @StoredState private var scopes: Set<String> = ["read"]
  @StoredState private var secret: JSONValue = .null
  @StoredState private var code = ""
  @StoredState private var confirmation: JSONValue = .null
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      FeatureHeader(title: "账号与安全")
      ThemePicker("管理项目", selection: $tab) {
        Text("账号与密码").tag("account")
        Text("登录会话").tag("sessions")
        if store.user["role"].text != "external" { Text("访问凭证").tag("tokens") }
        Text("两步验证").tag("totp")
        if admin { Text("家庭账号").tag("users"); Text("外部账号").tag("external") }
      }.pickerStyle(.segmented)
      WorkflowNotice(action: action)
      if tab == "external" && admin {
        ExternalAccountsView(store: store)
      } else if tab == "account" {
        AccountCredentialsView(store: store)
        Button("退出当前账号",systemImage:"rectangle.portrait.and.arrow.right"){Task {await store.disconnect()}}
      } else if tab == "sessions" {
        ResourceNotice(resource: sessions)
        ForEach(Array(sessions.value["items"].array.enumerated()), id: \.offset) { _, item in
          FeatureCard {
            HStack {
              VStack(alignment: .leading, spacing: 6) {
                Text(item["device_name"].string ?? "未标识设备").font(.headline)
                let kind = ["macos":"Mac", "android":"Android", "windows":"Windows", "linux":"Linux", "browser":"浏览器"][item["device_type"].text] ?? "设备类型未知"
                Text(item["current"].boolean ? "\(kind) · 当前会话" : kind).font(.caption).foregroundStyle(.secondary)
                Text("创建于 \(date(item["created_at"])) · 到期 \(date(item["expires_at"]))").font(
                  .caption
                ).foregroundStyle(.secondary)
              }
              Spacer()
              Button("注销", role: .destructive) {
                confirmation = .object([
                  "path": .string("/auth/sessions/\(item["id"].text.pathComponentEncoded)"),
                  "current": item["current"],
                ])
              }
            }
          }
        }
      } else if tab == "users" {
        ResourceNotice(resource: users)
        ForEach(Array(users.value["items"].array.enumerated()), id: \.offset) { _, item in
          HStack {
            Text(item["username"].text)
            Spacer()
            Text(item["role"].text == "admin" ? "管理员" : "家庭成员").foregroundStyle(.secondary)
          }
        }
        FeatureCard {
          Text("创建家庭账号").font(.headline)
          TextField("用户名", text: $name)
          SecureField("初始密码（至少 12 位）", text: $password)
          Button("创建账号") {
            action.send(
              "/auth/users",
              body: .object(["username": .string(name), "password": .string(password)]),
              store: store
            ) { _ in
              password = ""
              name = ""
              refresh = UUID()
            }
          }.disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || password.count < 12)
        }
      } else if tab == "tokens" {
        ResourceNotice(resource: tokens)
        if admin {
          FeatureCard {
            Text("新建访问凭证").font(.headline)
            TextField("凭证名称", text: $name)
            HStack {
              ForEach(
                [("read", "读取"), ("control", "播放控制"), ("library", "曲库编辑"), ("admin", "管理员")],
                id: \.0
              ) { id, label in
                Toggle(
                  label,
                  isOn: Binding(
                    get: { scopes.contains(id) },
                    set: { if $0 { scopes.insert(id) } else { scopes.remove(id) } })
                ).toggleStyle(.checkbox)
              }
            }
            Button("生成凭证") {
              action.send(
                "/auth/tokens",
                body: .object([
                  "name": .string(name), "scopes": .array(scopes.sorted().map(JSONValue.string)),
                ]), store: store
              ) {
                secret = $0
                refresh = UUID()
              }
            }.disabled(name.isEmpty || scopes.isEmpty)
          }
        }
        if let token = secret["token"].string {
          FeatureCard {
            Text("新凭证仅显示这一次，请保存在私有位置。").font(.caption)
            Text(token).font(.system(.callout, design: .monospaced)).textSelection(.enabled)
            Button("已保存，隐藏凭证") { secret = .null }
          }
        }
        ForEach(Array(tokens.value["items"].array.enumerated()), id: \.offset) { _, item in
          HStack {
            Text(item["name"].text)
            Text(item["scopes"].array.map(\.text).joined(separator: " · ")).font(.caption)
              .foregroundStyle(.secondary)
            Spacer()
            Button("撤销", role: .destructive) {
              confirmation = .object([
                "path": .string("/auth/tokens/\(item["id"].text.pathComponentEncoded)")
              ])
            }
          }
        }
      } else {
        FeatureCard {
          Text(store.user["totp_enabled"].boolean ? "两步验证已启用" : "使用验证器保护登录").font(.headline)
          Text("两步验证为可选设置。修改后保留当前登录，其他设备需重新登录。").font(.caption).foregroundStyle(.secondary)
          if secret["secret"].string == nil && secret["recovery_codes"].array.isEmpty {
            SecureField("验证当前密码", text: $securityPassword)
            if store.user["totp_enabled"].boolean {
              TextField("当前动态码或恢复码", text: $code)
              Button("关闭两步验证", role: .destructive) {
                action.send("/auth/totp/disable", body: .object(["current_password": .string(securityPassword), "totp": .string(code)]), store: store) { _ in
                  var user = store.user.object
                  user["totp_enabled"] = .bool(false)
                  store.user = .object(user)
                  securityPassword = ""
                  code = ""
                }
              }.disabled(securityPassword.isEmpty || code.isEmpty)
            } else {
              Button("设置两步验证") {
                action.send("/auth/totp/start", body: .object(["current_password": .string(securityPassword)]), store: store) { secret = $0; securityPassword = "" }
              }.disabled(securityPassword.isEmpty)
            }
          }
          if let value = secret["secret"].string {
            Text("在验证器中添加此密钥，然后填写当前验证码。请在 10 分钟内完成。").font(.caption).foregroundStyle(.secondary)
            Text(value).font(.system(.body, design: .monospaced)).textSelection(.enabled)
            TextField("六位验证码", text: $code)
            Button("确认启用") {
              action.send(
                "/auth/totp/confirm", body: .object(["code": .string(code)]), store: store
              ) { value in
                secret = value
                code = ""
                var user = store.user.object
                user["totp_enabled"] = .bool(true)
                store.user = .object(user)
              }
            }.disabled(code.count != 6 || !code.allSatisfy(\.isNumber))
          }
          if !secret["recovery_codes"].array.isEmpty {
            Text("恢复码仅显示一次，请保存。").font(.caption)
            Text(secret["recovery_codes"].array.map(\.text).joined(separator: "\n")).font(
              .system(.body, design: .monospaced)
            ).textSelection(.enabled)
            Button("已保存恢复码") { secret = .null }
          }
        }
      }
      Button("刷新状态") { refresh = UUID() }
    }.task(id: "\(store.sessionID):\(tab):\(refresh)") {
      let owner = store.sessionID
      if let account = try? await store.api("/auth/me"), owner == store.sessionID { store.user = account }
      guard owner == store.sessionID else { return }
      if tab == "sessions" { await sessions.load("/auth/sessions", store: store) }
      if tab == "tokens" { await tokens.load("/auth/tokens", store: store) }
      if tab == "users" { await users.load("/auth/users", store: store) }
    }
    .onChange(of: tab) {
      password = ""
      securityPassword = ""
      name = ""
      code = ""
      secret = .null
    }
    .onDisappear {
      action.cancel()
      securityPassword = ""
      password = ""
      secret = .null
      code = ""
    }
    .confirmationDialog(
      "撤销所选访问权限？",
      isPresented: Binding(
        get: { confirmation["path"].string != nil }, set: { if !$0 { confirmation = .null } }),
      titleVisibility: .visible
    ) {
      Button("确认注销", role: .destructive) {
        let target = confirmation
        confirmation = .null
        action.send(target["path"].text, method: "DELETE", store: store) { _ in
          refresh = UUID()
          if target["current"].boolean { Task { await store.disconnect() } }
        }
      }
    }
  }
  private var admin: Bool { store.user["role"].text == "admin" }
  private func date(_ value: JSONValue) -> String {
    Date(timeIntervalSince1970: value.number).formatted(date: .numeric, time: .shortened)
  }
}

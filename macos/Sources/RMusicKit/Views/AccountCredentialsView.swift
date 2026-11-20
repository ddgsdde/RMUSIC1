import SwiftUI

struct AccountCredentialsView: View {
  let store: MusicStore
  @StoredState private var action = WorkflowAction()
  @StoredState private var username = ""
  @StoredState private var currentPassword = ""
  @StoredState private var newPassword = ""
  @StoredState private var confirmation = ""
  @StoredState private var code = ""
  @StoredState private var validation = ""

  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      WorkflowNotice(action: action)
      FeatureCard {
        Text("账号与密码").font(.headline)
        Text("修改后保留当前登录，其他设备需重新登录，外部访问凭证也将撤销。")
          .font(.caption).foregroundStyle(.secondary)
        if store.user["role"].text == "admin" {
          TextField("登录账号", text: $username)
        } else {
          Text(store.user["username"].text).foregroundStyle(.secondary)
        }
        SecureField("当前密码", text: $currentPassword)
        SecureField("新密码（至少 10 个字符）", text: $newPassword)
        SecureField("确认新密码", text: $confirmation)
        if store.user["totp_enabled"].boolean {
          TextField("未使用的动态码或恢复码", text: $code)
        }
        if !validation.isEmpty { Text(validation).foregroundStyle(.red) }
        Button("保存账号与密码") {
          guard newPassword == confirmation else { validation = "两次输入的新密码不一致"; return }
          validation = ""
          var body: [String: JSONValue] = ["current_password": .string(currentPassword)]
          if !newPassword.isEmpty { body["new_password"] = .string(newPassword) }
          if !code.isEmpty { body["totp"] = .string(code) }
          if store.user["role"].text == "admin" && username != store.user["username"].text {
            body["username"] = .string(username)
          }
          action.send("/auth/account", method: "PATCH", body: .object(body), store: store) { value in
            store.user = value["user"]
            store.username = value["user"]["username"].text
            UserDefaults.standard.set(store.username, forKey: "username")
            username = value["user"]["username"].text
            currentPassword = ""
            newPassword = ""
            confirmation = ""
            code = ""
          }
        }.disabled(currentPassword.isEmpty || (!newPassword.isEmpty && newPassword.utf8.count < 10) ||
          (newPassword.isEmpty && username == store.user["username"].text))
      }
    }
    .onAppear { username = store.user["username"].text }
    .onDisappear { action.cancel(); currentPassword = ""; newPassword = ""; confirmation = ""; code = "" }
  }
}

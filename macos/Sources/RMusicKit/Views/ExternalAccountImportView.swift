import SwiftUI
import UniformTypeIdentifiers

struct ExternalAccountImportView: View {
  let store: MusicStore
  let initial: JSONValue
  let accounts: [JSONValue]
  let plugins: [JSONValue]
  let close: () -> Void
  @StoredState private var action = WorkflowAction()
  @StoredState private var job: JSONValue = .null
  @StoredState private var choosing = false
  @StoredState private var file: URL?
  @StoredState private var name = ""
  @StoredState private var password = ""
  @StoredState private var current = ""
  @StoredState private var code = ""
  @StoredState private var merge = ""
  @StoredState private var providers: Set<String> = []
  @StoredState private var cache = false
  @StoredState private var library = false
  @StoredState private var bandwidth = "0"
  private var uploaded: Bool { ["uploaded", "interrupted"].contains(job["state"].text) }
  private var running: Bool { ["applying", "uploading"].contains(job["state"].text) }
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      HStack { Text("导入账号数据").font(.title2.bold()); Spacer(); Button("关闭", action: close).disabled(action.busy) }
      ScrollView {
        VStack(alignment: .leading, spacing: 16) {
          if job["id"].text.isEmpty || ["created", "invalid"].contains(job["state"].text) {
            Button(file?.lastPathComponent ?? "选择 R-Music 账号档案") { choosing = true }
            Text("支持不超过 2 GiB 的 JSONL 档案。先上传并校验，再选择合并或新建账号。")
              .font(.caption).foregroundStyle(.secondary)
          }
          if !job["id"].text.isEmpty {
            Text("\(job["manifest"]["username"].text) · \(job["state"].text)").font(.headline)
            Text("\(job["applied"].number ?? 0, specifier: "%.0f") / \(job["records"].number ?? 0, specifier: "%.0f") 条资料")
            if !job["error"].text.isEmpty { Text(job["error"].text).foregroundStyle(.orange) }
          }
          if uploaded && job["target_user"].text.isEmpty {
            ThemePicker("导入方式", selection: $merge) {
              Text("按新用户导入").tag("")
              ForEach(Array(accounts.enumerated()), id: \.offset) { _, account in Text("合并到 \(account["username"].text)").tag(account["id"].text) }
            }
            if !job["merge_candidates"].array.isEmpty {
              Text("发现相同来源账号：\(job["merge_candidates"].array.map { $0["username"].text }.joined(separator: "、"))。请选择合并，或设置新的用户名。")
                .font(.caption).foregroundStyle(.secondary)
            }
            if merge.isEmpty {
              TextField("新账号用户名", text: $name)
              SecureField("初始密码（至少 10 位）", text: $password)
              ExternalPolicyFields(plugins: plugins, providers: $providers, cache: $cache, library: $library, bandwidth: $bandwidth)
            }
          }
          if !running && job["state"].text != "completed" {
            SecureField("当前管理员密码", text: $current)
            TextField("两步验证码或恢复码（已启用时填写）", text: $code)
            Button(uploaded ? "确认导入／继续" : "上传并预览") { submit() }
              .buttonStyle(.borderedProminent).disabled(action.busy || current.isEmpty || !uploaded && file == nil)
          }
          if !job["id"].text.isEmpty && !["applying", "interrupted"].contains(job["state"].text) {
            Button("移除此导入任务", role: .destructive) {
              action.send("/auth/account-imports/\(job["id"].text.pathComponentEncoded)", method: "DELETE", store: store) { _ in close() }
            }.disabled(action.busy)
          }
          WorkflowNotice(action: action)
        }.padding(2)
      }
    }.padding(24).frame(minWidth: 560, idealWidth: 640, minHeight: 520, idealHeight: 720)
      .interactiveDismissDisabled(action.busy)
      .task {
        job = initial
        if !initial["id"].text.isEmpty {
          do { job = try await store.api("/auth/account-imports/\(initial["id"].text.pathComponentEncoded)"); name = job["manifest"]["username"].text; merge = job["target_user"].text }
          catch { await store.handle(error) }
        }
      }
      .task(id: "\(job["id"].text):\(job["state"].text)") {
        guard running else { return }; let owner = store.sessionID
        while !Task.isCancelled && owner == store.sessionID && running {
          do { try await Task.sleep(for: .milliseconds(750)); job = try await store.api("/auth/account-imports/\(job["id"].text.pathComponentEncoded)") }
          catch is CancellationError { return } catch { action.error = error.localizedDescription; return }
        }
      }
      .fileImporter(isPresented: $choosing, allowedContentTypes: [.data, .plainText], allowsMultipleSelection: false) { result in
        do { file = try result.get().first } catch { action.error = error.localizedDescription }
      }
      .onChange(of: action.busy) { if !action.busy { current = ""; code = ""; password = "" } }
      .onDisappear { action.cancel(); current = ""; code = ""; password = "" }
  }
  private func submit() {
    var fields: [String: JSONValue] = ["current_password": .string(current)]
    if !code.isEmpty { fields["totp"] = .string(code) }
    if uploaded {
      let target = job["target_user"].text.isEmpty ? merge : job["target_user"].text
      fields["mode"] = .string(target.isEmpty ? "new" : "merge")
      if target.isEmpty {
        guard !name.trimmingCharacters(in: .whitespaces).isEmpty, password.count >= 10 else { action.error = "请输入新账号用户名和至少 10 位密码。"; return }
        fields["username"] = .string(name.trimmingCharacters(in: .whitespaces)); fields["password"] = .string(password)
        do { fields["policy"] = try ExternalPolicyFields.value(providers, cache, library, bandwidth) } catch { action.error = error.localizedDescription; return }
      } else { fields["target_user"] = .string(target) }
      action.send("/auth/account-imports/\(job["id"].text.pathComponentEncoded)/apply", body: .object(fields), store: store) { job = $0 }
    } else if let file {
      let credentials = JSONValue.object(fields), prior = job
      action.run(store: store, operation: {
        let created: JSONValue
        if prior["id"].text.isEmpty {
          created = try await store.api("/auth/account-imports", method: "POST", body: credentials)
        } else {
          created = try await store.api("/auth/account-imports/\(prior["id"].text.pathComponentEncoded)/authorize-upload", method: "POST", body: credentials)
        }
        let opened = file.startAccessingSecurityScopedResource()
        defer { if opened { file.stopAccessingSecurityScopedResource() } }
        return try await store.runtimeCall("import_account_file", .object(["job_id": created["id"], "path": .string(file.path)]))
      }) { job = $0; name = job["manifest"]["username"].text; self.file = nil }
    }
  }
}

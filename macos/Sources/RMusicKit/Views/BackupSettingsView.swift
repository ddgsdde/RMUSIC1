import SwiftUI

struct BackupSettingsView: View {
  let store: MusicStore
  @StoredState private var policy = ResourceStore()
  @StoredState private var history = ResourceStore()
  @StoredState private var health = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var export = FileExportAction()
  @StoredState private var directory = ""
  @StoredState private var enabled = false
  @StoredState private var interval = 24
  @StoredState private var keep = 7
  @StoredState private var drafted = false
  @StoredState private var page = 0
  @StoredState private var refresh = UUID()
  @StoredState private var healthRefresh = UUID()
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      FeatureCard {
        FeatureHeader(title: "备份与恢复", loading: policy.loading) {
          drafted = false
          refresh = UUID()
        }
        ResourceNotice(resource: policy)
        WorkflowNotice(action: action)
        Text("备份含登录凭据且未加密，请保存在私有目录。音乐文件和插件程序需另行保管。").font(.caption).foregroundStyle(.secondary)
        CoreDirectoryField(store: store, path: $directory)
        Toggle("定时备份", isOn: $enabled).toggleStyle(.switch)
        HStack {
          Stepper("每 \(interval) 小时", value: $interval, in: 1...720)
          Spacer()
          Stepper("保留 \(keep) 份自动备份", value: $keep, in: 1...90)
        }
        HStack {
          Button("保存设置") {
            action.send(
              "/maintenance/settings", method: "PUT",
              body: .object([
                "directory": .string(directory), "enabled": .bool(enabled),
                "interval_hours": .number(Double(interval)), "keep": .number(Double(keep)),
              ]), store: store
            ) { policy.value = $0 }
          }.disabled(directory.isEmpty)
          Button("立即备份") {
            action.send("/maintenance/backups", store: store) { _ in refresh = UUID() }
          }.disabled(
            history.value["items"].array.contains {
              ["pending", "running"].contains($0["status"].text)
            })
        }.disabled(action.busy)
        DisclosureGroup("恢复步骤") {
          Text(
            "先执行恢复演练，再导出备份。恢复需停止目标 Core，用随 Core 发布的 backup.py 恢复到新数据目录；原目录保留。切换数据卷后重装插件并检查音乐目录，输出需手动播放。"
          ).font(.caption).textSelection(.enabled)
        }
      }
      ResourceNotice(resource: history)
      if let error = export.error { Text(error).font(.caption).foregroundStyle(.orange) }
      ForEach(
        Array(history.value["items"].array.dropFirst(page * 5).prefix(5).enumerated()), id: \.offset
      ) { _, item in
        FeatureCard {
          HStack {
            Text(item["automatic"].boolean ? "自动备份" : "手动备份").font(.headline)
            Spacer()
            Text(status(item["status"].text)).foregroundStyle(.secondary)
          }
          Text(
            "\(Date(timeIntervalSince1970: item["created_at"].number).formatted()) · \(bytes(item["size"]))"
          ).font(.caption).foregroundStyle(.secondary)
          if let error = item["error"].string {
            Text(error).font(.caption).foregroundStyle(.orange)
          }
          if item["verified_at"].number > 0 {
            Label(
              item["verified_ok"].boolean ? "恢复演练通过" : "恢复演练失败",
              systemImage: item["verified_ok"].boolean
                ? "checkmark.circle" : "exclamationmark.circle")
            if !item["verified_ok"].boolean {
              Text(item["verification_message"].text).font(.caption)
            }
          }
          if item["status"].text == "completed" {
            HStack {
              Button("恢复演练") {
                action.send(
                  "/maintenance/backups/\(item["id"].text.pathComponentEncoded)/verify",
                  store: store
                ) { value in
                  action.notice = value["message"].text
                  refresh = UUID()
                }
              }
              Button(export.busy ? "正在保存…" : "导出备份") {
                export.save(
                  "/maintenance/backups/\(item["id"].text.pathComponentEncoded)/file",
                  name: "rmusic-backup-\(item["id"].text).zip", store: store)
              }.disabled(export.busy)
            }.disabled(action.busy)
          }
          DisclosureGroup("Core 文件位置") {
            Text(item["path"].text).font(.caption).textSelection(.enabled)
          }
        }
      }
      OffsetPageControls(
        offset: Binding(get: { page * 5 }, set: { page = $0 / 5 }),
        total: history.value["items"].array.count, limit: 5)
      FeatureCard {
        FeatureHeader(title: "Core 存储与运行") {
          healthRefresh = UUID()
        }
        ResourceNotice(resource: health)
        if !health.value.object.isEmpty {
          Fact(name: "可用空间", value: bytes(health.value["storage"]["available_bytes"]))
          Fact(name: "Core 内存", value: bytes(health.value["resources"]["process_rss_bytes"]))
          Fact(name: "后台任务", value: String(health.value["resources"]["active_tasks"].integer))
          ForEach(Array(health.value["storage"]["roots"].array.enumerated()), id: \.offset) {
            _, root in
            Label(
              root["path"].text,
              systemImage: root["online"].boolean
                ? "externaldrive.badge.checkmark" : "exclamationmark.triangle"
            ).font(.caption)
          }
        }
      }
    }.task(id: "\(store.sessionID):\(refresh)") {
      if await policy.load("/maintenance/settings", store: store), !drafted {
        let value = policy.value
        directory = value["directory"].text
        enabled = value["enabled"].boolean
        interval = max(1, value["interval_hours"].integer)
        keep = max(1, value["keep"].integer)
        drafted = true
      }
      repeat {
        await history.load("/maintenance/backups", store: store)
        do { try await Task.sleep(for: .seconds(5)) } catch { return }
      } while !Task.isCancelled
    }.task(id: "\(store.sessionID):\(refresh):\(healthRefresh)") { await health.load("/maintenance/health", store: store) }.onDisappear {
      action.cancel()
      export.cancel()
    }
  }
  private func bytes(_ value: JSONValue) -> String {
    guard value.number.isFinite, case .number = value else { return "未报告" }
    return ByteCountFormatter.string(
      fromByteCount: Int64(max(0, min(Double(Int64.max / 2), value.number))), countStyle: .file)
  }
  private func status(_ value: String) -> String {
    [
      "pending": "排队中", "running": "创建并校验", "completed": "已完成", "failed": "失败", "missing": "文件缺失",
      "expired": "已过期",
    ][value] ?? value
  }
}

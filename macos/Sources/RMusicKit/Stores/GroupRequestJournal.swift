import Darwin
import Foundation
import Observation

/// Persist before sending. Switching users or restarting must never invent a second request UUID.
@MainActor @Observable final class GroupRequestJournal {
  var pending: GroupRequest?
  @ObservationIgnored private var owner = ""
  @ObservationIgnored private let file: URL
  init(directory: URL? = nil) {
    let root =
      directory
      ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
      .appendingPathComponent(Bundle.main.bundleIdentifier ?? "com.rmusic.mac.development")
    file = root.appendingPathComponent("PendingActions/groups.json")
  }
  static func scope(_ store: MusicStore) -> String? {
    guard store.connected, let core = store.profile.core_id, !core.isEmpty,
      let user = store.user["id"].string, !user.isEmpty
    else { return nil }
    return core + ":" + user
  }
  func load(owner: String) throws {
    self.owner = owner
    pending = nil
    pending = try read()[owner]
  }
  func prepare(_ request: GroupRequest) throws {
    guard !owner.isEmpty, pending == nil, request.valid else {
      throw RuntimeFailure(message: "请先确认已有分组请求。")
    }
    try locked {
      var records = try read()
      guard records[owner] == nil, records.count < 32 else {
        throw RuntimeFailure(message: "尚有未确认的分组请求，请先处理。")
      }
      records[owner] = request
      try write(records)
    }
    pending = request
  }
  func clear(confirmed requestID: String) throws {
    guard pending?.requestID == requestID else { return }
    try locked {
      var records = try read()
      guard records[owner]?.requestID == requestID else {
        throw RuntimeFailure(message: "分组请求记录已变化，请重新读取。")
      }
      records.removeValue(forKey: owner)
      try write(records)
    }
    pending = nil
  }
  private func locked(_ operation: () throws -> Void) throws {
    let directory = file.deletingLastPathComponent()
    try FileManager.default.createDirectory(
      at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    let fd = open(
      directory.appendingPathComponent("groups.lock").path, O_CREAT | O_RDWR | O_CLOEXEC, 0o600)
    guard fd >= 0 else { throw RuntimeFailure(message: "无法锁定分组请求记录。") }
    defer { close(fd) }
    guard flock(fd, LOCK_EX | LOCK_NB) == 0 else {
      throw RuntimeFailure(message: "另一窗口正在提交分组请求，请稍后重试。")
    }
    defer { flock(fd, LOCK_UN) }
    try operation()
  }
  private func read() throws -> [String: GroupRequest] {
    guard FileManager.default.fileExists(atPath: file.path) else { return [:] }
    let handle = try FileHandle(forReadingFrom: file)
    defer { try? handle.close() }
    let data = try handle.read(upToCount: 8 * 1024 * 1024 + 1) ?? Data()
    guard data.count <= 8 * 1024 * 1024 else { throw RuntimeFailure(message: "未确认请求记录过大，已停止操作。") }
    let records = try JSONDecoder().decode([String: GroupRequest].self, from: data)
    guard records.count <= 32, records.values.allSatisfy(\.valid) else {
      throw RuntimeFailure(message: "未确认请求记录无效，已停止操作。")
    }
    return records
  }
  private func write(_ records: [String: GroupRequest]) throws {
    let data = try JSONEncoder().encode(records)
    guard data.count <= 8 * 1024 * 1024 else { throw RuntimeFailure(message: "未确认请求记录过大。") }
    let fm = FileManager.default
    let directory = file.deletingLastPathComponent()
    try fm.createDirectory(
      at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    let temporary = directory.appendingPathComponent(".groups-\(UUID().uuidString).tmp")
    guard
      fm.createFile(atPath: temporary.path, contents: data, attributes: [.posixPermissions: 0o600])
    else { throw RuntimeFailure(message: "无法保存分组请求，尚未发送。") }
    defer { try? fm.removeItem(at: temporary) }
    let handle = try FileHandle(forWritingTo: temporary)
    defer { try? handle.close() }
    try handle.synchronize()
    guard rename(temporary.path, file.path) == 0 else {
      throw RuntimeFailure(message: "无法提交分组请求记录，尚未发送。")
    }
  }
}

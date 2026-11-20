import AppKit
import Foundation

/// A native save panel chooses a destination; the runtime streams Core artifacts to that file.
@MainActor enum CoreFileService {
  static func save(_ path: String, name: String, store: MusicStore) async throws {
    let owner = store.sessionID
    let panel = NSSavePanel()
    panel.nameFieldStringValue = String(name.map { "/\\:".contains($0) ? "_" : $0 }.prefix(200))
    panel.canCreateDirectories = true
    try Task.checkCancellation()
    let url: URL? = await withTaskCancellationHandler {
      await withCheckedContinuation { continuation in
        let completion: (NSApplication.ModalResponse) -> Void = { response in
          continuation.resume(returning: response == .OK ? panel.url : nil)
        }
        if let window = NSApp.mainWindow ?? NSApp.keyWindow {
          panel.beginSheetModal(for: window, completionHandler: completion)
        } else {
          panel.begin(completionHandler: completion)
        }
      }
    } onCancel: {
      Task { @MainActor in panel.cancel(nil) }
    }
    try Task.checkCancellation()
    guard owner == store.sessionID, let url else { throw CancellationError() }
    _ = try await store.runtimeCall(
      "save_api_file",
      .object([
        "path": .string(path), "directory": .string(url.deletingLastPathComponent().path),
        "filename": .string(url.lastPathComponent),
      ]))
    try Task.checkCancellation()
    if owner == store.sessionID { store.notice = "文件已保存。" }
  }
  static func playlistText(_ url: URL) async throws -> String {
    try await Task.detached(priority: .userInitiated) {
      let opened = url.startAccessingSecurityScopedResource()
      defer { if opened { url.stopAccessingSecurityScopedResource() } }
      let file = try FileHandle(forReadingFrom: url)
      defer { try? file.close() }
      let data = try file.read(upToCount: 2 * 1024 * 1024 + 1) ?? Data()
      guard data.count <= 2 * 1024 * 1024, let text = String(data: data, encoding: .utf8) else {
        throw RuntimeFailure(message: "播放列表须为 UTF-8 文本且不超过 2 MiB。")
      }
      return text
    }.value
  }
}

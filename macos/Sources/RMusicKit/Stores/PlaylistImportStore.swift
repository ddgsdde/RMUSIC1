import Foundation
import Observation

/// Only Core references are imported. Candidate pages are discarded; at most 10k selected IDs remain.
@MainActor @Observable final class PlaylistImportStore {
  var content = ""
  var format = "json"
  var total = 0
  var exact = 0
  var review = 0
  var missing = 0
  var rows: [JSONValue] = []
  var selected: [Int: String] = [:]
  var loading = false
  var ready = false
  var error: String?
  var progress = 0
  @ObservationIgnored private var revision = UUID()
  @ObservationIgnored private var lastBody = Data()
  @ObservationIgnored private var requestID = UUID().uuidString
  func scan(_ url: URL, store: MusicStore) async {
    let owner = store.sessionID
    let request = UUID()
    revision = request
    loading = true
    ready = false
    error = nil
    rows = []
    selected = [:]
    total = 0
    progress = 0
    defer { if revision == request { loading = false } }
    do {
      let extensionName = url.pathExtension.lowercased()
      guard ["json", "csv", "m3u", "m3u8", "pls"].contains(extensionName) else {
        throw RuntimeFailure(message: "请选择 M3U、M3U8、PLS、CSV 或 R-Music JSON 播放列表。")
      }
      let text = try await CoreFileService.playlistText(url)
      var choices: [Int: String] = [:]
      var checked = 0
      var expected: Int?
      var first: [JSONValue] = []
      var matched = 0
      var needsReview = 0
      var notFound = 0
      repeat {
        try Task.checkCancellation()
        let result = try await store.api(
          "/playlists/import/preview", method: "POST",
          body: .object([
            "format": .string(extensionName), "content": .string(text),
            "offset": .number(Double(checked)), "limit": .number(100),
          ]))
        let count = result["total"].integer
        let page = result["items"].array
        guard (1...10000).contains(count), expected == nil || expected == count, !page.isEmpty,
          page.count <= 100, checked + page.count <= count
        else { throw RuntimeFailure(message: "播放列表分页发生变化，未提交导入，请重新预览。") }
        guard owner == store.sessionID, revision == request else { throw CancellationError() }
        expected = count
        for (index, item) in page.enumerated() {
          if item["status"].text == "matched", let id = item["matched"]["id"].string {
            choices[checked + index] = id
            matched += 1
          } else if item["status"].text == "review" {
            needsReview += 1
          } else {
            notFound += 1
          }
        }
        if first.isEmpty { first = Array(page.prefix(50)) }
        checked += page.count
        progress = checked
      } while checked < (expected ?? 0)
      try Task.checkCancellation()
      guard owner == store.sessionID, revision == request else { return }
      content = text
      format = extensionName
      total = checked
      rows = first
      selected = choices
      exact = matched
      review = needsReview
      missing = notFound
      ready = true
    } catch is CancellationError {} catch {
      if owner == store.sessionID, revision == request { self.error = error.localizedDescription }
    }
  }
  func page(_ offset: Int, store: MusicStore) async {
    guard ready else { return }
    let owner = store.sessionID
    let request = revision
    loading = true
    defer { if revision == request { loading = false } }
    do {
      let result = try await store.api(
        "/playlists/import/preview", method: "POST",
        body: .object([
          "format": .string(format), "content": .string(content), "offset": .number(Double(offset)),
          "limit": .number(50),
        ]))
      try Task.checkCancellation()
      guard owner == store.sessionID, revision == request else { return }
      guard result["total"].integer == total else {
        throw RuntimeFailure(message: "曲库匹配已变化，请重新预览。")
      }
      rows = result["items"].array
      error = nil
    } catch is CancellationError {} catch {
      if owner == store.sessionID, revision == request { self.error = error.localizedDescription }
    }
  }
  func payload(name: String) throws -> JSONValue {
    guard ready, !loading, !name.trimmingCharacters(in: .whitespaces).isEmpty, !selected.isEmpty,
      selected.count <= 10000
    else { throw RuntimeFailure(message: "请完成匹配预览并选择歌曲。") }
    let ids = selected.keys.sorted().compactMap { selected[$0] }
    let body: [String: JSONValue] = [
      "name": .string(name), "track_ids": .array(ids.map(JSONValue.string)),
    ]
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    let encoded = try encoder.encode(JSONValue.object(body))
    if encoded != lastBody {
      lastBody = encoded
      requestID = UUID().uuidString
    }
    return .object(body.merging(["request_id": .string(requestID)]) { _, new in new })
  }
  func reset() {
    revision = UUID()
    content = ""
    rows = []
    selected = [:]
    ready = false
    lastBody = Data()
  }
}

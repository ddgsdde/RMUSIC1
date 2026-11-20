import Foundation

struct PlaylistDraft {
  var name = ""
  var ids: [String] = []
  var originalName = ""
  var originalIDs: [String] = []
  mutating func load(_ playlist: Playlist) {
    name = playlist.name
    ids = playlist.track_ids
    originalName = name
    originalIDs = ids
  }
  mutating func move(_ index: Int, by distance: Int) {
    let target = index + distance
    guard ids.indices.contains(index), ids.indices.contains(target) else { return }
    ids.swapAt(index, target)
  }
  func payload(latest: Playlist) throws -> JSONValue {
    let clean = name.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !clean.isEmpty, clean.utf8.count <= 100, ids.count <= 10000 else {
      throw RuntimeFailure(message: "歌单名称为空或过长；歌曲最多一万首。")
    }
    guard latest.name == originalName, latest.track_ids == originalIDs else {
      throw RuntimeFailure(message: "歌单已在其他位置修改；当前编辑保留，请读取最新歌单后重新编辑。", status: 409)
    }
    return .object([
      "name": .string(clean), "track_ids": .array(ids.map(JSONValue.string)),
      "expected": .object([
        "name": .string(originalName), "track_ids": .array(originalIDs.map(JSONValue.string)),
      ]),
    ])
  }
}

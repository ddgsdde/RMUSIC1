import Foundation

struct IdentityAssertion: Identifiable {
  let id = UUID()
  var kind = "recording"
  var namespace = "rmusic-manual"
  var externalID = UUID().uuidString.lowercased()
  var title = ""
  var role = "recording"
  var position: JSONValue = .null
  init() {}
  init(_ value: JSONValue) {
    kind = value["kind"].text
    namespace = value["namespace"].text
    externalID = value["external_id"].text
    title = value["title"].text
    role = value["role"].text
    position = value["position"]
  }
  var value: JSONValue {
    var fields: [String: JSONValue] = [
      "kind": .string(kind), "namespace": .string(namespace), "external_id": .string(externalID),
      "title": .string(title), "role": .string(role),
    ]
    if case .number = position { fields["position"] = position }
    return .object(fields)
  }
  static let kinds = [("work", "作品"), ("recording", "录音"), ("release", "发行"), ("artist", "参与者")]
  static let roles = [
    ("performer", "演唱／演奏"), ("album_artist", "专辑艺术家"), ("composer", "作曲"), ("conductor", "指挥"),
    ("lyricist", "作词"), ("producer", "制作"), ("engineer", "录音工程"),
  ]
  static func manual(_ value: JSONValue) -> [Self] {
    value["entities"].array.filter { $0["basis"].text == "manual" }.map(Self.init)
  }
}

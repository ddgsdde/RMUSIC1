import Foundation

enum PlayerModule: String, Codable, CaseIterable, Identifiable {
  case cover, info, controls, lyrics, queue, zones, signal, meter
  var id: String { rawValue }
  var title: String {
    switch self {
    case .cover: "封面"
    case .info: "歌曲资料"
    case .controls: "播放控制"
    case .lyrics: "歌词"
    case .queue: "播放队列"
    case .zones: "音区"
    case .signal: "音频链路"
    case .meter: "输出电平"
    }
  }
}
struct PlayerLayout: Codable, Equatable {
  struct Item: Codable, Equatable, Identifiable {
    let module: PlayerModule
    var visible: Bool
    var column: Int
    var height: Double
    var id: String { module.id }
  }
  var items: [Item]
  static var standard: Self {
    Self(
      items: PlayerModule.allCases.map {
        Item(
          module: $0, visible: [.cover, .info, .lyrics, .controls].contains($0),
          column: $0 == .lyrics ? 1 : 0, height: $0 == .lyrics ? 600 : $0 == .cover ? 280 : 180)
      })
  }
  static var defaultJSON: String { standard.encoded }
  static func read(_ raw: String) -> Self {
    guard raw.utf8.count <= 8192, let data = raw.data(using: .utf8),
      let decoded = try? JSONDecoder().decode(Self.self, from: data),
      decoded.items.count == PlayerModule.allCases.count,
      Set(decoded.items.map(\.module)).count == PlayerModule.allCases.count,
      decoded.items.allSatisfy({
        (0...1).contains($0.column) && $0.height.isFinite && (120...800).contains($0.height)
      })
    else { return .standard }
    return decoded
  }
  var encoded: String {
    String(data: (try? JSONEncoder().encode(self)) ?? Data(), encoding: .utf8) ?? ""
  }
  mutating func move(_ index: Int, by distance: Int) {
    guard items.indices.contains(index), items.indices.contains(index + distance) else { return }
    items.swapAt(index, index + distance)
  }
}

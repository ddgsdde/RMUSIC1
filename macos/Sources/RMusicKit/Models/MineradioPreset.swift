import Foundation

struct MineradioPreset: Identifiable {
    let id: Int; let name: String
    static let all: [Self] = [
        .init(id: 0, name: "emily · 封面粒子"), .init(id: 1, name: "滚筒"), .init(id: 2, name: "星球"),
        .init(id: 3, name: "虚空"), .init(id: 4, name: "唱片"), .init(id: 5, name: "星河"),
        .init(id: 6, name: "安魂"), .init(id: 7, name: "音域回响 · Sonic Topography"),
        .init(id: 8, name: "音域回响 · Workshop"),
        .init(id: 9, name: "月蚀圣环"), .init(id: 10, name: "雨幕霓虹"), .init(id: 11, name: "折光蝶群"), .init(id: 12, name: "深海绽放")
    ]
}
struct VisualFeature: Codable, Sendable, Equatable {
    let playback_id: String
    let position: Double
    let bands: [Int]
    let energy: Int
    let peak: Int
    var valid: Bool { position.isFinite && position >= 0 && bands.count == 8 && (bands + [energy, peak]).allSatisfy { (0...255).contains($0) } }
    var dictionary: [String: Any] { ["playback_id": playback_id, "position": position, "bands": bands, "energy": energy, "peak": peak] }
}

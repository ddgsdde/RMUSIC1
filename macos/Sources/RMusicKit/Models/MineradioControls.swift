import Foundation
import RMusicVisualAssets

/// The bounds, labels and defaults are extracted from the pinned original control panel.
struct MineradioControl: Decodable, Identifiable {
    struct Choice: Decodable, Identifiable { let id: String; let label: String }
    let id: String; let tab: String; let group: String; let label: String; let kind: String
    let number: Double?; let minimum: Double?; let maximum: Double?; let step: Double?
    let flag: Bool?; let text: String?; let choices: [Choice]?
    static let all: [Self] = {
        guard let url = RMusicVisualAssets.directory?.appendingPathComponent("console.json"), let data = try? Data(contentsOf: url) else { return [] }
        return (try? JSONDecoder().decode([Self].self, from: data)) ?? []
    }()
}

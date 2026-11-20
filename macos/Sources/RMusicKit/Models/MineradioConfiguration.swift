import Foundation

struct MineradioConfiguration: Equatable {
    var preset: Int; var fps: Int; var particles: Int; var scale: Double; var intensity: Double; var active: Bool; var immersive: Bool; var shelf: Bool = false; var visible: Bool = true; var floatLayer: Bool = false; var backCover: Bool = false; var parameters: String = "{}"; var stageTop: Double = 0; var stageBottom: Double = 0; var motion: Bool = true
    var dictionary: [String: Any] { ["preset": preset, "motion": motion, "fps": fps, "particles": particles, "scale": scale, "intensity": intensity, "immersive": immersive, "shelf": shelf, "visible": visible, "floatLayer": floatLayer, "backCover": backCover, "parameters": parameters, "stageTop": stageTop, "stageBottom": stageBottom, "diagnostics": Bundle.main.bundleIdentifier?.hasPrefix("com.rmusic.mac.client.qa") == true] }
}

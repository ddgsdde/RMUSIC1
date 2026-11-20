import SwiftUI
import Observation

enum MusicTheme: String, CaseIterable, Identifiable {
    case coral, jade, ocean, violet, monochrome, amber, rose, graphite
    var id: String { rawValue }
    var name: String { switch self { case .coral: "珊瑚"; case .jade: "青玉"; case .ocean: "海蓝"; case .violet: "暮紫"; case .monochrome: "黑白"; case .amber: "琥珀"; case .rose: "烟玫瑰"; case .graphite: "石墨" } }
    func canvas(_ scheme: ColorScheme) -> Color {
        if self == .monochrome { return scheme == .dark ? .black : .white }
        let rgb: (Double, Double, Double)
        switch (self, scheme) {
        case (.coral, .light): rgb = (0.975, 0.970, 0.958)
        case (.amber, .light): rgb = (0.975, 0.953, 0.903)
        case (.rose, .light): rgb = (0.979, 0.950, 0.945)
        case (.graphite, .light): rgb = (0.946, 0.951, 0.952)
        case (.amber, .dark): rgb = (0.095, 0.077, 0.052)
        case (.rose, .dark): rgb = (0.096, 0.066, 0.075)
        case (.graphite, .dark): rgb = (0.054, 0.060, 0.065)
        case (.jade, .light): rgb = (0.958, 0.978, 0.967)
        case (.ocean, .light): rgb = (0.957, 0.974, 0.993)
        case (.violet, .light): rgb = (0.976, 0.963, 0.990)
        case (.jade, .dark): rgb = (0.055, 0.080, 0.068)
        case (.ocean, .dark): rgb = (0.055, 0.070, 0.095)
        case (.violet, .dark): rgb = (0.078, 0.060, 0.095)
        default: rgb = (0.075, 0.068, 0.063)
        }
        return Color(red: rgb.0, green: rgb.1, blue: rgb.2)
    }
    func rail(_ scheme: ColorScheme) -> Color {
        canvas(scheme).opacity(scheme == .dark ? 0.65 : 0.78)
    }
    var accent: Color {
        switch self {
        case .coral: Color(red: 0.91, green: 0.32, blue: 0.20)
        case .jade: Color(red: 0.12, green: 0.59, blue: 0.43)
        case .ocean: Color(red: 0.16, green: 0.46, blue: 0.85)
        case .violet: Color(red: 0.56, green: 0.35, blue: 0.83)
        case .monochrome: Color.primary
        case .amber: Color(red: 0.70, green: 0.48, blue: 0.20)
        case .rose: Color(red: 0.72, green: 0.36, blue: 0.44)
        case .graphite: Color(red: 0.44, green: 0.55, blue: 0.60)
        }
    }
}
@MainActor @Observable
final class AppearanceStore {
    static let shared = AppearanceStore()
    var theme: MusicTheme { didSet { defaults.set(theme.rawValue, forKey: "appearance.theme") } }
    var skin: WorkspaceSkin { didSet { defaults.set(skin.rawValue, forKey: "appearance.skin") } }
    var mode: String { didSet { defaults.set(mode, forKey: "appearance.mode") } }
    @ObservationIgnored private let defaults: UserDefaults
    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        skin = WorkspaceSkin(rawValue: defaults.string(forKey: "appearance.skin") ?? "") ?? .studio
        theme = MusicTheme(rawValue: defaults.string(forKey: "appearance.theme") ?? "") ?? .coral
        mode = ["system", "light", "dark"].first { $0 == defaults.string(forKey: "appearance.mode") } ?? "system"
    }
    var scheme: ColorScheme? { if skin == .mineradio { return .dark }; return mode == "dark" ? .dark : mode == "light" ? .light : nil }
}

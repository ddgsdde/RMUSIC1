import SwiftUI

/// Shared spacing, color and typography for the listening workspace.
@MainActor
enum MusicStyle {
    static var accent: Color { AppearanceStore.shared.theme.accent }
    static var onAccent: Color { AppearanceStore.shared.theme == .monochrome ? (AppearanceStore.shared.skin == .mineradio ? .black : Color(.windowBackgroundColor)) : .white }
    static var sidebarWidth: CGFloat { AppearanceStore.shared.skin.sidebarWidth }
    static func canvas(_ scheme: ColorScheme) -> Color { AppearanceStore.shared.skin == .mineradio ? Color(red: 0.025, green: 0.03, blue: 0.045).opacity(0.50) : AppearanceStore.shared.theme.canvas(scheme) }
    static func rail(_ scheme: ColorScheme) -> Color { AppearanceStore.shared.skin == .mineradio ? Color.black.opacity(0.26) : AppearanceStore.shared.theme.rail(scheme) }
    static func panel(_ scheme: ColorScheme) -> Color { scheme == .dark ? Color.white.opacity(0.045) : Color.white.opacity(0.72) }
}
struct BrandMark: View {
    var compact = false
    var body: some View {
        HStack(spacing: 10) {
            HStack(alignment: .center, spacing: 3) { ForEach(0..<4) { index in Capsule().fill(MusicStyle.accent).frame(width: 4, height: [16.0, 29, 23, 12][index]) } }.rotationEffect(.degrees(12))
            if !compact { Text("R·MUSIC").font(.system(size: 18, weight: .heavy, design: .rounded)).tracking(1) }
        }.accessibilityLabel("R-Music")
    }
}
struct WorkspacePrimaryButton: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.system(size: 13, weight: .semibold)).foregroundStyle(MusicStyle.onAccent)
            .padding(.horizontal, 18).padding(.vertical, 13).background(MusicStyle.accent, in: RoundedRectangle(cornerRadius: 12))
            .opacity(isEnabled ? (configuration.isPressed ? 0.8 : 1) : 0.42).scaleEffect(configuration.isPressed && !reduceMotion ? 0.98 : 1)
            .animation(reduceMotion ? nil : .spring(response: 0.25), value: configuration.isPressed)
    }
}
struct Eyebrow: View {
    let text: String
    var body: some View { Text(text).font(.system(size: 10, weight: .semibold)).tracking(2.5).foregroundStyle(.secondary) }
}

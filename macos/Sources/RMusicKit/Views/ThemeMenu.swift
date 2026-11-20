import SwiftUI
import Observation

@MainActor @Observable final class ThemeMenuPresentation { var showing = false }
struct ThemeMenuAnchor: PreferenceKey {
    static let defaultValue: Anchor<CGRect>? = nil
    static func reduce(value: inout Anchor<CGRect>?, nextValue: () -> Anchor<CGRect>?) { value = nextValue() ?? value }
}

struct ThemeMenu: View {
    @Environment(ThemeMenuPresentation.self) private var presentation
    var body: some View {
        Button { presentation.showing.toggle() } label: { Image(systemName: "circle.lefthalf.filled").frame(width: 34, height: 34).contentShape(Rectangle()) }
            .buttonStyle(.plain).help("主题与显示模式").accessibilityLabel("主题与显示模式")
            .anchorPreference(key: ThemeMenuAnchor.self, value: .bounds) { $0 }
    }
}

/// Resolve the anchor inside this window, so a palette never extends outside its content bounds.
struct ThemeMenuOverlay: View {
    @Bindable var presentation: ThemeMenuPresentation
    let anchor: Anchor<CGRect>?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        GeometryReader { geometry in
            if presentation.showing, let anchor {
                let rect = geometry[anchor]
                let width = min(340, geometry.size.width - 32)
                let height = min(AppearanceStore.shared.skin == .mineradio ? 500 : 550, geometry.size.height - 32)
                Color.black.opacity(0.001).contentShape(Rectangle()).onTapGesture { presentation.showing = false }
                ThemePalettePanel(close: { presentation.showing = false })
                    .frame(width: width, height: height)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 20))
                    .overlay(RoundedRectangle(cornerRadius: 20).stroke(.primary.opacity(0.10)))
                    .shadow(color: .black.opacity(0.25), radius: 24, y: 12)
                    .offset(x: max(16, min(rect.maxX - width, geometry.size.width - width - 16)), y: max(16, min(rect.maxY + 8, geometry.size.height - height - 16)))
                    .transition(.opacity.combined(with: .scale(scale: 0.98, anchor: .topTrailing)))
                    .accessibilityIdentifier("themePalette")
                    .onExitCommand { presentation.showing = false }
            }
        }.animation(reduceMotion ? nil : .easeOut(duration: 0.16), value: presentation.showing)
    }
}

private struct ThemePalettePanel: View {
    @Bindable private var appearance = AppearanceStore.shared
    let close: () -> Void
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack { Text("配色").font(.headline); Spacer(); Button("关闭外观设置", systemImage: "xmark") { close() }.labelStyle(.iconOnly).buttonStyle(MusicControlStyle()) }
                LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 4), spacing: 10) {
                    ForEach(MusicTheme.allCases) { theme in Button { appearance.theme = theme; close() } label: { VStack(spacing: 7) { Circle().fill(theme.accent).frame(width: 26, height: 26).overlay { if appearance.theme == theme { Image(systemName: "checkmark").font(.system(size: 11, weight: .bold)).foregroundStyle(theme == .monochrome ? .black : .white) } }; Text(theme.name).font(.caption) }.frame(maxWidth: .infinity).padding(.vertical, 8) }.buttonStyle(.plain).accessibilityLabel("\(theme.name)配色") }
                }
                Divider(); Text("界面皮肤").font(.headline)
                ForEach(WorkspaceSkin.allCases) { skin in Button { appearance.skin = skin; close() } label: { HStack { Text(skin.name); Spacer(); if appearance.skin == skin { Image(systemName: "checkmark") } }.padding(.vertical, 6).contentShape(Rectangle()) }.buttonStyle(.plain) }
                if appearance.skin != .mineradio {
                    Divider()
                    ThemePicker("显示模式", selection: $appearance.mode) { Text("系统").tag("system"); Text("浅色").tag("light"); Text("深色").tag("dark") }.pickerStyle(.segmented)
                }
            }.padding(22)
        }.scrollIndicators(.hidden)
    }
}

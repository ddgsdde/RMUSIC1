import SwiftUI

struct AppearanceView: View {
    @Bindable private var appearance = AppearanceStore.shared
    var body: some View {
        FeatureCard {
            Text("配色").font(.headline)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 110))], spacing: 12) {
                ForEach(MusicTheme.allCases) { theme in
                    Button { appearance.theme = theme } label: {
                        VStack(spacing: 9) {
                            RoundedRectangle(cornerRadius: 12).fill(theme.accent.gradient).frame(height: 58)
                                .overlay { if appearance.theme == theme { Image(systemName: "checkmark.circle.fill").foregroundStyle(.background).font(.title3) } }
                            Text(theme.name).font(.callout)
                        }.padding(9).background(.primary.opacity(appearance.theme == theme ? 0.06 : 0), in: RoundedRectangle(cornerRadius: 15))
                    }.buttonStyle(.plain).accessibilityLabel("\(theme.name)主题")
                }
            }
            PreferenceChoice("界面皮肤", selection: $appearance.skin) { ForEach(WorkspaceSkin.allCases) { Text($0.name).tag($0) } }
            ListeningAppearanceSettings()
            ThemePicker("显示模式", selection: $appearance.mode) { Text("跟随系统").tag("system"); Text("浅色").tag("light"); Text("深色").tag("dark") }.pickerStyle(.segmented).frame(maxWidth: 360).disabled(appearance.skin == .mineradio).help("Mineradio 使用深色舞台；其他主题沿用此显示模式")
        }.frame(maxWidth: 720, alignment: .leading)
    }
}

import SwiftUI

struct ListeningAppearanceSettings: View {
    @AppStorage("nativePlayerView") private var layout = "record"
    @AppStorage("listeningTheme") private var theme = "silk"
    var body: some View {
        Divider().padding(.vertical, 6)
        Text("沉浸播放").font(.headline)
        if AppearanceStore.shared.skin != .mineradio && theme != "mineradio" { ThemePicker("布局", selection: $layout) { ForEach(ListeningLayout.allCases) { Text($0.name).tag($0.rawValue) } } }
        ThemePicker("播放页皮肤", selection: Binding(get: { AppearanceStore.shared.skin == .mineradio ? "mineradio" : theme }, set: { theme = $0 })) { ForEach(ListeningTheme.allCases) { Text($0.name).tag($0.rawValue) } }.disabled(AppearanceStore.shared.skin == .mineradio).help("Mineradio 主题自动使用舞台皮肤；切回后恢复原来的播放皮肤")
        if AppearanceStore.shared.skin == .mineradio || theme == "mineradio" { MineradioSettings() }
    }
}
struct MineradioSettings: View {
    @StoredState private var console = false
    var body: some View {
        Button("打开视觉控制台", systemImage: "slider.horizontal.3") { console = true }
            .sheet(isPresented: $console) { MineradioConsoleView() }
    }
}

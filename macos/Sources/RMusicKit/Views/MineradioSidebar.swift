import SwiftUI

/// Persistent navigation outside the stage; compact mode keeps every destination accessible.
struct MineradioSidebar: View {
    let store: MusicStore
    let section: String
    let expanded: Bool
    let navigate: (String) -> Void
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 4) {
                ForEach(MineradioDestination.all) { item in
                    if item.id == "outputs" { Divider().overlay(.white.opacity(0.08)).padding(.vertical, 10) }
                    row(item.id, item.name, item.symbol)
                }
                if expanded && !store.collection.playlists.isEmpty {
                    Divider().padding(.vertical, 10)
                    Text("我的播放列表").font(.system(size: 10, weight: .medium)).foregroundStyle(.secondary).padding(.horizontal, 12).padding(.bottom, 6)
                    ForEach(store.collection.playlists.prefix(30)) { list in row("playlist:\(list.id)", list.name, "music.note.list") }
                }
            }.padding(expanded ? 10 : 5)
        }.scrollIndicators(.hidden)
            .background(MineradioMaterial(cornerRadius: 20))
            .clipShape(RoundedRectangle(cornerRadius: 20))
            .overlay(RoundedRectangle(cornerRadius: 20).stroke(.white.opacity(0.08), lineWidth: 1))
    }
    private func row(_ id: String, _ name: String, _ symbol: String) -> some View {
        Button { navigate(id) } label: {
            HStack(spacing: 12) {
                Image(systemName: symbol).font(.system(size: 15)).frame(width: 20)
                if expanded { Text(name).font(.system(size: 12, weight: section == id ? .semibold : .regular)).lineLimit(1); Spacer(minLength: 0) }
            }.foregroundStyle(section == id ? MusicStyle.accent : .white.opacity(0.70))
                .frame(maxWidth: .infinity, minHeight: 38, alignment: expanded ? .leading : .center)
                .padding(.horizontal, expanded ? 12 : 0)
                .background(section == id ? MusicStyle.accent.opacity(0.13) : .clear, in: RoundedRectangle(cornerRadius: 11))
                .contentShape(RoundedRectangle(cornerRadius: 11))
        }.buttonStyle(.plain).help(name).accessibilityLabel(name)
            .accessibilityAddTraits(section == id ? .isSelected : [])
    }
}

struct MineradioDestination: Identifiable {
    let id: String; let name: String; let symbol: String
    static let all: [Self] = [
        .init(id: "mineradio-home", name: "首页", symbol: "house"), .init(id: "discover", name: "发现音乐", symbol: "safari"),
        .init(id: "library", name: "音乐空间", symbol: "square.stack"), .init(id: "browse", name: "浏览音乐库", symbol: "square.grid.2x2"),
        .init(id: "favorites", name: "我喜欢", symbol: "heart"), .init(id: "playlists", name: "播放列表", symbol: "music.note.list"),
        .init(id: "stats", name: "聆听统计", symbol: "chart.bar"), .init(id: "downloads", name: "下载与缓存", symbol: "arrow.down"),
        .init(id: "outputs", name: "声音设备", symbol: "hifispeaker"), .init(id: "dsp", name: "DSP", symbol: "slider.horizontal.3"), .init(id: "client-settings", name: "本机设置", symbol: "laptopcomputer"),
        .init(id: "settings", name: "Core 设置", symbol: "server.rack"), .init(id: "connection", name: "Core 连接", symbol: "network")
    ]
}

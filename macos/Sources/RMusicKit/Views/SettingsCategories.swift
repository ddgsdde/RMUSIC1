import SwiftUI

struct SettingsCategory: Identifiable {
    let id: String
    let title: String
    let symbol: String
}

/// One horizontal navigation owner. Long Core categories scroll, including keyboard selection.
struct SettingsCategories: View {
    let title: String
    let items: [SettingsCategory]
    @Binding var selection: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var index: Int { items.firstIndex { $0.id == selection } ?? 0 }
    var body: some View {
        HStack(spacing: 6) {
            step(-1)
            ScrollViewReader { reader in
                ScrollView(.horizontal) {
                    HStack(spacing: 7) {
                        ForEach(items) { item in
                            let active = item.id == selection
                            Button { selection = item.id } label: {
                                HStack(spacing: 8) {
                                    Image(systemName: item.symbol).font(.system(size: 12, weight: .medium))
                                    Text(item.title).font(.system(size: 12, weight: active ? .semibold : .medium))
                                }.padding(.horizontal, 15).padding(.vertical, 11)
                                    .foregroundStyle(active ? MusicStyle.accent : Color.secondary)
                                    .background {
                                        RoundedRectangle(cornerRadius: 12).fill(LinearGradient(colors: active ? [MusicStyle.accent.opacity(0.22), MusicStyle.accent.opacity(0.06)] : [.clear, .clear], startPoint: .topLeading, endPoint: .bottomTrailing))
                                    }
                                    .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(active ? MusicStyle.accent.opacity(0.38) : .clear))
                                    .contentShape(RoundedRectangle(cornerRadius: 12))
                            }.buttonStyle(.plain).id(item.id).accessibilityAddTraits(active ? .isSelected : [])
                        }
                    }.padding(4)
                }.scrollIndicators(.hidden)
                    .onChange(of: selection, initial: true) {
                        withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.2)) { reader.scrollTo(selection, anchor: .center) }
                    }
            }
            step(1)
        }.padding(4).background(.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 17))
            .overlay(RoundedRectangle(cornerRadius: 17).strokeBorder(.primary.opacity(0.055)))
            .accessibilityElement(children: .contain).accessibilityLabel(title)
    }
    private func step(_ delta: Int) -> some View {
        Button { selection = items[index + delta].id } label: {
            Image(systemName: delta < 0 ? "chevron.left" : "chevron.right").font(.system(size: 10, weight: .semibold)).frame(width: 32, height: 34).contentShape(Rectangle())
        }.buttonStyle(.plain).disabled(!items.indices.contains(index + delta))
            .accessibilityLabel(delta < 0 ? "上一个设置分类" : "下一个设置分类")
    }
}

extension SettingsCategory {
    static let client: [Self] = [
        .init(id: "appearance", title: "外观", symbol: "paintpalette"),
        .init(id: "streaming", title: "网络播放", symbol: "network"),
        .init(id: "metadata", title: "资料与封面", symbol: "photo.on.rectangle"),
        .init(id: "storage", title: "下载与缓存", symbol: "arrow.down.circle"),
        .init(id: "outputs", title: "声音设备", symbol: "hifispeaker")
    ]
    static let core: [Self] = [
        .init(id: "resources", title: "资源与缓存", symbol: "internaldrive"),
        .init(id: "network", title: "网络与连接", symbol: "network"),
        .init(id: "artwork", title: "图片维护", symbol: "photo.stack"),
        .init(id: "requests", title: "接口保护", symbol: "shield"),
        .init(id: "statistics", title: "聆听统计", symbol: "chart.bar"),
        .init(id: "jobs", title: "后台任务", symbol: "clock"),
        .init(id: "platforms", title: "音乐平台", symbol: "music.note.house"),
        .init(id: "llm", title: "推荐算法", symbol: "sparkles"),
        .init(id: "account", title: "账号与安全", symbol: "person.crop.circle"),
        .init(id: "library", title: "音乐库管理", symbol: "square.stack"),
        .init(id: "backups", title: "备份与恢复", symbol: "externaldrive"),
        .init(id: "plugins", title: "插件工具", symbol: "puzzlepiece.extension"),
        .init(id: "general", title: "通用", symbol: "gearshape")
    ]
}

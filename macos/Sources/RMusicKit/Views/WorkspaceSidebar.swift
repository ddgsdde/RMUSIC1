import SwiftUI

struct WorkspaceSidebar: View {
    let store: MusicStore
    @Binding var section: String
    let create: () -> Void
    let rename: (Playlist) -> Void
    let delete: (Playlist) -> Void
    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Namespace private var selection
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            BrandMark().padding(.horizontal, 26).padding(.top, 32).padding(.bottom, 24)
            ScrollView {
            VStack(alignment: .leading, spacing: 0) {
            if !store.bridgeOnly {
                Text("聆听").font(.system(size: 10, weight: .medium)).foregroundStyle(.tertiary).padding(.leading, 28).padding(.bottom, 10)
                item("discover", "发现音乐", "safari")
                item("library", "音乐空间", "square.stack.3d.up")
                item("favorites", "我喜欢", "heart", count: store.collection.favorites.count)
                item("downloads", "下载与缓存", "arrow.down.to.line")
                item("browse", "浏览音乐库", "rectangle.grid.2x2")
                item("stats", "聆听统计", "chart.bar.xaxis")
                item("playlists", "播放列表", "music.note.list")
                HStack { Text("我的歌单").font(.system(size: 10, weight: .medium)).foregroundStyle(.tertiary); Spacer(); Button(action: create) { Image(systemName: "plus").font(.system(size: 11)) }.buttonStyle(.plain).disabled(!store.connected).help("新建歌单") }.padding(.horizontal, 28).padding(.top, 30).padding(.bottom, 12)
                    VStack(spacing: 3) {
                        ForEach(store.collection.playlists) { playlist in
                            item("playlist:\(playlist.id)", playlist.name, "music.note.list")
                                .contextMenu { Button("重命名") { rename(playlist) }; Button("删除歌单", role: .destructive) { delete(playlist) } }
                        }
                        if store.collection.playlists.isEmpty { Text(store.connected ? "暂无歌单" : "连接后找回你的收藏").font(.system(size: 11)).foregroundStyle(.tertiary).frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 28).padding(.top, 8) }
                    }

            }
            }
            }.scrollIndicators(.hidden)
            Spacer(minLength: 16)
            item("client-settings", "本机设置", "laptopcomputer")
            item("settings", "Core 设置", "server.rack")
            item("outputs", "声音设备", "hifispeaker.2", count: store.outputs.enabled.count)
            item("dsp", "DSP", "slider.horizontal.3")
            Button { section = "connection" } label: {
                HStack(spacing: 10) {
                    ZStack { RoundedRectangle(cornerRadius: 10).fill(.primary.opacity(0.045)); Image(systemName: "server.rack").font(.system(size: 15)).foregroundStyle(.secondary) }.frame(width: 36, height: 36)
                    VStack(alignment: .leading, spacing: 5) { Text(store.connected ? store.profile.name : "连接 Core").font(.system(size: 11, weight: .semibold)).lineLimit(1); Text(store.connectionLabel).font(.system(size: 9)).foregroundStyle(.secondary).lineLimit(1) }
                    Spacer(minLength: 0); Circle().fill(store.connected && store.coreReachable ? Color.green : MusicStyle.accent).frame(width: 5, height: 5)
                }.padding(12).background(.primary.opacity(section == "connection" ? 0.05 : 0), in: RoundedRectangle(cornerRadius: 14))
            }.buttonStyle(.plain).padding(.horizontal, 12).padding(.top, 14).padding(.bottom, 16)
        }.frame(width: MusicStyle.sidebarWidth).background(MusicStyle.rail(scheme))
    }
    private func item(_ key: String, _ title: String, _ icon: String, count: Int? = nil) -> some View {
        Button { section = key } label: {
            HStack(spacing: 12) {
                Image(systemName: icon).font(.system(size: 15, weight: section == key ? .semibold : .regular)).frame(width: 20)
                Text(title).font(.system(size: 12, weight: section == key ? .semibold : .regular)).lineLimit(1)
                Spacer(minLength: 0)
                if let count, count > 0 { Text("\(count)").font(.system(size: 9).monospacedDigit()).foregroundStyle(.secondary) }
            }.foregroundStyle(section == key ? Color.primary : Color.secondary).padding(.horizontal, 15).padding(.vertical, AppearanceStore.shared.skin.rowPadding)
                .background { if section == key { RoundedRectangle(cornerRadius: AppearanceStore.shared.skin.cornerRadius).fill(MusicStyle.panel(scheme)).matchedGeometryEffect(id: "navigation", in: selection) } }
                .contentShape(Rectangle())
        }.buttonStyle(.plain).padding(.horizontal, 12).animation(reduceMotion ? nil : .spring(response: 0.3, dampingFraction: 0.85), value: section)
    }
}

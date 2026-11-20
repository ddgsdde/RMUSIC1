import SwiftUI

struct WorkspaceHeader: View {
    @Bindable var store: MusicStore
    @Binding var section: String
    let signal: () -> Void
    var body: some View {
        HStack(spacing: 20) {
            VStack(alignment: .leading, spacing: 7) {
                Text(section == "connection" ? "连接 Core" : title).font(.system(size: 15, weight: .semibold))
                Text(section == "connection" ? "R-Music / Connect" : store.connected ? store.profile.name : "尚未连接 Core").font(.system(size: 9)).foregroundStyle(.tertiary)
            }
            Spacer()
            ThemeMenu()
            if !store.bridgeOnly && section != "connection" {
                HStack(spacing: 8) { Image(systemName: "magnifyingglass").font(.system(size: 11)).foregroundStyle(.secondary); WorkspaceSearchField(text: $store.query, placeholder: "搜索音乐、专辑、艺术家", fontSize: 11).frame(height: 20) }.padding(.horizontal, 13).padding(.vertical, 11).frame(width: 220).background(.primary.opacity(0.04), in: Capsule())
            }
            if store.connected {
                PlaybackTransferButton(store: store)
                Menu {
                    ForEach(store.playbackZones) { zone in
                        Button { store.selectPlaybackZone(zone.id) } label: {
                            Label(store.playbackOutputLabel(zone) + (store.onlineEndpoints.contains(zone.endpoint_id) ? "" : " · 离线"), systemImage: store.selectedZoneID == zone.id ? "checkmark.circle.fill" : store.outputs.contains(zone.endpoint_id) ? "laptopcomputer" : "hifispeaker")
                        }.disabled(!store.onlineEndpoints.contains(zone.endpoint_id) || store.systemSleeping)
                    }
                    Divider(); Button("管理声音设备") { section = "outputs" }
                } label: { HStack(spacing: 7) { Image(systemName: "hifispeaker"); Text(store.zone.map { store.playbackOutputLabel($0) } ?? "启用声音设备").lineLimit(1); Image(systemName: "chevron.down").font(.system(size: 8)) }.font(.system(size: 10)).frame(maxWidth: 230) }.menuStyle(.borderlessButton).fixedSize(horizontal: true, vertical: false)
                Button(action: signal) { Image(systemName: "waveform.path").font(.system(size: 15)) }.buttonStyle(MusicControlStyle()).foregroundStyle(.secondary).help("查看实际音频链路")
            }
        }.padding(.horizontal, 34).padding(.top, 18).padding(.bottom, 14).contentShape(Rectangle())
    }
    private var title: String {
        switch section { case "search": "搜索音乐"; case "settings": "Core 设置"; case "client-settings": "本机设置"; case "stats": "聆听统计"; case "browse": "浏览音乐库"; case "discover": "发现音乐"; case "library": "音乐空间"; case "favorites": "我喜欢"; case "downloads": "下载与缓存"; case "playlists": "播放列表"; case "outputs": "声音设备"; case "dsp": "DSP"; default: store.collection.name }
    }
}

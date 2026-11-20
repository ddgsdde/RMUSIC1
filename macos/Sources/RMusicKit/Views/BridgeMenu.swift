import SwiftUI

public struct BridgeMenu: View {
    @Bindable var store: MusicStore
    @Environment(\.openWindow) private var openWindow
    public init(store: MusicStore) { self.store = store }
    public var body: some View {
        Text(store.connected ? "R-Music · \(store.outputs.enabled.count) 路已启用" : "R-Music 网桥未连接")
        ForEach(store.outputs.enabled) { output in
            Section(String(output.name.prefix(25))) {
                if let zone = store.zones.first(where: { $0.endpoint_id == output.id }) {
                    Text(String((zone.track?.title ?? "等待播放").prefix(25)))
                    Button(zone.status == "playing" ? "暂停" : "播放") { store.selectedZoneID = zone.id; Task { await store.transport(zone.status == "playing" ? "pause" : "play") } }.disabled(!output.available || !output.online)
                    Button("停止") { store.selectedZoneID = zone.id; Task { await store.transport("stop") } }
                } else { Text(output.available ? "正在连接音区" : "设备未连接") }
            }
        }
        Divider()
        Button("管理 Core 与声音设备…") { openWindow(id: "bridge-settings") }
        if store.connected { Button("断开 Core") { Task { await store.disconnect() } } }
        Divider()
        Button("退出网桥") { NSApplication.shared.terminate(nil) }.keyboardShortcut("q")
    }
}

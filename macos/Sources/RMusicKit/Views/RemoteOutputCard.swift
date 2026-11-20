import SwiftUI

struct RemoteOutputCard: View {
    let output: JSONValue
    let store: MusicStore
    let save: ([String: JSONValue]) -> Void
    let control: (Zone, String) -> Void
    let create: () -> Void
    @StoredState private var expanded = false
    @Environment(\.colorScheme) private var scheme
    private var enabled: Bool { output["configurable"].boolean ? output["config"]["enabled"].boolean : true }
    private var zone: Zone? { store.zones.first { $0.endpoint_id == output["id"].text } }
    private var grouped: Bool { guard let zone else { return false }; return !store.playbackZones.contains { $0.id == zone.id } }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(alignment: .top) {
                Image(systemName: output["kind"].text == "browser" ? "laptopcomputer" : "hifispeaker").font(.system(size: 27, weight: .ultraLight)).foregroundStyle(.primary.opacity(0.7)).frame(width: 48, height: 48).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 14))
                Spacer()
                if output["configurable"].boolean { Toggle("启用 \(output["name"].text)", isOn: Binding(get: { enabled }, set: { save(["enabled": .bool($0)]) })).labelsHidden().toggleStyle(.switch).controlSize(.small) }
            }
            VStack(alignment: .leading, spacing: 7) {
                Text(output["name"].text).font(.system(size: 16, weight: .semibold)).lineLimit(2)
                    .contentShape(Rectangle()).onTapGesture(count: 2) { if output["configurable"].boolean { expanded = true } }
                    .help(output["configurable"].boolean ? "双击编辑输出名称与设置" : "系统输出")
                HStack(spacing: 6) { Circle().fill(output["online"].boolean ? MusicStyle.accent : .secondary.opacity(0.4)).frame(width: 5, height: 5); Text("\(output["online"].boolean ? "在线" : enabled ? "等待设备连接" : "未启用") · \(output["kind"].text.uppercased())") }.font(.system(size: 10)).foregroundStyle(.secondary)
            }
            if let zone {
                HStack(spacing: 9) {
                    Image(systemName: zone.status == "playing" ? "waveform" : "music.note").foregroundStyle(MusicStyle.accent)
                    Text(zone.track?.title ?? "等待选择音乐").font(.system(size: 11)).lineLimit(1)
                    Spacer(minLength: 0)
                    Button(grouped ? "已加入设备组合" : store.selectedZoneID == zone.id ? "当前设备" : "在这里听") { store.selectPlaybackZone(zone.id) }.buttonStyle(MusicControlStyle()).font(.system(size: 10, weight: .semibold)).foregroundStyle(MusicStyle.accent).disabled(!output["online"].boolean || grouped)
                }.padding(12).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 10))
                HStack(spacing: 18) {
                    Button(zone.status == "playing" ? "暂停" : "播放", systemImage: zone.status == "playing" ? "pause.fill" : "play.fill") { control(zone, zone.status == "playing" ? "pause" : "play") }
                    Button("下一首", systemImage: "forward.end.fill") { control(zone, "next") }
                    Button("停止", systemImage: "stop.fill") { control(zone, "stop") }
                    Spacer()
                }.labelStyle(.iconOnly).buttonStyle(MusicControlStyle()).font(.system(size: 12)).disabled(!output["online"].boolean || zone.track == nil || grouped)
            } else if output["online"].boolean { Button("创建播放音区", systemImage: "plus", action: create).buttonStyle(.borderless).font(.caption) }
            if let zone,!grouped {
                Button("打开 DSP 工作区",systemImage:"slider.horizontal.3") { store.dspZoneRequest = zone.id }.buttonStyle(.borderless)
            }
            if output["configurable"].boolean {
                Divider().opacity(0.5)
                HStack {
                    Text(output["config"]["exclusive"].boolean ? "独占输出" : "共享输出").font(.system(size: 10)).foregroundStyle(.secondary)
                    Spacer()
                    Button("输出设置", systemImage: "slider.horizontal.3") { expanded.toggle() }.labelStyle(.iconOnly).buttonStyle(MusicControlStyle())
                }

            }
            if let error = output["endpoint"]["playback"]["error"].string { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
        }.padding(22).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).background(MusicStyle.panel(scheme), in: RoundedRectangle(cornerRadius: 20)).overlay(RoundedRectangle(cornerRadius: 20).strokeBorder(enabled ? MusicStyle.accent.opacity(0.25) : .primary.opacity(0.045)))
        .sheet(isPresented: $expanded) {
            VStack(alignment: .leading, spacing: 24) {
                HStack { Text(output["name"].text).font(.title2.bold()); Spacer(); Button("完成") { expanded = false }.keyboardShortcut(.defaultAction) }
                OutputNameEditor(id: output["id"].text, name: output["name"].text, store: store)
                RemoteOutputSettings(output: output, localOutput: nil, store: store, save: save)
            }.padding(28).frame(width: 480)
        }
    }
}

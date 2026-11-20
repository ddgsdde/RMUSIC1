import SwiftUI

struct OutputsView: View {
    @Bindable var store: MusicStore
    var embedded = false
    @Environment(\.colorScheme) private var scheme
    @StoredState private var editing: NativeOutput?
    var body: some View {
        Group { if embedded { outputGrid } else { content } }
            .sheet(item: $editing) { output in
                VStack(alignment: .leading, spacing: 24) {
                    HStack { Text(output.name).font(.title2.bold()); Spacer(); Button("完成") { editing = nil }.keyboardShortcut(.defaultAction) }
                    if store.connected && store.coreReachable && !store.outputs.roaming { OutputNameEditor(id: output.id, name: output.name, store: store) }
                    ScrollView {
                        VStack(alignment:.leading,spacing:16) {
                            settings(store.outputs.items.first { $0.id == output.id } ?? output)
                            if let zone = store.zones.first(where: { $0.endpoint_id == output.id }) {
                                Button("打开 DSP 工作区",systemImage:"slider.horizontal.3") { editing = nil; store.dspZoneRequest = zone.id }
                            }
                        }
                    }
                }.padding(28).frame(width: 535,height:620)
            }
    }
    private var outputGrid: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 280), spacing: 18)], alignment: .leading, spacing: 18) {
            ForEach(store.outputs.items) { output in outputCard(output) }
        }
    }
    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                HStack(alignment: .bottom) {
                    Text("本机输出").font(.headline)
                    Spacer()
                    Button { Task { await store.perform { try await store.outputs.refresh(store) } } } label: { Label("重新扫描", systemImage: "arrow.clockwise") }.buttonStyle(.plain).font(.system(size: 11)).foregroundStyle(.secondary)
                }
                if !store.connected || store.outputs.roaming || !store.coreReachable { TravelAvailability(connected: store.connected && store.coreReachable) }
                if store.outputs.removed { HStack { Text("此 Mac 已在 Core 中移除，输出保持关闭。"); Spacer(); Button("恢复这台 Mac") { Task { await store.perform { try await store.outputs.restore(store) } } } }.font(.callout).padding(18) }
                HStack(spacing:30) {metric(String(store.outputs.items.count),"读取的设备");metric(String(store.outputs.enabled.count),"已启用的输出");Spacer()}
                outputGrid
            }.padding(34)
        }.scrollIndicators(.hidden).task {try? await store.outputs.refresh(store)}
    }
    private func outputCard(_ output: NativeOutput) -> some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(alignment: .top) {
                deviceIcon(output.name)
                Spacer()
                if store.outputs.busy.contains(output.id) { ProgressView().controlSize(.small) }
                Toggle("启用 \(output.name)", isOn: Binding(get: { output.enabled }, set: { configure(output, ["enabled": .bool($0)]) })).labelsHidden().tint(MusicStyle.accent).toggleStyle(.switch).controlSize(.small).disabled(store.outputs.busy.contains(output.id) || (!output.available && !output.enabled))
            }
            VStack(alignment: .leading, spacing: 7) {
                Text(output.name).font(.system(size: 16, weight: .semibold)).lineLimit(2)
                    .contentShape(Rectangle()).onTapGesture(count: 2) { editing = output }
                    .help("双击编辑输出名称与设置")
                HStack(spacing: 6) { Circle().fill(output.available && output.enabled ? MusicStyle.accent : .secondary.opacity(0.4)).frame(width: 5, height: 5); Text(output.status); if output.system_default && !output.followsSystemDefault { Text("· 系统默认") } }.font(.system(size: 10)).foregroundStyle(.secondary)
                if let route = output.route_name { Text("跟随系统 · \(route)").font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1) }
            }
            if let zone = store.zones.first(where: { $0.endpoint_id == output.id }), output.enabled {
                HStack(spacing: 9) {
                    Image(systemName: zone.status == "playing" ? "waveform" : "music.note").foregroundStyle(MusicStyle.accent)
                    Text(zone.track?.title ?? "等待选择音乐").font(.system(size: 11)).lineLimit(1)
                    Spacer()
                    Button(store.selectedZoneID == zone.id ? "当前音区" : "在这里听") { store.selectPlaybackZone(zone.id) }.buttonStyle(.plain).font(.system(size: 10, weight: .semibold)).foregroundStyle(MusicStyle.accent).disabled(!output.online)
                }.padding(12).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 10))
            }
            Divider().opacity(0.5)
            HStack { Text(output.available ? "设备当前 \((Double(output.mix_rate) / 1000).formatted()) kHz · \(output.channels) 声道" : "设备未连接，保留启用状态与恢复位置").font(.system(size: 10)).foregroundStyle(.secondary); Spacer(); Button { editing = output } label: { Image(systemName: "slider.horizontal.3") }.buttonStyle(.plain).help("输出设置") }
            if let error = output.error { VStack(alignment: .leading, spacing: 9) { Text(error).font(.system(size: 10)).foregroundStyle(.orange).textSelection(.enabled); if let zone = store.zones.first(where: { $0.endpoint_id == output.id }), output.available && output.enabled { Button("重新检测并恢复") { store.selectPlaybackZone(zone.id); Task { await store.retryOutput() } }.buttonStyle(.plain).font(.system(size: 10, weight: .semibold)) } } }
        }.padding(22).frame(maxWidth: .infinity, alignment: .leading).background(MusicStyle.panel(scheme), in: RoundedRectangle(cornerRadius: 20))
            .overlay(RoundedRectangle(cornerRadius: 20).strokeBorder(output.enabled ? MusicStyle.accent.opacity(0.25) : .primary.opacity(0.045)))
    }
    private func settings(_ output: NativeOutput) -> some View {
        VStack(alignment: .leading, spacing: 15) {
            HStack { Text(output.device_volume?.supported == true ? "设备音量可用" : "设备无可写音量"); Spacer() }.foregroundStyle(.secondary)
            PreferenceChoice("音量控制", selection: Binding(get: { output.config.volume_control == "software" ? "software" : output.device_volume?.supported == true && output.config.volume_control == "device" ? "device" : "fixed" }, set: { mode in
                configure(output, ["volume_control": .string(mode), "volume_max": .number(1), "hardware_volume_enabled": .bool(mode != "fixed")])
            })) {
                if output.device_volume?.supported == true { Text("设备音量").tag("device") }
                Text("固定最大").tag("fixed")
                Text("软件音量").tag("software")
            }
            Toggle("独占设备", isOn: Binding(get: { output.config.exclusive }, set: { configure(output, ["exclusive": .bool($0)]) })).toggleStyle(.switch).controlSize(.mini).disabled(output.capabilities?["exclusive"].boolean != true)
            PreferenceChoice("缓冲", selection: Binding(get: { output.config.buffer_ms }, set: { configure(output, ["buffer_ms": .number(Double($0))]) })) { ForEach([20, 50, 100, 200, 500, 1000], id: \.self) { Text("\($0) ms").tag($0) } }
            PreferenceChoice("采样率上限", selection: Binding(get: { output.config.max_sample_rate ?? 0 }, set: { configure(output, ["max_sample_rate": $0 == 0 ? .null : .number(Double($0))]) })) { Text("跟随音源").tag(0); ForEach(output.rates, id: \.self) { Text("\((Double($0) / 1000).formatted()) kHz").tag($0) } }
            PreferenceChoice("有效位深上限", selection: Binding(get: { output.config.max_bit_depth ?? 0 }, set: { configure(output, ["max_bit_depth": $0 == 0 ? .null : .number(Double($0))]) })) { Text("跟随音源").tag(0); ForEach(output.bitDepths, id: \.self) { Text("\($0) bit").tag($0) } }
        }.font(.system(size: 11)).disabled(store.outputs.busy.contains(output.id))
    }
    private func configure(_ output: NativeOutput, _ changes: [String: JSONValue]) { Task { await store.perform { try await store.outputs.configure(output, changes: changes, store: store) } } }
    private func metric(_ value: String, _ label: String) -> some View { HStack(alignment: .firstTextBaseline, spacing: 8) { Text(value).font(.system(size: 30, weight: .light)).monospacedDigit(); Text(label).font(.system(size: 10)).foregroundStyle(.secondary) } }
    private func deviceIcon(_ name: String) -> some View {
        let symbol = name.localizedCaseInsensitiveContains("head") ? "headphones" : name.localizedCaseInsensitiveContains("USB") ? "cable.connector" : "hifispeaker"
        return Image(systemName: symbol).font(.system(size: 27, weight: .ultraLight)).foregroundStyle(.primary.opacity(0.7)).frame(width: 48, height: 48).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 14))
    }
}

struct TravelAvailability: View {
    let connected: Bool
    var body: some View {
        VStack(alignment:.leading,spacing:6) {
            Label(connected ? "外出 · 在这台 Mac 上听" : "本机设备设置",systemImage:connected ? "headphones":"hifispeaker").font(.headline)
            Text(connected ? "音乐由 Core 提供，播放和队列由这台 Mac 控制。打开「Core 全部设备」仍能遥控其他在线设备；家庭音区之间可以流转和组合。其他获授权的设备可以遥控此 Mac 的外出播放器。外出队列和进度保存在本机，电台续播仍可用。外出输出独立于家庭同步和流转。回家后播完当前歌曲再恢复家庭音区；家中已有其他队列时会保留。连接中断后，完整缓存仍可播放；没有缓存的歌曲需要 Core。" : "可以直接启用和配置本机输出。点播曲库歌曲需要连接 Core，本机设备设置会保留。").font(.callout).foregroundStyle(.secondary)
        }.frame(maxWidth:.infinity,alignment:.leading)
    }
}

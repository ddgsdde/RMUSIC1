import SwiftUI

struct RemoteOutputSettings: View {
    let output: JSONValue
    let localOutput: NativeOutput?
    let store: MusicStore
    let save: ([String: JSONValue]) -> Void
    @StoredState private var volumeDraft: Double?
    private var config: JSONValue { output["config"] }
    var body: some View {
        VStack(spacing: 12) {
            Toggle("独占输出", isOn: Binding(get: { config["exclusive"].boolean }, set: { save(["exclusive": .bool($0)]) })).toggleStyle(.switch).controlSize(.small)
            PreferenceChoice("缓冲", selection: Binding(get: { config["buffer_ms"].integer }, set: { save(["buffer_ms": .number(Double($0))]) })) { ForEach([20, 50, 100, 200, 500, 1000, 2000], id: \.self) { Text("\($0) ms").tag($0) } }
            PreferenceChoice("采样率上限", selection: Binding(get: { config["max_sample_rate"].integer }, set: { save(["max_sample_rate": $0 == 0 ? .null : .number(Double($0))]) })) { Text("跟随音源").tag(0); ForEach(output["capabilities"]["sample_rates"].array.map(\.integer), id: \.self) { Text("\((Double($0) / 1000).formatted()) kHz").tag($0) } }
            PreferenceChoice("位深上限", selection: Binding(get: { config["max_bit_depth"].integer }, set: { save(["max_bit_depth": $0 == 0 ? .null : .number(Double($0))]) })) { Text("跟随音源").tag(0); ForEach(output["capabilities"]["bit_depths"].array.map(\.integer), id: \.self) { Text("\($0) bit").tag($0) } }
            if localOutput == nil { PreferenceChoice("音量方式", selection: Binding(get: { config["volume_control"].string ?? "fixed" }, set: { save(["volume_control": .string($0), "volume_max": .number(1)]) })) { if output["device_volume"]["supported"].boolean { Text("设备音量").tag("device") }; Text("固定最大").tag("fixed"); Text("软件音量").tag("software") } }
            if let localOutput { Text(localOutput.device_volume?.supported == true ? "设备硬件音量在本机 CoreAudio 页控制" : "设备无可写硬件音量").foregroundStyle(.secondary) }
            if localOutput == nil, config["volume_control"].text == "software", let zone = store.zones.first(where: { $0.endpoint_id == output["id"].text }) {
                Slider(value: Binding(get: { volumeDraft ?? zone.volume }, set: { volumeDraft = $0 }), in: 0...1, onEditingChanged: { editing in
                    if !editing, let value = volumeDraft {
                        Task {
                            await store.perform {
                                try await store.requireUngrouped(zone.id)
                                _ = try await store.api("/zones/\(zone.id.pathComponentEncoded)", method: "PATCH", body: softwareVolumePatch(value))
                                try await store.refreshZones()
                            }
                        }
                        volumeDraft = nil
                    }
                }) { Text("软件音量") }.disabled(!output["online"].boolean)
            }
            Text("输出格式修改在下次起播生效").foregroundStyle(.secondary)
        }
    }
}

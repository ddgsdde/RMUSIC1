import SwiftUI

struct StreamingSettingsView: View {
    let store: MusicStore
    @StoredState private var draft = StreamingPreferences()
    @StoredState private var action = WorkflowAction()
    var body: some View {
        ScrollView {
            FeatureCard {
                Text("网络播放").font(.headline)
                ThemePicker("连接模式", selection: $draft.mode) {
                    Text("自动识别").tag("auto")
                    Text("局域网").tag("lan")
                    Text("外出流式播放").tag("remote")
                    Text("可信 Tailscale／VPN").tag("vpn")
                }
                ThemePicker("外出音质", selection: $draft.quality) {
                    Text("保留原始规格").tag("original")
                    Text("自定无损转换上限").tag("custom")
                }
                if draft.quality == "custom" {
                ThemePicker("外出采样率上限", selection: $draft.max_sample_rate) {
                    ForEach(StreamingPreferences.rates, id: \.self) { rate in Text("\((Double(rate) / 1000).formatted()) kHz").tag(rate) }
                }
                ThemePicker("外出位深上限", selection: $draft.max_bit_depth) {
                    ForEach([16, 24, 32], id: \.self) { bits in Text("\(bits) bit").tag(bits) }
                }
                }
                Text("外出优先复用本机缓存。前 15 秒通常预取约 5 秒，弱网时提前加深缓冲，播到 15 秒后继续缓冲至曲末。")
                    .font(.caption).foregroundStyle(.secondary)
                Text("FLAC、ALAC 和有损格式保留原始码流；适合的整数 PCM 使用无损 FLAC。默认不降采样或降位深；DSD 由 Core 转为 PCM。转换流仅临时缓冲。")
                    .font(.caption).foregroundStyle(.secondary)
                Text("自动模式区分本地网段和远程路径；Tailscale、VPN 子网路由使用远程预缓冲。只有确认已连可信 VPN 时才选 VPN 模式，公网仍须 HTTPS。保存后从下一次点播／准备生效。").font(.caption).foregroundStyle(.secondary)
                Button("保存") { action.run(store: store, operation: { try await store.saveStreamingPolicy(draft); return .null }, commit: { _ in action.notice = "已保存，下一次点播生效" }) }
                    .disabled(action.busy)
                WorkflowNotice(action: action)
            }.padding(24)
        }.task { draft = store.streamingPolicy }
        .onDisappear { action.cancel() }
    }
}

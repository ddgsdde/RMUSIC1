import SwiftUI

struct SignalPathView: View {
    let store: MusicStore
    var close: () -> Void = {}
    var compact = false
    @StoredState private var resource = ResourceStore()
    @StoredState private var expanded = false
    @StoredState private var contentHeight: CGFloat = 360
    private var presentation: SignalPresentation { SignalPresentation(zone: store.zone, endpoint: store.signalEndpoint, info: resource.value) }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: compact ? 16 : 20) {
                HStack { Label("音频链路", systemImage: "waveform.path").font(.headline); Spacer(); Button("关闭链路", systemImage: "xmark", action: close).labelStyle(.iconOnly).buttonStyle(MusicControlStyle()) }
                if let zone = store.zone, store.signalEndpoint["capabilities"]["dsp"] != .bool(false) {
                    Button("打开 DSP 工作区",systemImage:"slider.horizontal.3") { close(); store.dspZoneRequest = zone.id }.buttonStyle(.borderless)
                }
                if let track = store.zone?.track {
                    Text(track.title).font(.subheadline).lineLimit(2)
                    if !presentation.resolution.isEmpty {
                        Text("链路有效规格 · " + presentation.resolution).font(.caption).foregroundStyle(.secondary)
                    }
                    if let reason = (track.provider_data ?? .null)["playback_selection"]["reason"].string, reason != "requested" {
                        Label(reason == "preferred" ? "已使用你设置的播放首选" : "原版本无法读取，已回退到同一录音", systemImage: "arrow.triangle.branch").font(.caption).foregroundStyle(.secondary)
                    }
                    VStack(spacing: 0) {
                        ForEach(Array(presentation.stages.enumerated()), id: \.element.id) { index, stage in
                            HStack(alignment: .top, spacing: 14) {
                                VStack(spacing: 0) {
                                    Image(systemName: stage.icon).font(.system(size: 13)).frame(width: 32, height: 32).foregroundStyle(stage.processed ? Color.orange : MusicStyle.accent).background((stage.processed ? Color.orange : MusicStyle.accent).opacity(0.12), in: Circle())
                                    if index < presentation.stages.count - 1 { Rectangle().fill(.quaternary).frame(width: 1).frame(minHeight: 32, maxHeight: .infinity) }
                                }.frame(width: 32)
                                VStack(alignment: .leading, spacing: 8) {
                                    HStack { Text(stage.name).font(.subheadline.bold()); Spacer(); Text(stage.status).font(.system(size: 9, weight: .medium)).padding(.horizontal, 8).padding(.vertical, 4).background(.primary.opacity(0.05), in: Capsule()) }
                                    Text(stage.detail).font(.system(size: 11)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                                }.padding(.vertical, 5).padding(.bottom, compact ? 10 : 20)
                            }.fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    ResourceNotice(resource: resource)
                    if let error = store.details.outputError { Label(error, systemImage: "exclamationmark.circle").font(.caption).foregroundStyle(.orange) }
                    if !presentation.processing.isEmpty { DisclosureGroup("处理详情", isExpanded: $expanded) {
                        VStack(alignment: .leading, spacing: 8) {
                            ForEach(Array(presentation.processing.enumerated()), id: \.offset) { _, detail in Text(detail).font(.caption).foregroundStyle(.secondary).textSelection(.enabled) }
                            Text("应用报告不等同于 DAC 硬件逐位验证。").font(.caption).foregroundStyle(.tertiary)
                        }.padding(.top, 10)
                    } }
                } else { ContentUnavailableView("尚未播放", systemImage: "waveform", description: Text("播放后显示实际链路")) }
            }.padding(compact ? 18 : 22)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height in
                    if compact && height > 0 { contentHeight = height.rounded(.up) }
                }
        }.frame(height: compact ? min(contentHeight, 540) : nil)
        .scrollBounceBehavior(.basedOnSize)
        .task(id: "\(store.sessionID):\(store.zone?.track?.id ?? "")") {
            if let track = store.zone?.track { await resource.load("/library/tracks/\(track.id.pathComponentEncoded)/audio-info", store: store) }
        }
    }
}

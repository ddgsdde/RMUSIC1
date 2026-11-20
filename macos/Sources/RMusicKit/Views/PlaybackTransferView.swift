import SwiftUI

struct PlaybackTransferButton: View {
    let store: MusicStore
    @StoredState private var open = false
    var body: some View {
        Button("播放流转", systemImage: "arrow.left.arrow.right") { open = true }
            .labelStyle(.iconOnly).buttonStyle(MusicControlStyle()).help("将完整播放队列流转到另一个设备")
            .disabled(!store.connected || !store.playbackZones.contains(where:{!$0.id.hasPrefix("travel-") && !$0.queue.isEmpty}))
            .sheet(isPresented: $open) { PlaybackTransferView(store: store) }
    }
}
private struct PlaybackTransferView: View {
    let store: MusicStore
    @Environment(\.dismiss) private var dismiss
    @StoredState private var sourceID = ""
    @StoredState private var targetID = ""
    @StoredState private var busy = false
    @StoredState private var error: String?
    private var transferable: [Zone] { store.playbackZones.filter { zone in !zone.id.hasPrefix("travel-") } }
    private var source: Zone? { store.zones.first { $0.id == sourceID } }
    private var target: Zone? { store.zones.first { $0.id == targetID } }
    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            HStack { Text("播放流转").font(.title2.bold()); Spacer(); Button("关闭") { dismiss() }.disabled(busy) }
            Text("完整队列、播放顺序、循环／随机设置及当前位置一起流转，保留播放或暂停状态。目标准备好后原设备停止，目标使用自己的音量与输出设置。").font(.callout).foregroundStyle(.secondary)
            Picker("来源设备", selection: $sourceID) {
                Text("选择来源").tag("")
                ForEach(transferable.filter { !$0.queue.isEmpty }) { zone in Text(store.outputLabels.label(zone)).tag(zone.id) }
            }.disabled(busy)
            Picker("目标设备", selection: $targetID) {
                Text("选择目标").tag("")
                ForEach(transferable.filter { $0.endpoint_id != source?.endpoint_id && store.onlineEndpoints.contains($0.endpoint_id) }) { zone in Text(store.outputLabels.label(zone)).tag(zone.id) }
            }.disabled(busy)
            if let source { Text("\(source.queue.count) 首 · \(source.track?.title ?? "播放队列") · \(source.status == "playing" ? "继续播放" : "保留暂停／停止")").font(.caption).foregroundStyle(.secondary) }
            if let target, !target.queue.isEmpty { Text("将替换目标现有的 \(target.queue.count) 首队列。").font(.caption).foregroundStyle(.secondary) }
            if let error { Text(error).font(.callout).foregroundStyle(.orange).textSelection(.enabled) }
            HStack { if busy { ProgressView().controlSize(.small) }; Spacer(); Button(busy ? "正在准备目标设备…" : "流转到此设备") { transfer() }.buttonStyle(.borderedProminent).disabled(busy || source == nil || target == nil) }
        }.padding(28).frame(width: 520).onAppear { sourceID = transferable.first(where:{$0.id==store.selectedZoneID && !$0.queue.isEmpty})?.id ?? transferable.first(where:{!$0.queue.isEmpty})?.id ?? "" }
            .onChange(of: sourceID) { targetID = "" }.interactiveDismissDisabled(busy)
    }
    private func transfer() {
        guard let source, let target else { return }; busy = true; error = nil
        let epoch = store.sessionID
        Task {
            defer { busy = false }
            do {
                _ = try await store.api("/zones/\(source.id.pathComponentEncoded)/transfer", method: "POST", body: .object(["target_zone_id": .string(target.id), "expected_playback_id": source.playback_id.map(JSONValue.string) ?? .null]))
                guard epoch == store.sessionID else { return }
                store.selectedZoneID = target.id; try await store.refreshZones(); dismiss()
            } catch { self.error = error.localizedDescription }
        }
    }
}

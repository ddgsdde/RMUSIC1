import SwiftUI

/// Transport-owned popover: settings always come from the selected Core zone.
struct PlaybackOptionsControl: View {
    let store: MusicStore
    var interactionChanged: (Bool) -> Void = { _ in }
    @StoredState private var expanded = false
    private var mode: PlaybackMode { PlaybackMode(zone: store.zone) }
    var body: some View {
        Button { expanded.toggle() } label: {
            HStack(spacing: 4) {
                PlaybackModeIcon(mode: mode)
                Image(systemName: "chevron.up").font(.system(size: 8, weight: .semibold))
            }.frame(width: 40, height: 40).contentShape(Rectangle())
        }.buttonStyle(.plain).help("播放选项：\(mode.rawValue)")
            .accessibilityLabel("播放选项").accessibilityValue(mode.rawValue).accessibilityIdentifier("playbackOptions")
            .disabled(!store.canPlay || store.zone == nil)
            .popover(isPresented: $expanded, arrowEdge: .bottom) {
                PlaybackOptionsPanel(store: store).id("\(store.sessionID):\(store.zone?.id ?? "")")
            }
            .onChange(of: expanded) { _, value in interactionChanged(value) }
            .onChange(of: store.selectedZoneID) { expanded = false }
            .onChange(of: store.sessionID) { expanded = false }
            .onDisappear { interactionChanged(false) }
    }
}

private struct PlaybackOptionsPanel: View {
    let store: MusicStore
    @StoredState private var action = WorkflowAction()
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("播放选项").font(.headline)
                Spacer()
                if action.busy { ProgressView().controlSize(.mini) }
            }
            if let zone = store.zone {
                LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                    ForEach(PlaybackMode.allCases, id: \.self) { mode in modeButton(mode, zone: zone) }
                }
                Divider()
                Button("停止播放", systemImage: "stop.fill") { Task { await store.transport("stop") } }.buttonStyle(.plain)
            }
            if let error = action.error { Text(error).font(.caption).foregroundStyle(.orange).fixedSize(horizontal: false, vertical: true) }
        }.padding(20).frame(width: 300).disabled(action.busy || !store.canPlay)
            .onDisappear { action.cancel() }
    }
    private func modeButton(_ mode: PlaybackMode, zone: Zone) -> some View {
        let selected = PlaybackMode(zone: zone) == mode
        return Button { patch(zone, ["repeat": .string(mode.repeatValue), "shuffle": .bool(mode == .random)]) } label: {
            VStack(spacing: 10) { PlaybackModeIcon(mode: mode); Text(mode.rawValue).font(.caption) }
                .frame(maxWidth: .infinity).padding(.vertical, 14)
                .foregroundStyle(selected ? MusicStyle.accent : .secondary)
                .background(selected ? MusicStyle.accent.opacity(0.12) : Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(selected ? MusicStyle.accent.opacity(0.6) : .clear))
        }.buttonStyle(.plain).accessibilityLabel(mode.rawValue).accessibilityValue(selected ? "已选中" : "未选中")
    }
    private func patch(_ zone: Zone, _ fields: [String: JSONValue]) {
        action.run(store: store, preflight: { try await store.requireUngrouped(zone.id) }, operation: {
            guard store.zone?.id == zone.id else { throw CancellationError() }
            let result = try await store.api("/zones/\(zone.id.pathComponentEncoded)", method: "PATCH", body: .object(fields))
            try Task.checkCancellation()
            try await store.refreshZones()
            return result
        })
    }

}

private enum PlaybackMode: String, CaseIterable {
    case ordered = "顺序播放", loop = "列表循环", single = "单曲循环", random = "随机播放"
    init(zone: Zone?) {
        if zone?.shuffle == true { self = .random }
        else if zone?.repeat == "one" { self = .single }
        else if zone?.repeat == "all" { self = .loop }
        else { self = .ordered }
    }
    var repeatValue: String { self == .ordered ? "off" : self == .single ? "one" : "all" }
}

private struct PlaybackModeIcon: View {
    let mode: PlaybackMode
    var body: some View {
        Group {
            if mode == .ordered {
                Path { path in
                    for y in [CGFloat(4), CGFloat(12)] {
                        path.move(to: CGPoint(x: 1, y: y)); path.addLine(to: CGPoint(x: 19, y: y))
                        path.move(to: CGPoint(x: 15, y: y-3)); path.addLine(to: CGPoint(x: 19, y: y)); path.addLine(to: CGPoint(x: 15, y: y+3))
                    }
                }.stroke(style: StrokeStyle(lineWidth: 1.7, lineCap: .round, lineJoin: .round)).frame(width: 20, height: 16)
            } else {
                Image(systemName: mode == .random ? "shuffle" : mode == .single ? "repeat.1" : "repeat").font(.system(size: 17, weight: .medium))
            }
        }.frame(width: 22, height: 20).accessibilityHidden(true)
    }
}

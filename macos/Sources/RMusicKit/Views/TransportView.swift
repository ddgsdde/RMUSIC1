import SwiftUI

struct TransportView: View {
    @Bindable var store: MusicStore
    var openPlayer: () -> Void = {}
    var openQueue: () -> Void = {}
    var floatingBackground = false
    var interactionChanged: (Bool) -> Void = { _ in }
    @StoredState private var showSignal = false
    @StoredState private var seeking: Double?
    @StoredState private var volumeInteraction = false
    @StoredState private var playbackOptions = false
    @ViewBuilder private var bufferLayer: some View {
        if let zone = store.zone, zone.id.hasPrefix("travel-"), let instance = zone.playback_id,
           let track = zone.track, track.duration > 0, seeking == nil,
           store.streamBuffer["core_playback_id"].string == instance,
           store.streamBuffer["track_id"].string == track.id {
            let played = min(track.duration, store.presentationClock.reading(zone).position)
            let prefix = store.streamBuffer["buffered_fraction"].number * track.duration
            let reportPosition = store.streamBuffer["position"].number
            let until = min(track.duration, store.streamBuffer["buffered_until"].number)
            let from = max(played, reportPosition)
            GeometryReader { geometry in
                let width = max(0, geometry.size.width - 12)
                let scale = width / track.duration
                ZStack(alignment: .leading) {
                    if prefix > played {
                        Capsule().fill(Color.secondary.opacity(0.5))
                            .frame(width: (min(prefix, track.duration) - played) * scale, height: 3)
                            .offset(x: played * scale + 6)
                    }
                    if until > max(from, prefix) {
                        Capsule().fill(Color.secondary.opacity(0.5))
                            .frame(width: (until - max(from, prefix)) * scale, height: 3)
                            .offset(x: max(from, prefix) * scale + 6)
                    }
                }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            }.accessibilityHidden(true)
        }
    }
    var body: some View {
        VStack(spacing: 0) {
            TimelineView(.animation(minimumInterval:store.uiActive ? 0.125 : 0.5,paused:store.zone?.status != "playing" || store.bufferingTrack != nil)) { _ in
            Slider(value: Binding(get: { min(seeking ?? (store.bufferingTrack == nil ? store.presentationClock.reading(store.zone).position : store.zone?.position ?? 0), duration) }, set: { seeking = $0 }), in: 0...duration) { editing in
                if !editing, let value = seeking { Task { await store.transport("seek", body: ["position": .number(value)]); seeking = nil } }
            }.controlSize(.mini).tint(MusicStyle.accent)
                .overlay { bufferLayer.allowsHitTesting(false) }
                .accessibilityLabel("播放进度").disabled(!store.canPlay || store.zone?.track == nil).padding(.horizontal, 16)
            }
            HStack(spacing: 18) {
                Button(action: openPlayer) {
                    HStack(spacing: 12) {
                        ArtworkView(track: store.zone?.track, store: store, radius: 8, resolveMissing: true).frame(width: 52, height: 52)
                        VStack(alignment: .leading, spacing: 6) {
                            Text(store.zone?.track?.title ?? "未在播放").font(.system(size: 13, weight: .semibold)).lineLimit(1)
                            Group { if let id=store.bufferingTrack,store.zone?.track?.id==id,store.zone?.status=="playing" {Text("正在缓冲…")} else if let track = store.zone?.track { CatalogTextLink(track: track, store: store) } else { Text("选择歌曲") } }.font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }
                }.buttonStyle(.plain).frame(minWidth: 150, maxWidth: .infinity).help("展开播放页")
                if let track = store.zone?.track {
                    FavoriteButton(track: track, store: store, size: 22).id(track.id)
                }
                HStack(spacing: 18) {
                    PlaybackButtons(store: store)
                    PlaybackOptionsControl(store: store, interactionChanged: { playbackOptions = $0 })
                }
                HStack(spacing: 14) {
                    Button { showSignal = true } label: {
                        VStack(spacing: 3) {
                            Image(systemName: "waveform.path").foregroundStyle(MusicStyle.accent)
                            Text(quality).font(.system(size: 9).monospacedDigit()).foregroundStyle(.secondary)
                        }.frame(minWidth: 70)
                    }.buttonStyle(MusicControlStyle()).help("实际音频链路").popover(isPresented: $showSignal) { SignalPathView(store: store, close: { showSignal = false }, compact: true).frame(width: 380) }
                    TimelineView(.animation(minimumInterval: 0.5, paused: store.zone?.status != "playing")) { _ in
                        Text("\(timeLabel(seeking ?? store.presentationClock.reading(store.zone).position)) / \(timeLabel(store.zone?.track?.duration ?? 0))").font(.system(size: 10).monospacedDigit()).foregroundStyle(.secondary)
                    }
                    VolumeControlView(store: store, interactionChanged: { volumeInteraction = $0 })
                    Button("播放队列", systemImage: "text.line.first.and.arrowtriangle.forward", action: openQueue).labelStyle(.iconOnly).buttonStyle(MusicControlStyle()).help("播放队列（\(store.zone?.queue.count ?? 0) 首）")
                }.frame(maxWidth: .infinity, alignment: .trailing)
            }.padding(.horizontal, 24).padding(.top, 10).padding(.bottom, 18)
        }.background {
            if floatingBackground { Rectangle().fill(.ultraThinMaterial).opacity(0.28).overlay(.black.opacity(0.12)) }
            else { Rectangle().fill(.bar) }
        }.contextMenu {
            if let track = store.zone?.track {
                Button("找相似", systemImage: "music.note.magnifyingglass") { store.similarSongs.find(track, store: store) }
            }
            Button("停止播放", systemImage: "stop.fill") { Task { await store.transport("stop") } }
        }
            .onChange(of: showSignal || seeking != nil || volumeInteraction || playbackOptions) { _, value in interactionChanged(value) }
            .onDisappear { interactionChanged(false) }
    }
    private var quality: String {
        let resolution = SignalPresentation(zone: store.zone, endpoint: store.signalEndpoint, info: .null).resolution
        return resolution.isEmpty ? "等待链路报告" : resolution
    }
    private var duration: Double { max(1, store.zone?.track?.duration ?? 1) }
}
struct PlaybackButtons: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let store: MusicStore
    var body: some View {
        HStack(spacing: 24) {
            Button("上一首", systemImage: "backward.end.fill") { Task { await store.transport("previous") } }
                .buttonStyle(MusicControlStyle()).font(.system(size: 16))
            Button { Task { await store.transport(store.zone?.status == "playing" ? "pause" : "play") } } label: {
                Image(systemName: store.zone?.status == "playing" ? "pause.fill" : "play.fill").font(.system(size: 18, weight: .semibold)).foregroundStyle(MusicStyle.onAccent).frame(width: 44, height: 44).background(MusicStyle.accent, in: Circle())
                    .contentTransition(reduceMotion ? .identity : .symbolEffect(.replace))
            }.buttonStyle(MusicButtonStyle()).keyboardShortcut(.space, modifiers: []).accessibilityLabel(store.zone?.status == "playing" ? "暂停" : "播放").accessibilityIdentifier("playPause")
            Button("下一首", systemImage: "forward.end.fill") { Task { await store.transport("next") } }.buttonStyle(MusicControlStyle()).font(.system(size: 16))
        }.labelStyle(.iconOnly).disabled(!store.canPlay)
    }
}

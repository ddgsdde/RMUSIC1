import SwiftUI

struct MusicButtonStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.opacity(configuration.isPressed ? 0.72 : 1)
            .scaleEffect(configuration.isPressed && !reduceMotion ? 0.95 : 1)
            .animation(reduceMotion ? nil : .spring(response: 0.25, dampingFraction: 0.7), value: configuration.isPressed)
    }
}
/// Small desktop actions retain a real hit area around their label.
struct MusicControlStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        ControlBody(configuration: configuration, enabled: enabled)
    }
    private struct ControlBody: View {
        let configuration: ButtonStyleConfiguration
        let enabled: Bool
        @StoredState private var hovering = false
        var body: some View {
            configuration.label
                .padding(.horizontal, 8).padding(.vertical, 6)
                .frame(minWidth: 32, minHeight: 32)
                .background(MusicStyle.accent.opacity(enabled && (hovering || configuration.isPressed) ? (configuration.isPressed ? 0.16 : 0.08) : 0), in: RoundedRectangle(cornerRadius: 8))
                .contentShape(Rectangle()).opacity(enabled ? 1 : 0.4)
                .onHover { hovering = $0 }
        }
    }
}
struct MusicTrackRow: View {
    let track: Track
    let index: Int
    let store: MusicStore
    var play: (() -> Void)?
    var playlist: Playlist?
    var showCatalogDetails = false
    @StoredState private var hovering = false
    var body: some View {
        HStack(spacing: 16) {
            Button {
                if current { Task { await store.transport(store.zone?.status == "playing" ? "pause" : "play") } }
                else { startPlayback() }
            } label: { ZStack {
                Text(String(format: "%02d", index + 1)).font(.caption.monospacedDigit()).foregroundStyle(.tertiary)
                    .opacity(hovering || current ? 0 : 1)
                Image(systemName: current && store.zone?.status == "playing" ? hovering ? "pause.fill" : "waveform" : "play.fill")
                    .foregroundStyle(current ? MusicStyle.accent : Color.primary).opacity(hovering || current ? 1 : 0)
            }.frame(width: 32, height: 42).contentShape(Rectangle()) }
                .buttonStyle(.plain).disabled(!store.canPlay || track.available == false)
                .help(current && store.zone?.status == "playing" ? "暂停" : "播放")
                .accessibilityLabel("\(current && store.zone?.status == "playing" ? "暂停" : "播放") \(track.title)")
            ArtworkView(track: track, store: store, radius: 7).frame(width: 42, height: 42)
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 8) { Text(track.title).font(.system(size: 14, weight: .medium)).foregroundStyle(current ? MusicStyle.accent : Color.primary).lineLimit(1); if !track.releaseDate.isEmpty { NewMusicBadge(date: track.releaseDate) } }
                CatalogTextLink(track: track, store: store).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
            }.frame(maxWidth: .infinity, alignment: .leading)
            if showCatalogDetails { VStack(alignment: .trailing, spacing: 4) { Text(track.releaseDate.isEmpty ? "发行日期未知" : track.releaseDate); Text("聆听 \((track.provider_data ?? .null)["catalog_display"]["play_count"].integer) 次") }.font(.caption2).foregroundStyle(.secondary).frame(width: 92) }
            CatalogTextLink(track: track, store: store, album: true).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1).frame(width: 150, alignment: .leading)
            Text(track.quality).font(.system(size: 10, weight: .medium)).lineLimit(2).multilineTextAlignment(.center).padding(.horizontal, 6).padding(.vertical, 3)
                .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(.quaternary)).foregroundStyle(.secondary).frame(width: showCatalogDetails ? 134 : 110).help(track.quality)
            FavoriteButton(track: track, store: store).id(track.id)
            Text(timeLabel(track.duration)).font(.system(size: 12).monospacedDigit()).foregroundStyle(.secondary).frame(width: 45, alignment: .trailing)
            Menu { TrackActions(track: track, store: store, playlist: playlist) } label: { Image(systemName: "ellipsis").frame(width: 32, height: 32).contentShape(Rectangle()) }.menuStyle(.borderlessButton).frame(width: 36, height: 32).help("歌曲操作")
        }
        .padding(.horizontal, 14).padding(.vertical, 10)
        .background(RoundedRectangle(cornerRadius: 10).fill(current ? MusicStyle.accent.opacity(0.08) : hovering ? Color.primary.opacity(0.045) : .clear))
        .contentShape(Rectangle()).onHover { hovering = $0 }
        .onTapGesture(count: 2) { startPlayback() }
        .contextMenu { TrackActions(track: track, store: store, playlist: playlist) }
        .accessibilityElement(children: .contain)
    }
    private var current: Bool { store.zone?.track?.id == track.id }
    private func startPlayback() { if let play { play() } else { Task { await store.play(track) } } }
}
struct PageControls: View {
    let page: Int
    let total: Int
    var loading = false
    let change: (Int) -> Void
    var body: some View {
        HStack {
            Text("\(total) 首歌曲").foregroundStyle(.secondary)
            if loading { ProgressView().controlSize(.mini) }
            Spacer()
            Button("上一页", systemImage: "chevron.left") { change(page - 1) }.disabled(page == 0 || loading)
            Text("\(page + 1)").monospacedDigit().foregroundStyle(.secondary)
            Button("下一页", systemImage: "chevron.right") { change(page + 1) }.disabled((page + 1) * 100 >= total || loading)
        }.font(.caption).buttonStyle(MusicControlStyle()).padding(.vertical, 12)
    }
}

import SwiftUI
import AppKit

struct NowPlayingView: View {
    let store: MusicStore
    let close: () -> Void
    var openQueue: () -> Void = {}
    @AppStorage("nativePlayerView") private var layout = "record"
    @AppStorage("listeningTheme") private var savedTheme = "silk"
    @AppStorage("playerAtmosphere") private var atmosphere = true
    @AppStorage("playerLowPerformance") private var lowPerformance = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var scheme
    @Environment(\.scenePhase) private var scenePhase
    @StoredState private var palette = ArtworkPalette.fallback
    @StoredState private var customizing = false
    private var theme: ListeningTheme { AppearanceStore.shared.skin == .mineradio ? .mineradio : ListeningTheme(rawValue: savedTheme) ?? .silk }
    private var isMineradio: Bool { theme == .mineradio }
    private var moving: Bool { !lowPerformance && !reduceMotion && scenePhase == .active && store.zone?.status == "playing" }
    var body: some View {
        ZStack {
            if isMineradio && AppearanceStore.shared.skin != .mineradio { MineradioSurface(store: store, immersive: true) }
            else if !isMineradio {
                ListeningBackdrop(palette: palette, theme: theme, moving: atmosphere && moving)
            }
            VStack(spacing: 0) {
                toolbar.padding(.horizontal, 34).padding(.top, 22).padding(.bottom, 12)
                if isMineradio { MineradioStageView(store: store).padding(.horizontal, 36) }
                else { GeometryReader { geometry in
                    composition(geometry.size).padding(.horizontal, max(36, geometry.size.width * 0.065)).padding(.vertical, 24)
                }
                }
                TransportView(store: store, openPlayer: close, openQueue: openQueue)
                    .clipShape(RoundedRectangle(cornerRadius: isMineradio ? 26 : 0))
                    .padding(.horizontal, isMineradio ? 76 : 0).padding(.bottom, isMineradio ? 22 : 0)
            }
        }
        .environment(\.colorScheme, isMineradio ? .dark : theme.scheme ?? scheme)
        .task(id: "\(store.sessionID):\(store.zone?.track?.id ?? ""):\(store.zone?.track?.cover_url ?? "")") {
            palette = .fallback
            guard let track = store.zone?.track, let image = await store.artwork.image(track, store: store, resolveMissing: true), !Task.isCancelled,
                  let bitmap = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return }
            let worker = Task.detached(priority: .utility) { ArtworkPalette.extract(bitmap) }
            let next = await withTaskCancellationHandler { await worker.value } onCancel: { worker.cancel() }
            guard !Task.isCancelled else { return }
            withAnimation(reduceMotion ? nil : .easeInOut(duration: 1)) { palette = next }
        }
        .sheet(isPresented: $customizing) {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    HStack { Text("播放页外观").font(.title2.bold()); Spacer(); Button("完成") { customizing = false }.keyboardShortcut(.defaultAction) }
                    ListeningAppearanceSettings()
                    Toggle("流动氛围背景", isOn: $atmosphere)
                    Toggle("减少动态效果", isOn: $lowPerformance)
                }.padding(28)
            }.frame(width: 540, height: isMineradio ? 580 : 340)
        }
        .accessibilityIdentifier("immersivePlayer")
    }
    private var toolbar: some View {
        HStack(spacing: 18) {
            Button("收起播放页", systemImage: "chevron.down", action: close).labelStyle(.iconOnly).buttonStyle(MusicControlStyle()).font(.title3).help("收起播放页")
            Spacer()
            Text(store.zone?.name ?? "选择声音设备").font(.caption).foregroundStyle(.secondary)
            Button("外观与布局", systemImage: "slider.horizontal.3") { customizing = true }.labelStyle(.iconOnly).buttonStyle(MusicControlStyle()).help("播放页外观与布局")
            if let track = store.zone?.track {
                Menu("歌曲操作", systemImage: "ellipsis") { TrackActions(track: track, store: store) }.labelStyle(.iconOnly).menuStyle(.borderlessButton).frame(width: 30)
            }
            ThemeMenu()
        }
    }
    @ViewBuilder private func composition(_ size: CGSize) -> some View {
        if layout == "custom" { PlayerModulesView(store: store, lyrics: ListeningLyrics(store: store)) }
        else if layout == "lyrics" {
            VStack(alignment: .leading, spacing: 26) { trackHeading; ListeningLyrics(store: store) }.frame(maxWidth: 820).frame(maxWidth: .infinity)
        } else {
            let width = min(500, max(220, min((size.width - 150) * 0.44, size.height - 130)))
            HStack(alignment: .center, spacing: max(55, size.width * 0.07)) {
                VStack(alignment: .leading, spacing: 24) {
                    Spacer(minLength: 0)
                    if isMineradio { Color.clear.frame(width: width, height: width).accessibilityHidden(true) }
                    else if layout == "record" { ListeningRecord(store: store, size: width, moving: moving, paper: theme == .paper) }
                    else {
                        ArtworkView(track: store.zone?.track, store: store, radius: theme == .paper ? 4 : 16, resolveMissing: true).frame(width: width, height: width)
                            .shadow(color: .black.opacity(0.22), radius: 28, y: 18)
                    }
                    if size.height > 430 { trackHeading }
                    Spacer(minLength: 0)
                }.frame(width: width)
                VStack(alignment: .leading, spacing: 24) {
                    if size.height <= 430 { trackHeading }
                    ListeningLyrics(store: store)
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }
    private var trackHeading: some View {
        VStack(alignment: .leading, spacing: 9) {
            Text(store.zone?.track?.title ?? "尚未播放").font(.system(size: 25, weight: .semibold, design: theme == .paper ? .serif : .default)).lineLimit(2)
            Group { if let track = store.zone?.track { CatalogTextLink(track: track, store: store) } }.font(.system(size: 15)).foregroundStyle(.secondary).lineLimit(1)
            if let track = store.zone?.track, !track.album.isEmpty { CatalogTextLink(track: track, store: store, album: true).font(.caption).foregroundStyle(.tertiary).lineLimit(1) }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}

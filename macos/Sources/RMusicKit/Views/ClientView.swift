import SwiftUI
import AppKit

/// Navigation, workspace and optional queue or signal panel share one window.
public struct ClientView: View {
    @Bindable var store: MusicStore
    @StoredState private var themeMenu = ThemeMenuPresentation()
    @StoredState private var section = "connection"
    @StoredState private var dailyPlaylist: DailyPlaylistSnapshot?
    @StoredState private var dailyOrigin = "mineradio-home"
    @StoredState private var player = false
    @StoredState private var inspector = false
    @StoredState private var queue = false
    @StoredState private var editing = false
    @StoredState private var playlistName = ""
    @StoredState private var renaming: Playlist?
    @StoredState private var deleting: Playlist?
    @StoredState private var dspZone: String?
    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    public init(store: MusicStore) {
        self.store = store
        _section = StoredState(initialValue: store.bridgeOnly ? "outputs" : AppearanceStore.shared.skin == .mineradio ? "mineradio-home" : "library")
    }
    public var body: some View {
        ZStack {
            if store.workspaceReady {
                workspace
                    .allowsHitTesting(store.workspaceReady)
                    .transition(.opacity.combined(with: reduceMotion ? .identity : .scale(scale: 0.985)).combined(with: reduceMotion ? .identity : .offset(y: 12)))
            } else {
                LoginScreen(store: store, coreUnavailable: store.startupUnavailable)
                    .allowsHitTesting(!store.workspaceReady)
                    .transition(.opacity.combined(with: reduceMotion ? .identity : .scale(scale: 1.015)))
            }
        }
        .animation(reduceMotion ? .easeInOut(duration: 0.12) : .smooth(duration: 0.55), value: store.workspaceReady)
        .overlay(alignment: .bottomTrailing) {
            if store.workspaceReady, let notice = store.notice {
                HStack(spacing: 10) {
                    Text(notice).font(.caption).lineLimit(2)
                    Button("关闭提示", systemImage: "xmark") { store.notice = nil }.labelStyle(.iconOnly).buttonStyle(MusicControlStyle())
                }.padding(12).frame(maxWidth: 360).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12)).padding(.trailing, 24).padding(.bottom, store.connected && !store.bridgeOnly ? 108 : 24)
            }
        }
        .overlay(alignment: .topTrailing) {
            if store.workspaceReady { SimilarSongsNotice(store: store).padding(.top, 42).padding(.trailing, 16) }
        }
        .onChange(of: store.sessionID) {
            store.similarSongs.close()
            dailyPlaylist = nil
            if section == "daily-playlist" { section = dailyOrigin }
        }
        .task(id: store.notice) {
            guard let notice = store.notice else { return }
            do { try await Task.sleep(for: .seconds(4)) } catch { return }
            if store.notice == notice { store.notice = nil }
        }
        .overlayPreferenceValue(ThemeMenuAnchor.self) { anchor in ThemeMenuOverlay(presentation: themeMenu, anchor: anchor) }
        .environment(themeMenu)
        .preferredColorScheme(AppearanceStore.shared.scheme)
        .tint(MusicStyle.accent)
        .toolbar(removing: .title)
        .toolbarBackgroundVisibility(.hidden, for: .windowToolbar)
        .background { MusicStyle.canvas(scheme).ignoresSafeArea() }
        .onAppear { store.setUIActive(NSApp.isActive) }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in store.setUIActive(true) }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in store.setUIActive(false) }
        .onChange(of: AppearanceStore.shared.skin) { if AppearanceStore.shared.skin == .mineradio { section = "mineradio-home" } else if section == "mineradio-home" { section = "library" } }
        .onChange(of: section) { store.detailTrack = nil; player = false; inspector = false; selectSection() }
        .onChange(of: store.showDownloads) { if store.showDownloads { section = "downloads"; store.showDownloads = false } }
        .onChange(of: store.showLibrary) { if store.showLibrary { player = false; queue = false; inspector = false; section = "library"; store.showLibrary = false } }
        .onChange(of: store.showConnection) { if store.showConnection { section = "connection"; store.showConnection = false } }
        .onChange(of: store.dspZoneRequest) {
            if let id = store.dspZoneRequest { dspZone = id; section = "dsp"; player = false; inspector = false; store.detailTrack = nil; store.dspZoneRequest = nil }
        }
        .onChange(of: store.query) { if !store.query.isEmpty { section = "search"; player = false }; store.searchChanged() }
        .onChange(of: store.connected) {
            if store.connected, section == "connection" { section = store.bridgeOnly ? "outputs" : AppearanceStore.shared.skin == .mineradio ? "mineradio-home" : "library" }
            if !store.connected { queue = false }
        }
        .onChange(of: store.collection.playlists.map(\.id)) { if section.hasPrefix("playlist:"), !store.collection.playlists.contains(where: { "playlist:\($0.id)" == section }) { section = "library" } }
        .alert(renaming == nil ? "新建歌单" : "重命名歌单", isPresented: $editing) {
            TextField("歌单名称", text: $playlistName)
            Button("取消", role: .cancel) {}
            Button("保存") { savePlaylist() }.disabled(playlistName.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .confirmationDialog("删除这个歌单？", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), titleVisibility: .visible) {
            Button("删除歌单", role: .destructive) { if let deleting { Task { await store.perform { try await store.collection.delete(deleting, store: store) } } }; deleting = nil }
        } message: { Text("歌曲文件会保留在 Core 音乐库。") }
        .animation(appearanceAnimation, value: AppearanceStore.shared.theme)
        .animation(appearanceAnimation, value: AppearanceStore.shared.mode)
        .animation(appearanceAnimation, value: AppearanceStore.shared.skin)
        .animation(appearanceAnimation, value: scheme)
        .task { await store.prepare() }
        .frame(minWidth: 1120, minHeight: 740)
    }
    private var pageIdentity: String { "\(store.sessionID):\(section):\(store.detailTrack?.id ?? "")" }
    private var appearanceAnimation: Animation { reduceMotion ? .easeOut(duration: 0.12) : .easeInOut(duration: 0.42) }
    private var workspace: some View {
        ZStack {
            if AppearanceStore.shared.skin == .mineradio {
                MineradioWorkspaceView(store: store, section: $section, player: $player, queue: $queue, inspector: $inspector, openDaily: openDaily) {
                    ZStack { page.id(pageIdentity).transition(.opacity.combined(with: reduceMotion ? .identity : .offset(y: 6))) }
                        .animation(reduceMotion ? .easeOut(duration: 0.1) : .smooth(duration: 0.26), value: pageIdentity)
                }
                    .transition(.opacity)
            } else if player {
                NowPlayingView(store: store, close: { player = false }, openQueue: { queue.toggle() })
                    .overlay(alignment: .trailing) {
                        ZStack(alignment: .trailing) {
                            if queue { PlaybackQueuePanel(store: store, close: { queue = false }).frame(width: 340).background(.regularMaterial).transition(.move(edge: .trailing).combined(with: .opacity)) }
                        }.animation(reduceMotion ? nil : .smooth(duration: 0.25), value: queue)
                    }
                    .transition(.opacity.combined(with: reduceMotion ? .identity : .scale(scale: 0.97)))
            } else { standardWorkspace.transition(.opacity) }
        }.animation(reduceMotion ? nil : .smooth(duration: 0.3), value: AppearanceStore.shared.skin == .mineradio ? false : player)
    }
    private var standardWorkspace: some View {
            HStack(spacing: 0) {
                WorkspaceSidebar(store: store, section: $section, create: { renaming = nil; playlistName = ""; editing = true }, rename: { renaming = $0; playlistName = $0.name; editing = true }, delete: { deleting = $0 })
                VStack(spacing: 0) {
                    WorkspaceHeader(store: store, section: $section, signal: { inspector.toggle() })
                    HStack(spacing: 0) {
                        ZStack(alignment: .topLeading) {
                            page.id(pageIdentity).transition(.opacity.combined(with: reduceMotion ? .identity : .offset(y: 8)))
                            if inspector { SignalPathView(store: store, close: { inspector = false }).frame(width: 350).frame(maxHeight: .infinity).background(.regularMaterial).frame(maxWidth: .infinity, alignment: .trailing).transition(.move(edge: .trailing).combined(with: .opacity)).zIndex(3) }
                        }.frame(maxWidth: .infinity, maxHeight: .infinity).clipped()
                            .animation(reduceMotion ? nil : .smooth(duration: 0.26), value: pageIdentity)
                            .animation(reduceMotion ? nil : .smooth(duration: 0.25), value: inspector)
                        if queue && store.connected { PlaybackQueuePanel(store: store, close: { queue = false }).frame(width: 340).background(.bar).transition(.move(edge: .trailing)) }
                    }.animation(reduceMotion ? nil : .smooth(duration: 0.25), value: queue)
                    if let connectionIssue = store.connectionIssue { banner(connectionIssue, error: false) }
                    if section != "connection", let error = store.error { banner(error, error: true) }
                    if store.connected { PlaybackRecoveryBanner(store: store) }
                    if store.connected && !store.bridgeOnly { TransportView(store: store, openPlayer: { player.toggle() }, openQueue: { queue.toggle(); inspector = false }) }
                }.background(MusicStyle.canvas(scheme))
            }
    }
    @ViewBuilder private var page: some View {
        if !store.connected {
            VStack(spacing: 16) {
                ProgressView(store.awaitingSavedSession ? "正在恢复登录…" : "正在连接 Core…")
                Button("退出登录并选择 Core") { Task { await store.disconnect() } }
                    .buttonStyle(.plain).foregroundStyle(.secondary)
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        else if let track = store.detailTrack { TrackDetailView(track: track, store: store, close: { store.detailTrack = nil }) }
        else if section == "connection" { ConnectionView(store: store) }
        else if section == "outputs" { DevicesView(store: store) }
        else if section == "dsp" { DspWorkspaceView(store:store,requestedZone:dspZone) }
        else if section == "client-settings" { ClientSettingsView(store: store) }
        else if section == "search" { SearchView(store: store) }
        else if section == "library" { LibraryView(store: store, openDaily: openDaily) }
        else if section == "daily-playlist", let dailyPlaylist {
            DailyPlaylistPage(store: store, snapshot: dailyPlaylist, backLabel: dailyOrigin == "library" ? "返回音乐空间" : "返回首页") {
                section = dailyOrigin
            }
        }
        else if section == "stats" { ListeningStatsView(store: store).id(store.sessionID) }
        else if section == "settings" { CoreSettingsView(store: store) }
        else if section == "browse" { LibraryBrowserView(store: store) }
        else if section == "discover" { DiscoverView(store: store) }
        else if section == "downloads" { DownloadsView(store: store) }
        else if section == "playlists" { PlaylistsView(store: store, create: { renaming = nil; playlistName = ""; editing = true }, rename: { renaming = $0; playlistName = $0.name; editing = true }, delete: { deleting = $0 }) }
        else { CollectionView(store: store) }
    }
    private func openDaily(_ snapshot: DailyPlaylistSnapshot) {
        dailyPlaylist = snapshot
        dailyOrigin = section
        section = "daily-playlist"
    }
    private func selectSection() {
        guard store.connected else { return }
        if section == "favorites" { Task { await store.perform { try await store.collection.refresh(store); try await store.collection.open(store.collection.favoriteID, store: store) } } }
        else if section.hasPrefix("playlist:") { let id = String(section.dropFirst(9)); Task { await store.perform { try await store.collection.open(id, store: store) } } }
        else { store.collection.selectedID = nil }
    }
    private func savePlaylist() { Task { await store.perform { if let renaming { try await store.collection.rename(renaming, name: playlistName, store: store) } else { try await store.collection.create(playlistName, store: store) } } } }
    private func banner(_ text: String, error: Bool) -> some View {
        HStack { Image(systemName: error ? "exclamationmark.circle" : "info.circle"); Text(text).font(.system(size: 10)).textSelection(.enabled); Spacer(); Button("关闭", systemImage: "xmark") { if error { store.error = nil } else { store.notice = nil } }.labelStyle(.iconOnly).buttonStyle(MusicControlStyle()) }.foregroundStyle(error ? Color.red : Color.secondary).padding(12)
    }
}

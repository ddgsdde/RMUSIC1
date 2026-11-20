import SwiftUI

/// Independent theme chrome; Core state and feature views remain shared with the native workspace.
struct MineradioWorkspaceView<Content: View>: View {
    @Bindable var store: MusicStore
    @Binding var section: String
    @Binding var player: Bool
    @Binding var queue: Bool
    @Binding var inspector: Bool
    let content: Content
    let openDaily: (DailyPlaylistSnapshot) -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @StoredState private var navigation: String?
    @SceneStorage("mineradio.sidebar.expanded") private var sidebarExpanded = true
    init(store: MusicStore, section: Binding<String>, player: Binding<Bool>, queue: Binding<Bool>, inspector: Binding<Bool>, openDaily: @escaping (DailyPlaylistSnapshot) -> Void, @ViewBuilder content: () -> Content) {
        self.store = store; _section = section; _player = player; _queue = queue; _inspector = inspector; self.openDaily = openDaily; self.content = content()
    }
    var body: some View {
        ZStack {
            Color(red: 0.025, green: 0.03, blue: 0.045)
            if store.connected {
                MineradioSurface(store: store, immersive: player, stageTop: player ? 150 : 0, stageBottom: player ? 134 : 0).ignoresSafeArea()
            }
            VStack(spacing: 24) {
                header
                ZStack(alignment: .trailing) {
                    ZStack {
                        if player { MineradioStageView(store: store).transition(.opacity) }
                        else {
                            HStack(alignment: .top, spacing: 20) {
                                MineradioSidebar(store: store, section: section, expanded: sidebarExpanded, navigate: navigate)
                                    .frame(width: sidebarExpanded ? 198 : 56)
                                ZStack {
                                    if section == "mineradio-home" && store.connected { MineradioHomeView(store: store, navigate: navigate, openPlayer: { player = true }, openDaily: openDaily).transition(.opacity) }
                                    else {
                                        content.frame(maxWidth: .infinity, maxHeight: .infinity)
                                            .background(MineradioMaterial()).clipShape(RoundedRectangle(cornerRadius: 26))
                                            .overlay(RoundedRectangle(cornerRadius: 26).stroke(.white.opacity(0.09), lineWidth: 1))
                                            .transition(.opacity.combined(with: reduceMotion ? .identity : .offset(y: 6)))
                                    }
                                }.frame(maxWidth: .infinity, maxHeight: .infinity)
                                    .animation(pageAnimation, value: section == "mineradio-home")
                            }.transition(.opacity.combined(with: reduceMotion ? .identity : .offset(y: 6)))
                        }
                    }.animation(reduceMotion ? .easeOut(duration: 0.1) : .smooth(duration: 0.3), value: player)
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
                .overlay(alignment: .trailing) {
                    ZStack(alignment: .trailing) {
                        if queue { PlaybackQueuePanel(store: store, close: { queue = false }).frame(width: 350).background(MineradioMaterial(cornerRadius: 24)).clipShape(RoundedRectangle(cornerRadius: 24)).shadow(color: .black.opacity(0.5), radius: 32).transition(.move(edge: .trailing).combined(with: .opacity)) }
                    }.animation(pageAnimation, value: queue)
                }
                .overlay(alignment: .trailing) {
                    ZStack(alignment: .trailing) {
                        if inspector { SignalPathView(store: store, close: { inspector = false }).frame(width: 370).background(MineradioMaterial(cornerRadius: 24)).clipShape(RoundedRectangle(cornerRadius: 24)).shadow(color: .black.opacity(0.5), radius: 32).transition(.move(edge: .trailing).combined(with: .opacity)) }
                    }.animation(pageAnimation, value: inspector)
                }
                VStack(spacing: 8) {
                    if let issue = store.connectionIssue ?? store.error { Text(issue).font(.caption).foregroundStyle(.orange).lineLimit(2).padding(.horizontal, 20) }
                    PlaybackRecoveryBanner(store: store)
                    MineradioTransportBar(store: store, immersive: player, openPlayer: { player.toggle() }, openQueue: { queue.toggle(); inspector = false })
                }.padding(.horizontal, 40)
            }.padding(.horizontal, 36).padding(.top, 28).padding(.bottom, 22)
        }.environment(\.colorScheme, .dark)
            .onChange(of: section) { navigation = nil }

    }
    private var pageAnimation: Animation { reduceMotion ? .easeOut(duration: 0.1) : .smooth(duration: 0.26) }
    private var header: some View {
        HStack(spacing: 22) {
            Button(player ? "返回首页" : sidebarExpanded ? "收起导航" : "展开导航", systemImage: player ? "chevron.down" : "sidebar.left") { if player { navigate("mineradio-home") } else { withAnimation(.easeInOut(duration: 0.18)) { sidebarExpanded.toggle() } } }.labelStyle(.iconOnly).buttonStyle(MineradioCircleButton())
            Button { navigation = navigation == "brand" ? nil : "brand" } label: {
                VStack(alignment: .leading, spacing: 3) { Text("R·MUSIC").font(.system(size: 14, weight: .semibold)).tracking(2); Text("MINERADIO").font(.system(size: 8)).tracking(2.5).foregroundStyle(.secondary) }
            }.buttonStyle(.plain).popover(isPresented: navigationBinding("brand"), arrowEdge: .bottom) { menu }
            Spacer(minLength: 20)
            HStack(spacing: 12) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                WorkspaceSearchField(text: $store.query, placeholder: "搜索歌曲、歌手、专辑", fontSize: 13).frame(height: 20)
                if !store.query.isEmpty { Button("清空搜索", systemImage: "xmark") { store.query = "" }.labelStyle(.iconOnly).buttonStyle(MusicControlStyle()) }
            }.padding(.horizontal, 20).frame(maxWidth: 620).frame(height: 46).background(MineradioMaterial(cornerRadius: 16)).clipShape(RoundedRectangle(cornerRadius: 16)).overlay(RoundedRectangle(cornerRadius: 16).stroke(.white.opacity(0.1), lineWidth: 1))
            Spacer(minLength: 20)
            PlaybackTransferButton(store: store)
            Menu {
                ForEach(store.playbackZones) { zone in Button { store.selectPlaybackZone(zone.id) } label: { Label(store.playbackOutputLabel(zone) + (store.onlineEndpoints.contains(zone.endpoint_id) ? "" : " · 离线"), systemImage: store.selectedZoneID == zone.id ? "checkmark.circle.fill" : "hifispeaker") }.disabled(!store.onlineEndpoints.contains(zone.endpoint_id) || store.systemSleeping) }
                Divider(); Button("管理声音设备") { navigate("outputs") }
            } label: {
                Label(store.zone.map { store.playbackOutputLabel($0) } ?? "选择声音设备", systemImage: "hifispeaker")
                    .font(.system(size: 11, weight: .medium)).lineLimit(1).truncationMode(.middle)
                    .frame(minWidth: 110, idealWidth: 190, maxWidth: 220, alignment: .trailing)
            }.menuStyle(.borderlessButton).help(store.zone.map { store.playbackOutputLabel($0) } ?? "选择声音设备")
            ThemeMenu()
        }.frame(height: 46)
    }
    private var menu: some View {
        MineradioSidebar(store: store, section: player ? "" : section, expanded: true, navigate: navigate)
            .frame(width: 240, height: 530).environment(\.colorScheme, .dark)
    }
    private func navigationBinding(_ anchor: String) -> Binding<Bool> { Binding(get: { navigation == anchor }, set: { if !$0 && navigation == anchor { navigation = nil } }) }
    private func navigate(_ id: String) { player = false; queue = false; inspector = false; store.query = ""; section = id; navigation = nil }
}
struct MineradioMaterial: View {
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    var cornerRadius: CGFloat = 26
    var body: some View {
        RoundedRectangle(cornerRadius: cornerRadius).fill(.ultraThinMaterial).opacity(reduceTransparency ? 1 : 0.32)
            .overlay {
                RoundedRectangle(cornerRadius: cornerRadius).fill(LinearGradient(colors: [MusicStyle.accent.opacity(0.08), Color.black.opacity(reduceTransparency ? 0.72 : 0.24)], startPoint: .topLeading, endPoint: .bottomTrailing))
            }
    }
}
struct MineradioCircleButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { configuration.label.font(.system(size: 15)).foregroundStyle(.white.opacity(configuration.isPressed ? 1 : 0.75)).frame(width: 38, height: 38).background(.white.opacity(configuration.isPressed ? 0.12 : 0.045), in: Circle()).overlay(Circle().stroke(.white.opacity(0.11), lineWidth: 1)) }
}

import SwiftUI

/// One connection workspace: verified targets, credentials, then the music library.
public struct ConnectionView: View {
    @Bindable var store: MusicStore
    private let fullWindow: Bool
    @StoredState private var inspector = CoreConnectionInspector()
    @StoredState private var local = CoreConnectionInspector()
    @StoredState private var refresh = UUID()
    @Environment(\.colorScheme) private var scheme
    public init(store: MusicStore, fullWindow: Bool = false) { self.store = store; self.fullWindow = fullWindow }
    private var candidate: CoreProfile {
        store.connectionTrust.applying(to: store.connectionBinding, primary: CoreConnectionAddress.normalize(store.lanAddress) ?? store.lanAddress, alternatives: [])
    }
    private var probeID: String { "\(store.connected):\(store.lanAddress):\(candidate.skip_certificate_verification == true):\(candidate.core_id ?? ""):\(refresh)" }
    public var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(spacing: 26) {
                    if fullWindow { BrandMark().padding(.bottom, 4) }
                    if store.workspaceReady { connectedCard.frame(maxWidth: 650) }
                    else {
                        ViewThatFits(in: .horizontal) {
                            HStack(alignment: .top, spacing: 22) { targets.frame(width: 310); form.frame(width: 450) }
                            VStack(spacing: 20) { targets; form }.frame(maxWidth: 520)
                        }
                    }
                }.frame(maxWidth: .infinity, minHeight: fullWindow ? max(0, geometry.size.height - 80) : 0).padding(fullWindow ? 40 : 24)
            }.scrollIndicators(.hidden)
        }
        .task(id: probeID) {
            guard !store.connected else { inspector.clear(); return }
            await inspector.inspect(candidate, store: store)
        }
        .task(id: "\(store.connected):\(refresh)") {
            guard !store.connected else { local.clear(); store.discovery.stop(); return }
            store.discovery.start(store)
            await local.inspect(CoreProfile(address: "http://127.0.0.1:18864"), store: store, debounce: false)
        }
        .onDisappear { inspector.clear(); local.clear(); store.discovery.stop() }
        .onChange(of: store.lanAddress) { old, new in store.connectionTrust.changed(from: old, to: new); if !store.connecting { store.error = nil } }
        .onChange(of: store.remoteAddress) { old, new in store.connectionTrust.changed(from: old, to: new) }
    }
    private var targets: some View { CoreConnectionTargetsView(store: store, local: local, selectedID: inspector.identity?.core_id ?? store.connectionBinding.core_id) { store.discovery.refresh(store); refresh = UUID() } }
    private var form: some View { CoreConnectionForm(store: store, inspector: inspector) { refresh = UUID() } }
    private var connectedCard: some View {
        VStack(alignment: .leading, spacing: 24) {
            Label(store.coreReachable ? "音乐库已连接" : "正在重新连接音乐库", systemImage: store.coreReachable ? "checkmark.circle.fill" : "arrow.triangle.2.circlepath").font(.callout).foregroundStyle(store.coreReachable ? Color.green : Color.orange)
            Text(store.profile.name).font(.system(size: 28, weight: .semibold))
            ConnectionQualityView(store: store)
            Divider()
            HStack { Label(store.username, systemImage: "person.crop.circle"); Spacer(); Text("\(store.outputs.enabled.count) 路输出已启用") }.font(.caption).foregroundStyle(.secondary)
            if store.outputs.roaming || !store.coreReachable {TravelAvailability(connected:store.coreReachable)}
            if store.bridgeOnly {Button("断开 Core"){Task{await store.disconnect()}}}
        }.padding(28).background(MusicStyle.panel(scheme), in: RoundedRectangle(cornerRadius: 24))
    }
}

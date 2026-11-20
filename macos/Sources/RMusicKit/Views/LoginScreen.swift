import SwiftUI

/// The authentication boundary occupies the whole window; no workspace controls exist beneath it.
struct LoginScreen: View {
    let store: MusicStore
    var coreUnavailable = false
    @Environment(\.colorScheme) private var scheme
    @StoredState private var devices = false
    @StoredState private var offline = false
    var body: some View {
        ZStack {
            MusicStyle.canvas(scheme).ignoresSafeArea()
            RadialGradient(colors: [MusicStyle.accent.opacity(scheme == .dark ? 0.07 : 0.045), .clear],
                           center: .center, startRadius: 50, endRadius: 540)
                .ignoresSafeArea().allowsHitTesting(false)
            if coreUnavailable {
                VStack(spacing: 16) {
                    Image(systemName: "network.slash").font(.system(size: 36, weight: .light)).foregroundStyle(.secondary)
                    Text("无法连接 Core").font(.title2.weight(.semibold))
                    Text(store.error ?? "请检查网络与 Core 服务。").foregroundStyle(.secondary)
                    HStack(spacing: 12) {
                        Button("退出登录并选择 Core") { Task { await store.disconnect() } }
                        Button(store.connecting ? "正在连接…" : "重试连接") { Task { await store.retrySavedConnection() } }
                            .buttonStyle(.borderedProminent).disabled(store.connecting)
                    }
                }.padding(36)
                .task {
                    for seconds in [2, 4, 8, 16] {
                        do { try await Task.sleep(for: .seconds(seconds)) } catch { return }
                        guard store.startupUnavailable else { return }
                        await store.retrySavedConnection()
                    }
                    while !Task.isCancelled && store.startupUnavailable {
                        do { try await Task.sleep(for: .seconds(30)) } catch { return }
                        await store.retrySavedConnection()
                    }
                }
            } else {
                ConnectionView(store: store, fullWindow: true)
            }
        }
        .overlay(alignment:.bottomTrailing) {if !store.bridgeOnly {HStack(spacing:12){Button("离线音乐",systemImage:"arrow.down.circle"){offline=true};Button("本机声音设备",systemImage:"hifispeaker"){devices=true}}.padding(24)}}
        .sheet(isPresented:$offline){OfflineListeningView(store:store)}
        .sheet(isPresented:$devices) {VStack {HStack {Spacer();Button("完成"){devices=false}}.padding();OutputsView(store:store)}.frame(width:700,height:540)}
        .overlay(alignment: .topTrailing) { ThemeMenu().padding(.top, 22).padding(.trailing, 28) }
        .accessibilityIdentifier("globalLogin")
    }
}

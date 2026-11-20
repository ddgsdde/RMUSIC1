import SwiftUI
import RMusicKit

@main
struct RMusicBridgeApp: App {
    @NSApplicationDelegateAdaptor(AppLifecycle.self) private var lifecycle
    @StoredState private var store: MusicStore
    init() {
        let store = MusicStore(bridgeOnly: true)
        _store = StoredState(initialValue: store)
        AppLifecycle.store = store
    }
    var body: some Scene {
        WindowGroup("R-Music 网桥", id: "bridge-settings") { ClientView(store: store) }
            .defaultSize(width: 1120, height: 780)
            .windowStyle(.hiddenTitleBar)
            .windowBackgroundDragBehavior(.enabled)
        MenuBarExtra("R-Music 网桥", systemImage: "hifispeaker.2") { BridgeMenu(store: store) }
    }
}

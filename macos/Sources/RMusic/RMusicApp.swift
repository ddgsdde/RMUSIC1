import SwiftUI
import RMusicKit

@main
struct RMusicApp: App {
    @NSApplicationDelegateAdaptor(AppLifecycle.self) private var lifecycle
    @StoredState private var store: MusicStore
    init() {
        let store = MusicStore()
        _store = StoredState(initialValue: store)
        AppLifecycle.store = store
    }
    var body: some Scene {
        WindowGroup("R-Music", id: "main") { ClientView(store: store) }
            .defaultSize(width: 1360, height: 860)
            .windowStyle(.hiddenTitleBar)
            .windowBackgroundDragBehavior(.enabled)
        Settings { ClientSettingsViewHost(store: store) }
    }
}

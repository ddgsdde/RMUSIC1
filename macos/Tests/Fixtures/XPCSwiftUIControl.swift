// Minimal framework control: no RMusicKit, Core, audio, network or file panels.
import SwiftUI
@main struct XPCSwiftUIControl: App {
    var body: some Scene {
        WindowGroup { Text("SwiftUI XPC control").frame(width: 360, height: 180) }
    }
}

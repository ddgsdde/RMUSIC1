// Minimal AppKit control: intentionally does not load SwiftUI or RMusicKit.
import AppKit
@main @MainActor final class XPCAppKitControl: NSObject, NSApplicationDelegate {
    var window: NSWindow?
    static func main() {
        let app = NSApplication.shared
        let delegate = XPCAppKitControl()
        app.delegate = delegate
        app.setActivationPolicy(.regular)
        app.run()
        withExtendedLifetime(delegate) {}
    }
    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 360, height: 180), styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window?.title = "AppKit XPC control"
        window?.center()
        window?.makeKeyAndOrderFront(nil)
    }
}

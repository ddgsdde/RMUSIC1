import AppKit

/// AppKit is confined to process lifetime and sleep notifications; views own no native windows.
@MainActor
public final class AppLifecycle: NSObject, NSApplicationDelegate {
    public static var store: MusicStore?
    private var power: SystemPowerMonitor?
    private var terminating = false
    public func applicationDidFinishLaunching(_ notification: Notification) {
        let menuOnly = Bundle.main.object(forInfoDictionaryKey: "LSUIElement") as? Bool ?? false
        NSApp.setActivationPolicy(menuOnly ? .accessory : .regular)
        // Development bundles can retain Launch Services' generic executable icon.
        // Set the running application's Dock image from our packaged resource too.
        if let url = Bundle.main.url(forResource: "RMusic", withExtension: "icns"),
           let icon = NSImage(contentsOf: url), icon.isValid {
            icon.isTemplate = false
            NSApp.applicationIconImage = icon
        }
        if !menuOnly { NSApp.activate(ignoringOtherApps: true) }
        power = SystemPowerMonitor { sleeping in
            if sleeping { await Self.store?.suspend() }
            else { await Self.store?.wake() }
        }
    }
    public func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard !terminating else { return .terminateLater }
        terminating = true
        power?.stop(); power = nil
        // Defer until after terminateLater has returned, including when no child was started.
        DispatchQueue.main.async {
            Task { @MainActor in await Self.store?.shutdown(); NSApp.reply(toApplicationShouldTerminate: true) }
        }
        return .terminateLater
    }
    public func applicationWillTerminate(_ notification: Notification) {
        power?.stop(); power = nil
    }
    public func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
}

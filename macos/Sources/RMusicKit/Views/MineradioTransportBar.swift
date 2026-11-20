import SwiftUI

/// Keeps a stable hover target while the stage controls are transparent.
struct MineradioTransportBar: View {
    let store: MusicStore
    let immersive: Bool
    var openPlayer: () -> Void
    var openQueue: () -> Void
    @Environment(\.scenePhase) private var phase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @AppStorage("mineradio.autoHideTransport") private var autoHideTransport = true
    @StoredState private var hovering = false
    @StoredState private var interacting = false
    @StoredState private var revealed = true
    private var canFade: Bool {
        autoHideTransport && immersive && store.zone?.status == "playing" && phase == .active && !hovering && !interacting
            && store.connectionIssue == nil && store.error == nil && store.details.outputError == nil
    }
    var body: some View {
        TransportView(store: store, openPlayer: openPlayer, openQueue: openQueue, floatingBackground: true, interactionChanged: { interacting = $0 })
            .clipShape(RoundedRectangle(cornerRadius: 26))
            .overlay(RoundedRectangle(cornerRadius: 26).stroke(.white.opacity(0.13), lineWidth: 1))
            .shadow(color: .black.opacity(0.4), radius: 28, y: 14)
            .opacity(revealed ? 1 : 0)
            .animation(reduceMotion ? nil : .easeInOut(duration: revealed ? 0.16 : 0.45), value: revealed)
            .allowsHitTesting(revealed)
            .background {
                TransportHoverRegion { inside in
                    hovering = inside
                    if inside { revealed = true }
                }
            }
        .task(id: canFade) {
            revealed = true
            guard canFade else { return }
            do { try await Task.sleep(for: .seconds(4)); try Task.checkCancellation(); revealed = false }
            catch { }
        }
    }
}

/// AppKit keeps tracking this fixed region even when SwiftUI omits transparent controls.
/// A local event monitor also observes moves handled by the overlapping Metal surface.
/// It passes every event through and is removed as soon as the region leaves its window.
private struct TransportHoverRegion: NSViewRepresentable {
    let changed: (Bool) -> Void

    func makeNSView(context: Context) -> HoverView { HoverView() }
    func updateNSView(_ view: HoverView, context: Context) {
        view.changed = changed
    }
    static func dismantleNSView(_ view: HoverView, coordinator: ()) {
        view.stopObserving()
        view.changed = nil
    }

    @MainActor final class HoverView: NSView {
        var changed: ((Bool) -> Void)?
        private var tracking: NSTrackingArea?
        private var monitor: Any?
        private var inside = false

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            stopObserving()
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: [.mouseMoved, .leftMouseDragged, .rightMouseDragged, .leftMouseDown, .rightMouseDown, .scrollWheel]) { [weak self] event in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    self.setInside(event.window === self.window && self.bounds.contains(self.convert(event.locationInWindow, from: nil)))
                }
                return event
            }
        }
        func stopObserving() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
            inside = false
        }

        override func updateTrackingAreas() {
            super.updateTrackingAreas()
            if let tracking { removeTrackingArea(tracking) }
            let area = NSTrackingArea(rect: .zero,
                options: [.mouseEnteredAndExited, .mouseMoved, .activeInActiveApp, .inVisibleRect, .enabledDuringMouseDrag],
                owner: self, userInfo: nil)
            addTrackingArea(area)
            tracking = area
        }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func mouseEntered(with event: NSEvent) { setInside(true) }
        override func mouseMoved(with event: NSEvent) { setInside(true) }
        override func mouseExited(with event: NSEvent) { setInside(false) }
        private func setInside(_ value: Bool) {
            guard inside != value else { return }
            inside = value
            changed?(value)
        }
    }
}

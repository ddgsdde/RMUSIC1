import AppKit
import SwiftUI

func wheelVolume(_ value: Double, delta: Double, precise: Bool) -> Double {
    min(1, max(0, value + min(0.08, max(-0.08, delta * (precise ? 0.002 : 0.02)))))
}

struct VolumeWheelInput: NSViewRepresentable {
    var enabled: Bool
    var change: (Double, Bool) -> Void
    func makeNSView(context: Context) -> WheelView { WheelView() }
    func updateNSView(_ view: WheelView, context: Context) { view.enabled = enabled; view.change = change }
    static func dismantleNSView(_ view: WheelView, coordinator: ()) { view.stop() }
    @MainActor final class WheelView: NSView {
        var enabled = false
        var change: ((Double, Bool) -> Void)?
        private var monitor: Any?
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow(); stop()
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
                let consumed = MainActor.assumeIsolated {
                    guard let self, self.enabled, event.window === self.window,
                          self.bounds.contains(self.convert(event.locationInWindow, from: nil)),
                          event.scrollingDeltaY != 0 else { return false }
                    self.change?(Double(event.scrollingDeltaY), event.hasPreciseScrollingDeltas)
                    return true
                }
                return consumed ? nil : event
            }
        }
        func stop() { if let monitor { NSEvent.removeMonitor(monitor) }; monitor = nil }
    }
}

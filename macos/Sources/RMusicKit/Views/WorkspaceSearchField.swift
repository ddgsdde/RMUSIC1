import AppKit
import SwiftUI

/// Keep AppKit's field editor in charge of candidate selection.
struct WorkspaceSearchField: NSViewRepresentable {
    @Binding var text: String
    let placeholder: String
    let fontSize: CGFloat

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeNSView(context: Context) -> NSTextField {
        let field = NSTextField()
        field.delegate = context.coordinator
        field.placeholderString = placeholder
        field.font = .systemFont(ofSize: fontSize)
        field.isBordered = false
        field.drawsBackground = false
        field.usesSingleLineMode = true
        field.setAccessibilityLabel(placeholder)
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return field
    }

    func updateNSView(_ field: NSTextField, context: Context) {
        context.coordinator.text = $text
        if (field.currentEditor() as? NSTextView)?.hasMarkedText() != true, field.stringValue != text {
            field.stringValue = text
        }
    }

    static func dismantleNSView(_ field: NSTextField, coordinator: Coordinator) {
        coordinator.stopMonitoring()
    }

    @MainActor final class Coordinator: NSObject, NSTextFieldDelegate {
        var text: Binding<String>
        private var monitor: Any?
        private var compositionEventTime: TimeInterval?

        init(text: Binding<String>) { self.text = text }

        func controlTextDidBeginEditing(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            stopMonitoring()
            monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown, .keyDown]) { [weak self, weak field] event in
                let consumed = MainActor.assumeIsolated {
                    guard let self, let field, let window = field.window, event.window === window,
                          let editor = field.currentEditor() as? NSTextView, window.firstResponder === editor else { return false }
                    if event.type == .keyDown {
                        self.compositionEventTime = editor.hasMarkedText() ? event.timestamp : nil
                        if self.compositionEventTime == nil && [36, 53, 76].contains(event.keyCode) {
                            self.releaseFocus(field)
                            return true
                        }
                    } else if !field.bounds.contains(field.convert(event.locationInWindow, from: nil)) {
                        self.releaseFocus(field)
                    }
                    return false
                }
                return consumed ? nil : event
            }
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField,
                  let editor = field.currentEditor() as? NSTextView, !editor.hasMarkedText() else { return }
            if text.wrappedValue != editor.string { text.wrappedValue = editor.string }
        }

        func controlTextDidEndEditing(_ notification: Notification) {
            stopMonitoring()
            guard let field = notification.object as? NSTextField else { return }
            if text.wrappedValue != field.stringValue { text.wrappedValue = field.stringValue }
        }

        func control(_ field: NSControl, textView: NSTextView, doCommandBy command: Selector) -> Bool {
            guard command == #selector(NSResponder.insertNewline(_:)) || command == #selector(NSResponder.cancelOperation(_:)) else { return false }
            if textView.hasMarkedText() { return false }
            if let compositionEventTime, compositionEventTime == NSApp.currentEvent?.timestamp { return true }
            releaseFocus(field)
            return true
        }

        private func releaseFocus(_ field: NSControl) {
            guard let window = field.window, let editor = field.currentEditor(), window.firstResponder === editor else { return }
            if let content = window.contentView, window.makeFirstResponder(content) { return }
            window.makeFirstResponder(nil)
        }

        func stopMonitoring() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
            compositionEventTime = nil
        }
    }
}

import AppKit
import SwiftUI
import Testing
@testable import RMusicKit

@Test @MainActor func searchRetainsMarkedTextAndReleasesTheFieldEditor() throws {
    _ = NSApplication.shared
    let frame = NSRect(x:0,y:0,width:300,height:110)
    let window = NSWindow(contentRect:frame,styleMask:[.borderless],backing:.buffered,defer:false)
    window.isReleasedWhenClosed = false
    defer { window.close() }
    let field = NSTextField(frame:NSRect(x:15,y:20,width:260,height:24))
    let delegate = WorkspaceSearchField.Coordinator(text:.constant(""))
    field.delegate = delegate
    window.contentView?.addSubview(field)
    #expect(window.makeFirstResponder(field))
    let editor = try #require(field.currentEditor() as? NSTextView)
    editor.setMarkedText("拼", selectedRange:NSRange(location:1,length:0), replacementRange:NSRange(location:NSNotFound,length:0))
    #expect(editor.hasMarkedText())
    #expect(!delegate.control(field,textView:editor,doCommandBy:#selector(NSResponder.insertNewline(_:))))
    #expect(window.firstResponder === editor)
    editor.unmarkText()
    #expect(delegate.control(field,textView:editor,doCommandBy:#selector(NSResponder.cancelOperation(_:))))
    #expect(window.firstResponder !== editor)
}

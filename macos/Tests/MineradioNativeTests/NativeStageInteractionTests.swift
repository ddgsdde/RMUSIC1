import AppKit
import MetalKit
import Testing
@testable import MineradioNative

@Test @MainActor func nativeStagePlainDragChangesCameraWithoutMovingAWindow() throws {
    let view = NativeMetalView(frame: NSRect(x: 0, y: 0, width: 900, height: 600), device: try #require(MTLCreateSystemDefaultDevice()))
    let state = NativeStageState(preset: 0, fps: 30, density: 80, scale: 0.5, intensity: 0.85, active: false, visible: false, immersive: true, parameters: "{}", position: 0, playing: false, cover: nil)
    let renderer = try NativeMetalRenderer(view: view, state: state, failed: { _ in })
    defer { renderer.close() }
    view.renderer = renderer
    let before = renderer.camera.view(cinema: false)
    view.mouseDown(with: try stageMouse(.leftMouseDown, at: NSPoint(x: 450, y: 300)))
    view.mouseDragged(with: try stageMouse(.leftMouseDragged, at: NSPoint(x: 480, y: 312), delta: NSPoint(x: 30, y: 12)))
    view.mouseUp(with: try stageMouse(.leftMouseUp, at: NSPoint(x: 480, y: 312)))
    renderer.camera.step(1)
    #expect(renderer.camera.view(cinema: false) != before, "舞台普通拖动必须调整视角，不需要 Option")
}

@Test @MainActor func nativeStageTopEdgeNeverRotatesAndGesturesKeepTheirOriginalTarget() throws {
    let view = NativeMetalView(frame: NSRect(x: 0, y: 0, width: 900, height: 600), device: try #require(MTLCreateSystemDefaultDevice()))
    let state = NativeStageState(preset: 0, fps: 30, density: 80, scale: 0.5, intensity: 0.85, active: false, visible: false, immersive: true, parameters: "{}", position: 0, playing: false, cover: nil)
    let renderer = try NativeMetalRenderer(view: view, state: state, failed: { _ in })
    defer { renderer.close() }
    view.renderer = renderer
    renderer.camera.drag(x: 50, y: 15); renderer.camera.step(10)
    let before = renderer.camera.view(cinema: false)
    let top = NSPoint(x: 450, y: 590), stage = NSPoint(x: 450, y: 300)
    view.mouseDown(with: try stageMouse(.leftMouseDown, at: top))
    view.mouseDragged(with: try stageMouse(.leftMouseDragged, at: stage, delta: NSPoint(x: 50, y: 290)))
    view.mouseUp(with: try stageMouse(.leftMouseUp, at: stage, clicks: 2))
    renderer.camera.step(10)
    #expect(renderer.camera.view(cinema: false) == before, "窗口拖动即使进入舞台也不能旋转或复位")
    view.mouseDown(with: try stageMouse(.leftMouseDown, at: stage))
    view.mouseDragged(with: try stageMouse(.leftMouseDragged, at: top, delta: NSPoint(x: 20, y: -290)))
    view.mouseUp(with: try stageMouse(.leftMouseUp, at: top))
    renderer.camera.step(10)
    #expect(renderer.camera.view(cinema: false) != before, "舞台拖动跨入上边缘也应继续调整视角")
    let rotated = renderer.camera.view(cinema: false)
    view.mouseDown(with: try stageMouse(.leftMouseDown, at: top, clicks: 2))
    view.mouseUp(with: try stageMouse(.leftMouseUp, at: top, clicks: 2))
    #expect(renderer.camera.view(cinema: false) == rotated, "上边缘双击不能复位舞台")
    view.mouseDown(with: try stageMouse(.leftMouseDown, at: stage, clicks: 2))
    view.mouseUp(with: try stageMouse(.leftMouseUp, at: stage, clicks: 2))
    var original = NativeCamera(); original.reset(preset: 0)
    #expect(renderer.camera.view(cinema: false) == original.view(cinema: false))
    view.mouseDragged(with: try stageMouse(.leftMouseDragged, at: stage, delta: NSPoint(x: 100, y: 100)))
    renderer.camera.step(10)
    #expect(renderer.camera.view(cinema: false) == original.view(cinema: false), "已结束的手势不能继续旋转")
}

@Test @MainActor func nativeStageDispatchesWindowDraggingOnlyFromTheTopEdge() throws {
    _ = NSApplication.shared
    let window = StageDragWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 600), styleMask: .borderless, backing: .buffered, defer: false)
    let view = NativeMetalView(frame: NSRect(x: 0, y: 0, width: 900, height: 600), device: nil)
    window.contentView = view
    defer { view.stopObserving(); window.contentView = nil }
    let stage = NSPoint(x: 450, y: 300), top = NSPoint(x: 450, y: 590)
    view.mouseDown(with: try stageMouse(.leftMouseDown, at: stage))
    view.mouseDragged(with: try stageMouse(.leftMouseDragged, at: top, delta: NSPoint(x: 20, y: -290)))
    view.mouseUp(with: try stageMouse(.leftMouseUp, at: top))
    #expect(window.dragCalls == 0)
    view.mouseDown(with: try stageMouse(.leftMouseDown, at: top))
    view.mouseDragged(with: try stageMouse(.leftMouseDragged, at: stage, delta: NSPoint(x: 20, y: 290)))
    #expect(window.dragCalls == 1)
    view.mouseDragged(with: try stageMouse(.leftMouseDragged, at: stage, delta: NSPoint(x: 20, y: 10)))
    #expect(window.dragCalls == 1, "AppKit 消费鼠标抬起后，旧手势不能重复开始窗口拖动")
    view.mouseUp(with: try stageMouse(.leftMouseUp, at: stage))
}

@MainActor private final class StageDragWindow: NSWindow {
    var dragCalls = 0
    override func performDrag(with event: NSEvent) { dragCalls += 1 }
}

@MainActor private func stageMouse(_ type: NSEvent.EventType, at point: NSPoint, delta: NSPoint = .zero, clicks: Int = 1) throws -> NSEvent {
    let event = try #require(NSEvent.mouseEvent(with: type, location: point, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, eventNumber: 1, clickCount: clicks, pressure: 1))
    let cg = try #require(event.cgEvent)
    cg.setDoubleValueField(.mouseEventDeltaX, value: delta.x)
    cg.setDoubleValueField(.mouseEventDeltaY, value: delta.y)
    return try #require(NSEvent(cgEvent: cg))
}

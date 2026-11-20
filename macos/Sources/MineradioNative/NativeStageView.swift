import SwiftUI
@preconcurrency import MetalKit

public struct NativeStageView: NSViewRepresentable {
    public let state: NativeStageState
    public var failed: (String) -> Void
    public var visibility: (Bool) -> Void
    public var focus: (Bool) -> Void
    public var selectQueue: (Int,String) -> Void
    public var queueWindow: ([Int]) -> Void
    public init(state: NativeStageState, failed: @escaping (String) -> Void, visibility: @escaping (Bool) -> Void = { _ in }, focus: @escaping (Bool) -> Void = { _ in }, selectQueue: @escaping (Int,String) -> Void = { _,_ in }, queueWindow: @escaping ([Int]) -> Void = { _ in }) {
        self.state = state; self.failed = failed; self.visibility = visibility; self.focus = focus
        self.selectQueue = selectQueue; self.queueWindow = queueWindow
    }
    public func makeNSView(context: Context) -> NativeMetalView {
        let view = NativeMetalView(frame: .zero, device: MTLCreateSystemDefaultDevice())
        view.colorPixelFormat = .bgra8Unorm
        // The renderer owns transient MSAA/depth attachments; MTKView owns only
        // the resolved drawable, avoiding a second persistent attachment pair.
        view.depthStencilPixelFormat = .invalid
        view.clearColor = MTLClearColorMake(0.025,0.03,0.045,1)
        view.framebufferOnly = true; view.autoResizeDrawable = false
        view.visibility = visibility
        view.focus = focus
        do {
            let renderer = try NativeMetalRenderer(view: view, state: state, failed: failed)
            renderer.shelf.select = selectQueue; renderer.shelf.windowChanged = queueWindow
            context.coordinator.renderer = renderer; view.delegate = renderer; view.renderer = renderer
        } catch { failed(String(describing: error)) }
        return view
    }
    public func updateNSView(_ view: NativeMetalView, context: Context) {
        view.visibility = visibility; view.focus = focus
        view.resolutionScale = state.scale
        context.coordinator.renderer?.shelf.select = selectQueue
        context.coordinator.renderer?.shelf.windowChanged = queueWindow
        context.coordinator.renderer?.update(state)
    }
    public func makeCoordinator() -> Coordinator { Coordinator() }
    public static func dismantleNSView(_ view: NativeMetalView, coordinator: Coordinator) {
        view.stopObserving(); coordinator.renderer?.close(); coordinator.renderer = nil
    }
    @MainActor public final class Coordinator { var renderer: NativeMetalRenderer? }
}

@MainActor public final class NativeMetalView: MTKView {
    weak var renderer: NativeMetalRenderer?
    var visibility: (Bool) -> Void = { _ in }
    var focus: (Bool) -> Void = { _ in }
    private var observers: [NSObjectProtocol] = []
    private var tracking: NSTrackingArea?
    private var dragDistance: CGFloat = 0
    private var initialDrag: NSEvent?
    private enum DragTarget { case window, camera }
    private var dragTarget: DragTarget?
    var resolutionScale: Double = 1 { didSet { resizeDrawable() } }
    public override func layout() { super.layout(); resizeDrawable() }
    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow(); stopObserving()
        guard let window else { renderer?.setOccluded(true); visibility(false); return }
        for name in [NSWindow.didChangeOcclusionStateNotification,NSWindow.didMiniaturizeNotification,NSWindow.didDeminiaturizeNotification,NSWindow.didBecomeKeyNotification,NSWindow.didResignKeyNotification] {
        observers.append(NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let visible = self.window?.occlusionState.contains(.visible) == true && self.window?.isMiniaturized == false
                self.renderer?.setOccluded(!visible); self.visibility(visible); self.focus(self.window?.isKeyWindow == true)
            }
        })}
        renderer?.setOccluded(!window.occlusionState.contains(.visible) || window.isMiniaturized)
        // A reused surface may have gone offscreen before this view was attached.
        // An already-visible window won't send another occlusion transition for us.
        Task { @MainActor [weak self, weak window] in
            guard let self, let window, self.window === window else { return }
            self.visibility(window.occlusionState.contains(.visible) && !window.isMiniaturized); self.focus(window.isKeyWindow)
        }
    }
    func stopObserving() { for observer in observers { NotificationCenter.default.removeObserver(observer) }; observers = [] }
    private func resizeDrawable() {
        let pixels = max(1,bounds.width*bounds.height)
        let ratio = min(1.35,sqrt(5_200_000/pixels))*max(0.5,min(1,resolutionScale))
        let size = CGSize(width: max(1,bounds.width*ratio),height: max(1,bounds.height*ratio))
        if drawableSize != size { drawableSize = size }
    }
    public override var mouseDownCanMoveWindow: Bool { false }
    public override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect:.zero,options:[.mouseMoved,.mouseEnteredAndExited,.activeInKeyWindow,.inVisibleRect],owner:self,userInfo:nil)
        addTrackingArea(area); tracking = area
    }
    private func point(_ event: NSEvent) -> SIMD2<Float> {
        let p = convert(event.locationInWindow,from:nil)
        let y = Float(p.y/max(1,bounds.height))*2-1
        return SIMD2(Float(p.x/max(1,bounds.width))*2-1,isFlipped ? -y : y)
    }
    // AppKit coordinates are in points, so the title strip stays the same size
    // on Retina displays. Choose the gesture once, at mouse-down.
    private func isWindowDragArea(_ event: NSEvent) -> Bool {
        let p = convert(event.locationInWindow, from: nil)
        let fromTop = isFlipped ? p.y - bounds.minY : bounds.maxY - p.y
        return fromTop >= 0 && fromTop < 44
    }
    public override func mouseMoved(with event: NSEvent) {
        if NSEvent.pressedMouseButtons & 1 == 0 { renderer?.pointer(isWindowDragArea(event) ? nil : point(event)) }
    }
    public override func mouseExited(with event: NSEvent) { renderer?.pointer(nil) }
    public override func mouseDown(with event: NSEvent) {
        dragDistance = 0
        dragTarget = isWindowDragArea(event) ? .window : .camera
        initialDrag = dragTarget == .window ? event : nil
        renderer?.pointer(nil)
    }
    public override func mouseUp(with event: NSEvent) {
        defer { initialDrag = nil; dragTarget = nil; dragDistance = 0 }
        guard dragTarget == .camera, dragDistance < 4 else { return }
        if renderer?.shelf.click(point(event)) == true { return }
        if event.clickCount == 2 { renderer?.resetCamera(); setNeedsDisplay(bounds) }
    }
    public override func mouseDragged(with event: NSEvent) {
        guard let dragTarget else { return }
        dragDistance += abs(event.deltaX)+abs(event.deltaY)
        if dragTarget == .camera { renderer?.camera.drag(x:Float(event.deltaX),y:Float(event.deltaY)); setNeedsDisplay(bounds) }
        else if let initialDrag {
            self.initialDrag = nil; renderer?.pointer(nil)
            window?.performDrag(with: initialDrag)
            // AppKit may consume mouse-up while moving the window.
            self.dragTarget = nil
        }
    }
    public override func scrollWheel(with event: NSEvent) {
        guard !isWindowDragArea(event), dragTarget != .window else { return }
        if renderer?.scrollShelf(-Float(event.scrollingDeltaY),at:point(event)) != true { renderer?.camera.zoom(-Float(event.scrollingDeltaY)) }
        setNeedsDisplay(bounds)
    }
}

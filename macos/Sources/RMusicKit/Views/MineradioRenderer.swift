#if !RMUSIC_NATIVE_MINERADIO
import SwiftUI
import WebKit
import OSLog
import RMusicVisualAssets

/// Only the GPU surface crosses into WebKit; navigation, controls and playback remain SwiftUI/Rust.
struct MineradioRenderer: NSViewRepresentable {
    typealias Configuration = MineradioConfiguration
    let configuration: Configuration
    let cover: String?
    let frames: [VisualFeature]
    let playback: MineradioPlayback
    let lyrics: [MineradioLyric]
    let queue: [MineradioQueueItem]
    let selectQueue: (Int, String) -> Void
    let failed: () -> Void
    var visibility: (Bool) -> Void = { _ in }
    var focus: (Bool) -> Void = { _ in }
    var queueWindow: ([Int]) -> Void = { _ in }
    func makeCoordinator() -> Coordinator { Coordinator(failed: failed, visibility: visibility, selectQueue: selectQueue, queueWindow: queueWindow) }
    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.preferences.isElementFullscreenEnabled = false
        config.mediaTypesRequiringUserActionForPlayback = .all
        config.userContentController.add(context.coordinator, name: "visualStatus")
        let view = WKWebView(frame: .zero, configuration: config)
        view.navigationDelegate = context.coordinator
        context.coordinator.view = view
        guard let directory = Self.assetDirectory else { failed(); return view }
        context.coordinator.root = directory
        view.loadFileURL(directory.appendingPathComponent("index.html"), allowingReadAccessTo: directory)
        return view
    }
    func updateNSView(_ view: WKWebView, context: Context) { context.coordinator.update(configuration, cover: cover, frames: frames, playback: playback, lyrics: lyrics, queue: queue) }
    static func dismantleNSView(_ view: WKWebView, coordinator: Coordinator) {
        coordinator.closed = true
        coordinator.visibilityObserver.map(NotificationCenter.default.removeObserver)
        view.evaluateJavaScript("window.rmusicVisual?.dispose()", completionHandler: nil)
        view.stopLoading(); view.navigationDelegate = nil
        view.configuration.userContentController.removeScriptMessageHandler(forName: "visualStatus")
        view.loadHTMLString("", baseURL: nil)
        coordinator.view = nil
    }
    private static var assetDirectory: URL? {
        if let url = Bundle.main.resourceURL?.appendingPathComponent("Mineradio"), FileManager.default.fileExists(atPath: url.appendingPathComponent("index.html").path) { return url }
        return RMusicVisualAssets.directory
    }
    @MainActor final class Coordinator: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
        private let logger = Logger(subsystem: "com.rmusic.mac.client", category: "Mineradio")
        weak var view: WKWebView?
        var root: URL?
        var visibilityObserver: NSObjectProtocol?
        var closed = false
        private var ready = false
        private var busy = false
        private var dirty = false
        private var configuration: Configuration?
        private var cover: String?
        private var frames: [VisualFeature] = []
        private var playback: MineradioPlayback?
        private var lyrics: [MineradioLyric] = []
        private var queue: [MineradioQueueItem] = []
        private var sentQueue: [MineradioQueueItem]?
        private var sentPlayback: MineradioPlayback?
        private var sentLyrics: [MineradioLyric]?
        private var sentConfiguration: Configuration?
        private var sentCover: String?
        private var sentFrames: [VisualFeature]?
        private let failed: () -> Void
        private let visibility: (Bool) -> Void
        private let queueWindow: ([Int]) -> Void
        private let selectQueue: (Int, String) -> Void
        init(failed: @escaping () -> Void, visibility: @escaping (Bool) -> Void, selectQueue: @escaping (Int, String) -> Void, queueWindow: @escaping ([Int]) -> Void) { self.failed = failed; self.visibility = visibility; self.selectQueue = selectQueue; self.queueWindow = queueWindow }
        func update(_ config: Configuration, cover: String?, frames: [VisualFeature], playback: MineradioPlayback, lyrics: [MineradioLyric], queue: [MineradioQueueItem]) {
            configuration = config; self.cover = cover; self.frames = frames; self.playback = playback; self.lyrics = lyrics; self.queue = queue
            if visibilityObserver == nil, let view, let window = view.window {
                visibilityObserver = NotificationCenter.default.addObserver(forName: NSWindow.didChangeOcclusionStateNotification, object: window, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { if let self { self.visibility(self.view?.window?.occlusionState.contains(.visible) == true); self.sentConfiguration = nil; self.flush() } }
                }
            }
            flush()
        }
        private func flush() {
            guard ready, !closed, let view, var config = configuration else { return }
            if let window = view.window, !window.occlusionState.contains(.visible) { config.active = false; config.visible = false }
            if busy { dirty = true; return }
            var commands: [String] = []; var arguments: [String: Any] = [:]
            if config != sentConfiguration {
                logger.debug("Stage activity: draw=\(config.active), visible=\(config.visible), appForeground=\(NSApp.isActive), windowVisible=\(view.window?.occlusionState.contains(.visible) == true)")
                arguments["config"] = config.dictionary; arguments["active"] = config.active
                commands.append("window.rmusicVisual.configure(config); window.rmusicVisual.activity(active);")
                sentConfiguration = config
            }
            if sentCover != cover { arguments["cover"] = cover ?? ""; commands.append("window.rmusicVisual.cover(cover);"); sentCover = cover }
            if playback != sentPlayback, let playback { arguments["metadata"] = playback.dictionary; commands.append("window.rmusicVisual.metadata(metadata);"); sentPlayback = playback }
            if queue != sentQueue { arguments["queue"] = queue.map(\.dictionary); commands.append("window.rmusicVisual.queue(queue);"); sentQueue = queue }
            if lyrics != sentLyrics { arguments["lyrics"] = lyrics.map(\.dictionary); commands.append("window.rmusicVisual.lyrics(lyrics);"); sentLyrics = lyrics }
            if frames != sentFrames { arguments["batch"] = ["frames": config.active ? frames.filter(\.valid).prefix(24).map(\.dictionary) : []]; commands.append("window.rmusicVisual.audio(batch);"); sentFrames = frames }
            guard !commands.isEmpty else { return }
            busy = true
            view.callAsyncJavaScript(commands.joined(separator: "\n"), arguments: arguments, in: nil, in: .page) { [weak self] result in
                guard let self, !self.closed else { return }; self.busy = false
                if case .failure(let error) = result { self.logger.error("Visual command failed: \(String(describing: error), privacy: .public)"); self.failed(); return }
                if self.dirty { self.dirty = false; self.flush() }
            }
        }
        func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
            guard !closed, message.frameInfo.isMainFrame, let item = message.body as? [String: Any] else { return }
            if item["name"] as? String == "queueSelect", let value = item["value"] as? [String: Any], let index = value["index"] as? Int, let id = value["id"] as? String, queue.contains(where: { $0.index == index && $0.id == id }) { selectQueue(index, id) }
            if item["name"] as? String == "queueWindow", let indices = item["value"] as? [Int], indices.count <= 11, indices.allSatisfy({ index in queue.contains(where: { $0.index == index }) }) { queueWindow(indices) }
            if item["name"] as? String == "ready" { ready = true; flush() }
            if item["name"] as? String == "metrics", Bundle.main.bundleIdentifier?.hasPrefix("com.rmusic.mac.client.qa") == true, let value = item["value"], JSONSerialization.isValidJSONObject(value), let data = try? JSONSerialization.data(withJSONObject: value), data.count <= 4096, let text = String(data: data, encoding: .utf8) { logger.info("Stage metrics: \(text, privacy: .public)") }
            if item["name"] as? String == "rendererError" { logger.error("Visual JavaScript failed: \(String(describing: item["value"]), privacy: .public)"); failed() }
            if item["name"] as? String == "contextLost" { failed() }
        }
        func webViewWebContentProcessDidTerminate(_ webView: WKWebView) { if !closed { failed() } }
        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { if !closed { failed() } }
        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
            let allowed = action.request.url.map { url in url.isFileURL && root.map { root in url.standardizedFileURL.path.hasPrefix(root.standardizedFileURL.path + "/") } == true } ?? false
            return allowed ? .allow : .cancel
        }
    }
}

#endif

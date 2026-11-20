import SwiftUI
import AppKit

struct MineradioSurface: View {
    let store: MusicStore
    var immersive = false
    var live = true
    var stageTop = 0.0
    var stageBottom = 0.0
    @StoredState private var features = VisualFeatureStore()
    @StoredState private var queueArtwork = MineradioQueueArtwork()
    @StoredState private var shelfWindow: [Int] = []
    @StoredState private var imageData: String?
    @StoredState private var renderFailed = false
    @StoredState private var visible = true
    @StoredState private var foreground = NSApp.isActive
    @StoredState private var windowFocused = true
    @StoredState private var parameterSnapshot = "{}"
    @StoredState private var lyricPayload: [MineradioLyric] = []
    @AppStorage("mineradio.parameters") private var parameters = "{}"
    @AppStorage("mineradio.preset") private var preset = 0
    @AppStorage("mineradio.fps") private var fps = 60
    @AppStorage("mineradio.particles") private var particles = 180
    @AppStorage("mineradio.scale") private var scale = 1.0
    @AppStorage("mineradio.intensity") private var intensity = 0.85
    @AppStorage("mineradio.floatLayer") private var floatLayer = false
    @AppStorage("mineradio.backCover") private var backCover = false
    @AppStorage("mineradio.shelf") private var shelf = false
    @AppStorage("mineradio.enabled") private var enabled = true
    @AppStorage("mineradio.reactive") private var reactive = true
    @AppStorage("mineradio.pauseWhenUnfocused") private var pauseWhenUnfocused = true
    @AppStorage("playerLowPerformance") private var lowPerformance = false
    @AppStorage("lyricOffset") private var lyricOffset = 0.0
    @AppStorage("lyricMode") private var lyricMode = "bilingual"
    @AppStorage("lyricTranslation") private var lyricTranslation = true
    @Environment(\.scenePhase) private var phase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var surfaceVisible: Bool { enabled && live && visible && phase != .background && !store.systemSleeping && !renderFailed }
    private var drawing: Bool { surfaceVisible && (!pauseWhenUnfocused || foreground && windowFocused) && !reduceMotion }
    // Drawing belongs to the foreground surface, not the transport. Pausing freezes
    // the audible clock / analysis, while camera and shader animation keep their cadence.
    private var active: Bool { drawing && store.zone?.status == "playing" }
    private var followID: String { "\(store.sessionID):\(store.zone?.id ?? ""):\(store.zone?.playback_id ?? ""):\(active):\(reactive)" }
    private var config: MineradioRenderer.Configuration {
        let limited = lowPerformance || ProcessInfo.processInfo.thermalState == .serious || ProcessInfo.processInfo.thermalState == .critical
        // Foreground feature pages retain motion behind their translucent surfaces.
        // Only the immersive stage needs the full configured pixel / particle budget.
        return .init(preset: preset, fps: limited ? 24 : immersive ? fps : min(30, fps), particles: limited ? 80 : immersive ? particles : min(118, particles), scale: limited ? min(0.65, scale) : immersive ? scale : min(0.75, scale), intensity: intensity, active: surfaceVisible, immersive: immersive, shelf: shelf, visible: surfaceVisible, floatLayer: floatLayer, backCover: backCover, parameters: parameterSnapshot, stageTop: stageTop, stageBottom: stageBottom, motion: drawing)
    }
    private var playback: MineradioPlayback {
        let offset = lyricOffset / 1000 + store.details.document["offset_ms"].number / 1000
        let reading = store.presentationClock.reading(store.zone)
        return MineradioPlayback(id: store.zone?.track?.id ?? "", session: store.zone?.playback_id ?? "", queueIndex: store.zone?.queue_index ?? -1, title: store.zone?.track?.title ?? "", artist: store.zone?.track?.artist ?? "", position: reading.position, duration: store.zone?.track?.duration ?? 0, playing: reading.playing, lyricOffset: offset)
    }
    private var lyrics: [MineradioLyric] {
        guard immersive else { return [] }
        let mode = lyricTranslation ? lyricMode : "original"
        return store.details.lines.prefix(2000).map { MineradioLyric(time: $0.time, text: mode == "translation" ? $0.translation ?? $0.text : $0.text, translation: mode == "bilingual" ? $0.translation : nil, words: mode == "translation" ? [] : $0.words) }
    }
    private var queue: [MineradioQueueItem] {
        let items = store.zone?.queue ?? [], start = max(0, (store.zone?.queue_index ?? 0) - 20)
        return items.enumerated().dropFirst(start).prefix(50).map { MineradioQueueItem(id: $0.element.id, title: $0.element.title, artist: $0.element.artist, index: $0.offset, cover: queueArtwork.images[$0.offset] ?? "") }
    }
    private var shelfArtworkID: String { "\(store.sessionID):\(store.zone?.id ?? ""):\(drawing && immersive && shelf):" + shelfWindow.map(String.init).joined(separator: ",") + queue.map { "\($0.index):\($0.id):\(store.zone?.queue[$0.index].cover_url ?? "")" }.joined(separator: "|") }
    var body: some View {
        ZStack {
            Color(red: 0.025, green: 0.03, blue: 0.045)
            if !renderFailed {
                MineradioRenderer(configuration: config, cover: imageData, frames: active && reactive ? features.frames : [], playback: playback, lyrics: lyricPayload, queue: queue, selectQueue: { index, id in
                    guard let zone = store.zone, zone.queue.indices.contains(index), zone.queue[index].id == id else { return }
                    Task { await store.transport("play", body: ["queue_index": .number(Double(index))]) }
                }, failed: { renderFailed = true }, visibility: { visible = $0 }, focus: { windowFocused = $0 }, queueWindow: { shelfWindow = $0 })
                    .transaction { $0.animation = nil }
            }
            if renderFailed {
                VStack(spacing: 16) { ArtworkView(track: store.zone?.track, store: store, radius: 16, resolveMissing: true).frame(width: 300, height: 300); Text("动态舞台暂不可用").font(.caption).foregroundStyle(.secondary); Button("重新载入舞台") { renderFailed = false } }.frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            LinearGradient(colors: [.black.opacity(immersive ? 0.08 : 0.36), .clear, .black.opacity(0.18)], startPoint: .top, endPoint: .bottom)
                .allowsHitTesting(false)
                .animation(reduceMotion ? nil : .easeInOut(duration: 0.3), value: immersive)
        }.allowsHitTesting(renderFailed || immersive).accessibilityHidden(!renderFailed)
        .onAppear { foreground = NSApp.isActive }
        .onChange(of: store.details.lines, initial: true) { lyricPayload = lyrics }
        .onChange(of: lyricMode) { lyricPayload = lyrics }
        .onChange(of: lyricTranslation) { lyricPayload = lyrics }
        .onChange(of: immersive) { lyricPayload = lyrics }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in foreground = true }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in foreground = false }
        .task(id: shelfArtworkID) {
            await queueArtwork.load(store, indices: shelfWindow, enabled: drawing && immersive && shelf)
        }
        .task(id: parameters) {
            do { try await Task.sleep(for: .milliseconds(100)) } catch { return }
            parameterSnapshot = parameters
        }
        .task(id: followID) {
            guard active, let id = store.zone?.id, let playback = store.zone?.playback_id else { return }
            await features.follow(store, zoneID: id, playbackID: playback, reactive: reactive)
        }
        .task(id: "\(store.sessionID):\(store.artwork.revision):\(store.artwork.sourceKey(store.zone?.track))") {
            guard let track = store.zone?.track else { imageData = nil; return }
            let image = await store.artwork.image(track, store: store, resolveMissing: true)
            guard !Task.isCancelled else { return }
            guard let image, let bitmap = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { imageData = nil; return }
            let worker = Task.detached(priority: .utility) { () -> String? in
                let rep = NSBitmapImageRep(cgImage: bitmap)
                guard !Task.isCancelled, let jpeg = rep.representation(using: .jpeg, properties: [.compressionFactor: 0.7]), jpeg.count < 512 * 1024 else { return nil }
                return "data:image/jpeg;base64," + jpeg.base64EncodedString()
            }
            let data = await withTaskCancellationHandler { await worker.value } onCancel: { worker.cancel() }
            guard !Task.isCancelled else { return }; imageData = data
        }
    }
}

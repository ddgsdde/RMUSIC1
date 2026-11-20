import Foundation
import simd

public struct NativeStageState {
    public var session = ""
    public var frames: [NativeVisualFrame] = []
    public var lyrics: [NativeLyricLine] = []
    public var lyricOffset = 0.0
    public var stageTop = 0.0
    public var stageBottom = 0.0
    public var preset: Int
    public var fps: Int
    public var density: Int
    public var scale: Double
    public var intensity: Double
    public var motion = true
    public var shouldRenderContinuously: Bool { active && visible && (motion || immersive && playing) }
    public var active: Bool
    public var visible: Bool
    public var immersive: Bool
    public var parameters: String
    public var position: Double
    public var playing: Bool
    public var sampledAt: Double
    public var cover: String?
    public var shelf = false
    public var backCover = false
    public var queueIndex = -1
    public var queue: [NativeQueueItem] = []
    public init(preset: Int, fps: Int, density: Int, scale: Double, intensity: Double, active: Bool, visible: Bool, immersive: Bool, parameters: String, position: Double, playing: Bool, cover: String?) {
        self.preset=preset; self.fps=fps; self.density=density; self.scale=scale; self.intensity=intensity
        self.active=active; self.visible=visible; self.immersive=immersive; self.parameters=parameters
        self.position=position; self.playing=playing; self.cover=cover; self.sampledAt=ProcessInfo.processInfo.systemUptime
    }
}

public struct NativeQueueItem: Sendable, Equatable {
    public let id: String, title: String, artist: String, cover: String
    public let index: Int
    public init(id: String, title: String, artist: String, cover: String, index: Int) {
        self.id = id; self.title = title; self.artist = artist; self.cover = cover; self.index = index
    }
}
struct NativeParameters {
    nonisolated(unsafe) private static let defaults: [String:Any] = {
        guard let root = Bundle.module.url(forResource:"Resources",withExtension:nil), let data = try? Data(contentsOf:root.appendingPathComponent("defaults.json")) else { return [:] }
        return (try? JSONSerialization.jsonObject(with:data) as? [String:Any]) ?? [:]
    }()
    var raw: [String: Any] = [:]
    var lyricRasterKey = "{}"
    init(_ json: String) {
        raw = Self.defaults
        guard json.count <= 16384, let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
        raw.merge(object) { _,new in new }
        let keys = Set(["lyricDisplayMode","lyricCustomLineCount","lyricTranslationMode","lyricFont","lyricWeight","lyricLetterSpacing"])
        let raster = raw.filter { keys.contains($0.key) }
        if let data = try? JSONSerialization.data(withJSONObject:raster,options:[.sortedKeys]), let key = String(data:data,encoding:.utf8) { lyricRasterKey = key }
    }
    func number(_ key: String, _ fallback: Float) -> Float {
        let value = (raw[key] as? NSNumber)?.floatValue ?? fallback
        return value.isFinite ? value : fallback
    }
    func flag(_ key: String, _ fallback: Bool) -> Bool { (raw[key] as? Bool) ?? fallback }
    func color(_ key: String, _ fallback: SIMD3<Float>) -> SIMD3<Float> {
        guard let hex = raw[key] as? String, hex.count == 7, let value = UInt32(hex.dropFirst(),radix:16) else { return fallback }
        return SIMD3(Float((value>>16)&255),Float((value>>8)&255),Float(value&255))/255
    }
}

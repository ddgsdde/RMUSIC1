import Foundation

public struct NativeVisualFrame: Sendable, Equatable {
    public let session: String
    public let position: Double
    public let bands: [Float]
    public let energy: Float
    public init(session: String, position: Double, bands: [Float], energy: Float) {
        self.session = session; self.position = position; self.bands = bands; self.energy = energy
    }
}

/// Consumes the existing playback tap. Never opens an audio stream or performs a second FFT.
struct NativeFeatureTimeline {
    struct Onset { let position: Double; let strength: Float; let low: Float; let body: Float }
    private(set) var onsets: [Onset] = []
    private var frames: [NativeVisualFrame] = []
    private var lastBatch: [NativeVisualFrame] = []
    private(set) var bands = [Float](repeating: 0, count: 8)
    private(set) var measuredBands = [Float](repeating:0,count:8)
    private(set) var energy: Float = 0
    private(set) var beat: Float = 0
    private var lastSample: Double = -1
    private var previousLow: Float = 0
    var samplePosition: Double? { lastSample >= 0 ? lastSample : nil }

    mutating func push(_ batch: [NativeVisualFrame], session: String) {
        guard batch != lastBatch else { return }
        lastBatch = batch
        var merged = Dictionary(uniqueKeysWithValues: frames.map { ($0.position, $0) })
        for frame in batch.prefix(24) where frame.session == session && frame.position.isFinite && frame.position >= 0 && frame.bands.count == 8 && frame.bands.allSatisfy({ $0.isFinite && (0...1).contains($0) }) && frame.energy.isFinite {
            merged[frame.position] = frame
        }
        frames = merged.values.sorted { $0.position < $1.position }.suffix(48)
    }
    mutating func pause() { frames = []; lastBatch = []; lastSample = -1; previousLow = 0; energy = 0 }
    mutating func reset() { self = NativeFeatureTimeline() }
    mutating func step(position: Double, playing: Bool, dt: Float) {
        onsets.removeAll(keepingCapacity:true)
        let frame = playing ? frames.last(where: { $0.position <= position + 0.005 }) : nil
        let fresh = frame.map { position - $0.position <= 0.3 } ?? false
        measuredBands = fresh ? frame!.bands : Array(repeating:0,count:8)
        beat *= exp(-dt * 5)
        // Consume every due feature in source-time order, including short onsets
        // between draws. A delayed batch ages its pulse instead of replaying it now.
        if playing {
            for due in frames where due.position > lastSample && due.position <= position + 0.005 {
                let low = (due.bands[0] + due.bands[1]) * 0.5
                let onset = max(0, low - previousLow)
                if onset > 0.06 && position - due.position <= 0.3 {
                    beat = max(beat, min(1, onset * 3) * exp(-Float(max(0, position - due.position)) * 5))
                    onsets.append(Onset(position:due.position,strength:min(1,onset*3),low:due.bands[0],body:due.bands[1]))
                }
                previousLow = low; lastSample = due.position
            }
            frames.removeAll { $0.position < lastSample }
        }
        for i in bands.indices {
            let target: Float = fresh ? frame!.bands[i] : 0
            bands[i] += (target - bands[i]) * (1 - exp(-dt * (target > bands[i] ? 45 : 12)))
        }
        energy = fresh ? frame!.energy : 0
    }
}

public struct NativeLyricWord: Sendable, Equatable {
    public let time: Double
    public let text: String
    public let duration: Double?
    public init(time: Double, text: String, duration: Double? = nil) { self.time = time; self.text = text; self.duration = duration }
}
public struct NativeLyricLine: Sendable, Equatable {
    public let time: Double
    public let text: String
    public let translation: String?
    public let words: [NativeLyricWord]
    let wordOffsets: [Int]
    public init(time: Double, text: String, translation: String?, words: [NativeLyricWord]) {
        self.time = time; self.text = text; self.translation = translation; self.words = Array(words.prefix(500))
        var offset = 0
        wordOffsets = self.words.map { word in defer { offset += word.text.utf16.count }; return offset }
    }
}

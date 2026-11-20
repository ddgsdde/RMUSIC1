#if RMUSIC_NATIVE_MINERADIO
import SwiftUI
import MineradioNative

/// The native v2 executable never imports or constructs a WebKit stage.
struct MineradioRenderer: View {
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
    @StoredState private var preparedLyrics: [NativeLyricLine] = []
    private var state: NativeStageState {
        var value = NativeStageState(preset: configuration.preset, fps: configuration.fps, density: configuration.particles, scale: configuration.scale, intensity: configuration.intensity, active: configuration.active, visible: configuration.visible, immersive: configuration.immersive, parameters: configuration.parameters, position: playback.position, playing: playback.playing, cover: cover)
        value.motion = configuration.motion
        value.session = playback.session
        value.sampledAt = playback.sampledAt
        value.stageTop = configuration.stageTop; value.stageBottom = configuration.stageBottom
        value.lyricOffset = playback.lyricOffset
        value.frames = frames.map { NativeVisualFrame(session: $0.playback_id, position: $0.position, bands: $0.bands.map { Float($0) / 255 }, energy: Float($0.energy) / 255) }
        value.lyrics = preparedLyrics
        value.shelf = configuration.shelf
        value.backCover = configuration.backCover
        value.queueIndex = playback.queueIndex
        value.queue = queue.prefix(50).map { NativeQueueItem(id:$0.id,title:$0.title,artist:$0.artist,cover:$0.cover,index:$0.index) }
        return value
    }
    var body: some View {
        NativeStageView(state: state, failed: { _ in failed() }, visibility: visibility, focus: focus, selectQueue: selectQueue, queueWindow: queueWindow)
            .onChange(of: lyrics, initial: true) {
                preparedLyrics = lyrics.map { NativeLyricLine(time: $0.time, text: $0.text, translation: $0.translation, words: $0.words.map { NativeLyricWord(time: $0.time, text: $0.text, duration: $0.duration) }) }
            }
    }
}
#endif

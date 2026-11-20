import Foundation
import MediaPlayer

/// macOS media commands belong to this app's active local output. No device I/O runs here.
@MainActor
final class SystemPlayback {
    private weak var store: MusicStore?
    private var targets: [(MPRemoteCommand, Any)] = []
    private var owned = false
    private let activity = PlaybackActivity()
    init(store: MusicStore) {
        self.store = store
        let commands = MPRemoteCommandCenter.shared()
        for (command, action) in [(commands.playCommand, "play"), (commands.pauseCommand, "pause"), (commands.togglePlayPauseCommand, "toggle"), (commands.stopCommand, "stop"), (commands.nextTrackCommand, "next"), (commands.previousTrackCommand, "previous")] {
            let target = command.addTarget { [weak self] _ in
                Task { @MainActor [weak self] in await self?.perform(action) }
                return .success
            }
            targets.append((command, target)); command.isEnabled = false
        }
        let target = commands.changePlaybackPositionCommand.addTarget { [weak self] event in
            guard let position = (event as? MPChangePlaybackPositionCommandEvent)?.positionTime else { return .commandFailed }
            Task { @MainActor [weak self] in await self?.perform("seek", position: position) }
            return .success
        }
        targets.append((commands.changePlaybackPositionCommand, target)); commands.changePlaybackPositionCommand.isEnabled = false
    }
    isolated deinit {
        for (command, target) in targets { command.removeTarget(target) }
        if owned { MPNowPlayingInfoCenter.default().nowPlayingInfo = nil; MPNowPlayingInfoCenter.default().playbackState = .stopped }
    }
    func update() {
        guard let store else { return }
        let zone = store.systemSleeping ? nil : store.localZone
        let track = zone?.track
        let status = zone?.status ?? "stopped"
        activity.update(playing: !store.systemSleeping && PlaybackActivity.hasActiveOutput(
            zones: store.zones, enabledEndpoints: Set(store.outputs.enabled.map(\.id))))
        guard let track, ["playing", "paused"].contains(status) else {
            if owned { MPNowPlayingInfoCenter.default().nowPlayingInfo = nil; MPNowPlayingInfoCenter.default().playbackState = .stopped; owned = false }
            for (command, _) in targets { command.isEnabled = false }; return
        }
        owned = true
        let position = zone?.position ?? 0
        MPNowPlayingInfoCenter.default().nowPlayingInfo = [MPMediaItemPropertyTitle: track.title, MPMediaItemPropertyArtist: track.artist, MPMediaItemPropertyAlbumTitle: track.album, MPMediaItemPropertyPlaybackDuration: track.duration, MPNowPlayingInfoPropertyElapsedPlaybackTime: position, MPNowPlayingInfoPropertyPlaybackRate: status == "playing" ? 1.0 : 0.0]
        MPNowPlayingInfoCenter.default().playbackState = status == "playing" ? .playing : .paused
        for (command, _) in targets { command.isEnabled = true }
    }
    private func perform(_ action: String, position: Double = 0) async {
        guard let store else { return }
        if !store.systemSleeping, let zone = store.localZone {
            await store.transport(action == "toggle" ? zone.status == "playing" ? "pause" : "play" : action, body: action == "seek" ? ["position": .number(position)] : [:],target:zone)
        }
        update()
    }
}

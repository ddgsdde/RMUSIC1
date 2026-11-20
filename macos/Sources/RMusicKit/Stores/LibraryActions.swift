import Foundation

extension MusicStore {
    func openRelated(_ track: Track, kind: String = "album") {
        Task { await perform {
            let refs = try await catalogReferences(track, album: kind == "album", artistIndex: 0)
            guard refs.count == 1 else { throw RuntimeFailure(message: "请直接点击对应歌手的名字。") }
            await openCatalogReference(refs[0], track: track, album: kind == "album")
        } }
    }
    func enqueue(_ track: Track) async {
        if outputs.roaming {travel.remember([track])}
        await perform {
            guard let zone else { throw RuntimeFailure(message: "先启用并选择声音设备。") }
            try await requireUngrouped(zone.id)
            _ = try await api("/zones/\(zone.id.pathComponentEncoded)/queue?view=state", method: "POST", body: .object(["track_ids": .array([.string(track.id)])]))
            try await refreshZones()
        }
    }
    func playNext(_ track: Track) async {
        if outputs.roaming {travel.remember([track])}
        await perform {
            guard let zone else { throw RuntimeFailure(message: "先启用并选择声音设备。") }
            try await requireUngrouped(zone.id)
            _ = try await api("/zones/\(zone.id.pathComponentEncoded)/queue?view=state", method: "POST", body: .object(["track_ids": .array([.string(track.id)]), "play_next": .bool(true)]))
            try await refreshZones()
        }
    }
    func reorderQueue(_ original: Zone, from: Int, to: Int) async {
        await perform {
            try await requireUngrouped(original.id)
            let order = movedTrackIDs(Array(original.queue.indices).map(String.init), from: from, to: to).compactMap(Int.init)
            _ = try await api("/zones/\(original.id.pathComponentEncoded)/queue/reorder?view=state", method: "POST", body: .object([
                "expected_queue": .array(original.queue.map { .string($0.id) }),
                "expected_playback_id": original.playback_id.map(JSONValue.string) ?? .null,
                "order": .array(order.map { .number(Double($0)) }),
            ]))
            try await refreshZones()
        }
    }
    func playList(_ tracks: [Track], starting id: String? = nil) async {
        await perform {
            guard let zone, !tracks.isEmpty else { throw RuntimeFailure(message: "先启用并选择声音设备。") }
            try await requireUngrouped(zone.id)
            if outputs.roaming {travel.remember(tracks)}
            let index = tracks.firstIndex { $0.id == id } ?? 0
            _ = try await api("/zones/\(zone.id.pathComponentEncoded)/queue?view=state", method: "PUT", body: .object(["track_ids": .array(tracks.map { .string($0.id) }),"queue_index": .number(Double(index)),"shuffle": .bool(false)]))
            _ = try await api("/zones/\(zone.id.pathComponentEncoded)/play", method: "POST", body: .object(["queue_index": .number(Double(index))]))
            try await refreshZones()
        }
    }
    func startRadio(_ track: Track) async {
        await perform {
            guard let zone else { throw RuntimeFailure(message: "请先选择播放设备。") }
            _ = try await api("/zones/\(zone.id.pathComponentEncoded)", method: "PATCH", body: .object([
                "autoplay": .object(["enabled": .bool(true), "source": .string("local"), "fallback_local": .bool(true)])
            ]))
            await playList([track])
        }
    }
    func openAlbumID(_ id: String, name: String) async {
        artistID = nil; artistName = nil; libraryQueryOverride = ""; albumID = id; albumName = name; page = 0; showLibrary = true
        await perform { try await loadLibrary() }
    }
}

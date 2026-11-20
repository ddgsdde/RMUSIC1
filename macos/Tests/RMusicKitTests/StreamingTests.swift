import Foundation
import Testing
@testable import RMusicKit

@Test func remoteTransportAndPrecisionConversionAreDifferentSignalStages() throws {
    let value: JSONValue = .object([
        "id": .string("z"), "name": .string("QA"), "endpoint_id": .string("mac"), "status": .string("playing"), "position": .number(1), "volume": .number(1), "muted": .bool(false), "queue_index": .number(0), "queue": .array([]), "playback_id": .string("p"),
        "track": .object(["id": .string("song"), "title": .string("DSD"), "artist": .string("QA"), "album": .string("QA"), "duration": .number(10), "format": .string("dsf"), "sample_rate": .number(2822400), "bit_depth": .number(1)])])
    let zone = try value.decode(Zone.self)
    let endpoint: JSONValue = .object(["playback": .object(["track_id": .string("song"), "core_playback_id": .string("p"), "signal_path": .object(["sample_rate": .number(44100), "bit_depth": .number(24), "channels": .number(2), "processing": .array([.string("Remote FLAC transport: 44100 Hz / 16 bit / 2 ch; buffer up to 15 s / 32 MiB"), .string("Core stream conversion: 2822400 Hz / 1 bit / 2 ch → 44100 Hz / 16 bit / 2 ch")])])])])
    let view = SignalPresentation(zone: zone, endpoint: endpoint, info: .null)
    #expect(view.stages.first { $0.name == "传输" }?.status == "FLAC 无损编码")
    #expect(view.stages.first { $0.name == "Core 格式转换" }?.status == "DSD → PCM")
    #expect(view.stages.first { $0.name == "传输" }?.detail.contains("HTTPS FLAC") == true)
    let stale = SignalPresentation(zone: zone, endpoint: .object([:]), info: .null)
    #expect(!stale.hasReport && !stale.stages.contains { $0.name == "Core 格式转换" })
}

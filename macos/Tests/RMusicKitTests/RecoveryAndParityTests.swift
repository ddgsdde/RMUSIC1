import Foundation
import Testing

@testable import RMusicKit

private func fixtureJSON(_ string: String) throws -> JSONValue {
  try JSONDecoder().decode(JSONValue.self, from: Data(string.utf8))
}

@Test func recoveryNeverUsesAnotherPlaybackInstancesErrorOrBufferState() throws {
  let zone = try fixtureJSON(
    #"{"id":"z","name":"QA","endpoint_id":"e","status":"playing","track":{"id":"t","title":"Song","artist":"QA","album":"QA","duration":12,"format":"wav","sample_rate":48000,"bit_depth":16},"position":2,"volume":1,"muted":false,"playback_id":"current","queue_index":0,"queue":[]}"#
  ).decode(Zone.self)
  let old = try fixtureJSON(
    #"{"error":{"zone_id":"z","track_id":"t","core_playback_id":"old","recoverable":true,"device_failed":true,"error":"stale"}}"#
  )
  #expect(PlaybackRecovery.resolve(zone: zone, report: old) == nil)
  let current = try fixtureJSON(
    #"{"error":{"zone_id":"z","track_id":"t","core_playback_id":"current","recoverable":true,"source_failed":true,"error":"source stopped"}}"#
  )
  #expect(PlaybackRecovery.resolve(zone: zone, report: current)?.kind == .source)
  #expect(PlaybackRecovery.resolve(zone: zone, report: current)?.recoverable == true)
  let buffering = try fixtureJSON(
    #"{"playback":{"track_id":"t","core_playback_id":"current","signal_path":{"processing":["Playback buffering: source data pending"]}}}"#
  )
  #expect(PlaybackRecovery.resolve(zone: zone, report: buffering)?.kind == .buffering)
}


@Test func recoveryDistinguishesSourceBufferingFromHistoricalOutputGap() throws {
  let zone = try fixtureJSON(#"{"id":"z","name":"QA","endpoint_id":"e","status":"playing","track":{"id":"t","title":"Song","artist":"QA","album":"QA","duration":12,"format":"wav","sample_rate":48000,"bit_depth":16},"position":2,"volume":1,"muted":false,"playback_id":"current","queue_index":0,"queue":[]}"#).decode(Zone.self)
  let historical = try fixtureJSON(#"{"playback":{"track_id":"t","core_playback_id":"current","signal_path":{"processing":["CoreAudio underrun; continuity lost"]}}}"#)
  let buffering = try fixtureJSON(#"{"playback":{"track_id":"t","core_playback_id":"current","signal_path":{"processing":["CoreAudio underrun; continuity lost","Playback buffering: source data pending"]}}}"#)
  #expect(PlaybackRecovery.resolve(zone: zone, report: historical)?.kind == .continuity)
  #expect(PlaybackRecovery.resolve(zone: zone, report: buffering)?.kind == .buffering)
  #expect(PlaybackRecovery.resolve(zone: zone, report: buffering)?.message.contains("音源") == true)
  // Unknown cause must not be presented as a device failure or an instruction to enlarge buffers.
  #expect(PlaybackRecovery.resolve(zone: zone, report: historical)?.message.contains("调整") == false)
}

@Test func dspDraftKeepsConvolutionAndSampleRateIndependentBands() throws {
  var draft = PlaybackSettingsDraft()
  draft.original = [
    "convolution": .object(["response_ids": .array([.string("original-response")])])
  ]
  draft.bands = [.init(frequency: 1000, gain: -3, q: 1)]
  let value = try draft.payload(rate: 48000)
  #expect(value["dsp"]["convolution"]["response_ids"].array.first?.string == "original-response")
  draft.bands[0].frequency = 24000
  #expect(try draft.payload(rate: 48000)["dsp"]["bands"].array.first?["frequency"].integer == 24000)
  draft.bands[0].frequency = 1000
  draft.preamp = .nan
  #expect(throws: RuntimeFailure.self) { try draft.payload(rate: 48000) }
}

@Test func pluginPatchCannotWriteBackRedactedSecretsAndKeepsNestedValues() throws {
  let schema = try fixtureJSON(
    #"{"type":"object","properties":{"key":{"type":"string","secret":true},"nested":{"type":"object","properties":{"enabled":{"type":"boolean"},"token":{"type":"string","secret":true}}}}}"#
  )
  let base = try fixtureJSON(
    #"{"key":{"configured":true},"nested":{"enabled":true,"token":{"configured":true}}}"#)
  let changed = SchemaDraft.merged(base, try fixtureJSON(#"{"nested":{"enabled":false}}"#))
  let patch = SchemaDraft.patch(base: base, value: changed, schema: schema)
  #expect(patch["key"].object.isEmpty)
  #expect(patch["nested"]["token"].object.isEmpty)
  #expect(patch["nested"]["enabled"].boolean == false)
  #expect(patch.object.count == 1)
  let required = try fixtureJSON(
    #"{"type":"object","required":["enabled","count"],"properties":{"enabled":{"type":"boolean"},"count":{"type":"integer"}}}"#
  )
  let initial = SchemaDraft.prepared(.object([:]), schema: required)
  #expect(initial.object.keys.contains("enabled") && !initial["enabled"].boolean)
  #expect(initial.object.keys.contains("count") && initial["count"].number == 0)
}

@Test func lyricVariantsPronunciationAndOffsetsStayAlignedAndOversizedDataIsRejected() throws {
  let document = try fixtureJSON(
    #"{"text":"[00:01.00]one\n[00:02.00]two","translations":[{"id":"zh","text":"[00:01.04]一\n[00:02.02]二"},{"id":"other","text":"[00:01.00]A\n[00:02.00]B"}],"romanization":"[00:01.00]ichi\n[00:02.00]ni"}"#
  )
  let lines = try renderLyricDocument(document, language: "zh")
  #expect(lines.map(\.translation) == ["一", "二"])
  #expect(lines.map(\.romanization) == ["ichi", "ni"])
  #expect(try renderLyricDocument(document, language: "other").first?.translation == "A")
  let oversized = JSONValue.object(["text": .string(String(repeating: "a", count: 1_048_577))])
  #expect(throws: RuntimeFailure.self) { try renderLyricDocument(oversized, language: "") }
  let original = (0..<2000).map { "[\($0):00.00]line-\($0)" }.joined(separator: "\n")
  let translated = (0..<2000).map { "[\($0):00.01]translation-\($0)" }.joined(separator: "\n")
  #expect(parseLyrics(original, translation: translated).last?.translation == "translation-1999")
}

@MainActor private final class ImportPreviewRuntime: RuntimeTransport {
  var requests = 0
  var unstable = false
  func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
    requests += 1
    let offset = body["json"]["offset"].integer
    let count = body["json"]["limit"].integer
    let rows = (offset..<min(offset + count, 205)).map { index in
      JSONValue.object([
        "status": .string(index % 2 == 0 ? "matched" : "review"),
        "matched": index % 2 == 0 ? .object(["id": .string("core-\(index)")]) : .null,
        "candidates": .array([.object(["id": .string("ambiguous-\(index)")])]),
      ])
    }
    return .object(["items": .array(rows), "total": .number(unstable && offset > 0 ? 204 : 205)])
  }
  func shutdown() async {}
}
@Test @MainActor func playlistImportChecksEveryPageSelectsOnlyExactMatchesAndReusesRequestOnRetry()
  async throws
{
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
    "rmusic-import-test-\(UUID())")
  try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
  let file = directory.appendingPathComponent("references.json")
  try Data("[]".utf8).write(to: file)
  let runtime = ImportPreviewRuntime()
  let store = MusicStore(runtime: runtime)
  let draft = PlaylistImportStore()
  await draft.scan(file, store: store)
  #expect(
    draft.ready && draft.total == 205 && draft.selected.count == 103 && draft.rows.count == 50)
  #expect(runtime.requests == 3)
  #expect(draft.selected[1] == nil)
  draft.selected[1] = "manually-confirmed"
  let first = try draft.payload(name: "Imported")
  let retry = try draft.payload(name: "Imported")
  #expect(first["request_id"].text == retry["request_id"].text)
  #expect(first["track_ids"].array[1].text == "manually-confirmed")
  #expect(try draft.payload(name: "Different")["request_id"].text != first["request_id"].text)
  await draft.page(100, store: store)
  #expect(draft.rows.count == 50 && draft.selected.count == 104)
  runtime.unstable = true
  await draft.scan(file, store: store)
  #expect(!draft.ready && draft.selected.isEmpty && draft.error != nil)
  draft.reset()
  #expect(draft.content.isEmpty && draft.rows.isEmpty)
}

@Test func groupRequestsRequireRealCapabilitiesAndRetryKeepsTheSameIdentity() throws {
  let endpoint = try fixtureJSON(
    #"{"online":true,"capabilities":{"scheduled_start_v1":false,"scheduled_queue_v1":false,"group_output_v2":false,"measured_listens_v2":true}}"#
  )
  #expect(!GroupRequest.eligible(endpoint))
  let pending = GroupRequest(path: "/sync/starts/qa/control", fields: ["action": .string("pause")])
  let retry = pending
  #expect(pending.body["request_id"].text == retry.body["request_id"].text)
  #expect(UUID(uuidString: pending.body["request_id"].text) != nil)
}

@Test @MainActor func rememberedOutputResolvesAfterStartupAndAcrossZoneChanges() throws {
  let core = UUID().uuidString, account = UUID().uuidString
  let preference = "playbackOutput.\(core).\(account)"
  defer { UserDefaults.standard.removeObject(forKey:preference) }
  func store(for user: String) -> MusicStore {
    let store = MusicStore(runtime:ImportPreviewRuntime())
    store.profile.core_id = core
    store.user = .object(["id":.string(user)])
    return store
  }
  func output(_ endpoint: String, zone: String) throws -> Zone {
    try fixtureJSON("""
      {"id":"\(zone)","name":"QA","endpoint_id":"\(endpoint)","status":"stopped",
       "position":0,"volume":1,"muted":false,"track":null,"queue_index":0,"queue":[]}
      """).decode(Zone.self)
  }
  let local = try output("local",zone:"local-zone")
  let remote = try output("desk-dac",zone:"previous-zone")
  let prior = store(for:account)
  prior.zones = [local,remote]
  prior.selectPlaybackZone(remote.id)
  let reopened = store(for:account)
  reopened.loadPlaybackOutputPreference()
  reopened.zones = [local]
  reopened.restorePlaybackSelection(in:reopened.zones,fallback:local)
  #expect(reopened.selectedZoneID == local.id)
  let recreated = try output(remote.endpoint_id,zone:"recreated-zone")
  reopened.zones = [local,recreated]
  reopened.restorePlaybackSelection(in:reopened.zones,fallback:local)
  #expect(reopened.selectedZoneID == recreated.id)
  let other = store(for:"other-account")
  other.loadPlaybackOutputPreference()
  other.zones = [local,recreated]
  other.restorePlaybackSelection(in:other.zones,fallback:local)
  #expect(other.selectedZoneID == local.id)
}

@Test func equalizerPreviewUsesTheSameCutSlopesAndBandGain() {
  typealias Band = PlaybackSettingsDraft.Band
  let peak = Band(frequency:1000,gain:6,q:1)
  let response = EqualizerResponse.sections(peak,rate:48000)
  #expect(abs(EqualizerResponse.at(1000,rate:48000,sections:response)-6) < 0.0001)
  let cut = Band(kind:.highPass,frequency:1000,q:1/sqrt(2),slope:48)
  let steep = EqualizerResponse.sections(cut,rate:48000)
  #expect(steep.count == 4)
  #expect(EqualizerResponse.at(100,rate:48000,sections:steep) < -78)
}

@Test func dspDraftKeepsIndependentEqualizerModulesAndPromotesTheNextStage() throws {
  var draft = PlaybackSettingsDraft()
  #expect(draft.modules(portable:false) == ["eq", "loudness", "fades"])
  #expect(draft.modules(portable:true) == ["loudness", "fades"])
  draft.installedModules = ["eq","gain"]
  draft.bands = [.init(frequency:200,gain:3)]
  draft.extraEQ = [.init(bands:[.init(frequency:1500,gain:-2)],bypass:true)]
  let chain = try draft.payload(rate:48000)["dsp"]
  #expect(chain["bands"].array.first?["frequency"].integer == 200)
  #expect(chain["extra_eq"].array.first?["bypass"].boolean == true)
  draft.allEQ = Array(draft.allEQ.dropFirst())
  let promoted = try draft.payload(rate:48000)["dsp"]
  #expect(promoted["bands"].array.first?["frequency"].integer == 1500)
  #expect(promoted["eq_bypass"].boolean)
  #expect(promoted["extra_eq"] == .null)
  #expect(promoted["modules"].array.compactMap(\.string) == ["eq","gain"])
}

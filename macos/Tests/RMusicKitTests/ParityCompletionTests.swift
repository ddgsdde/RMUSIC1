import Foundation
import Testing

@testable import RMusicKit

@Test @MainActor func groupJournalSurvivesReopeningSeparatesOwnersAndNeverReplacesPendingUUID()
  throws
{
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
    "rmusic-group-journal-" + UUID().uuidString)
  defer { try? FileManager.default.removeItem(at: directory) }
  let first = GroupRequestJournal(directory: directory)
  let second = GroupRequestJournal(directory: directory)
  let request = GroupRequest(
    path: "/sync/starts", fields: ["zone_ids": .array([.string("a"), .string("b")])])
  try first.load(owner: "core:user")
  try second.load(owner: "core:user")
  try first.prepare(request)
  #expect(throws: (any Error).self) {
    try second.prepare(GroupRequest(path: "/sync/starts", fields: [:]))
  }
  try second.load(owner: "core:user")
  #expect(second.pending?.requestID == request.requestID)
  try second.clear(confirmed: UUID().uuidString)
  #expect(second.pending?.requestID == request.requestID)
  let other = GroupRequestJournal(directory: directory)
  try other.load(owner: "core:other")
  #expect(other.pending == nil)
  try other.prepare(GroupRequest(path: "/sync/starts", fields: [:]))
  try first.clear(confirmed: request.requestID)
  try second.load(owner: "core:user")
  #expect(second.pending == nil)
  try other.load(owner: "core:other")
  #expect(other.pending != nil)
  let file = directory.appendingPathComponent("PendingActions/groups.json")
  #expect(
    (try FileManager.default.attributesOfItem(atPath: file.path)[.posixPermissions] as? Int)
      == 0o600)
  try Data("invalid journal".utf8).write(to: file)
  #expect(throws: (any Error).self) { try second.load(owner: "core:user") }
  #expect(throws: (any Error).self) { try second.prepare(request) }
}

@Test func nativeGroupSeekAndJumpRespectDifferentCoreContracts() throws {
  let seek = JSONValue.object(GroupPlan.seek(from: 2, to: 2, position: 3))
  #expect(seek["action"].text == "seek" && seek.object["from_queue_index"] == nil)
  let jump = JSONValue.object(GroupPlan.seek(from: 2, to: 3, position: 0))
  #expect(jump["action"].text == "jump" && jump["from_queue_index"].integer == 2)
  var plan = GroupPlan()
  let track = Track(
    id: "exact", title: "QA", artist: "QA", album: "QA", duration: 1, format: "flac",
    sample_rate: 44100, bit_depth: 24)
  plan.queue = [track, track]
  let fields = try plan.fields(rooms: ["a", "b"], current: nil)
  #expect(fields["queue"]?.array.count == 2 && fields["pcm_rate_adjustment"] == nil)
  plan.automaticRate = true
  #expect(!plan.valid(current: nil))
  #expect(!GroupRequest(path: "/auth/tokens", fields: [:]).valid)
}

@Test func complexSchemaProtectsSecretsAndValidatesRealCoreLimits() throws {
  let string: JSONValue = .object(["type": .string("string"), "maxLength": .number(8192)])
  let secret: JSONValue = .object([
    "type": .string("object"), "secret": .bool(true), "properties": .object(["key": string]),
    "required": .array([.string("key")]),
  ])
  let schema: JSONValue = .object([
    "type": .string("object"),
    "properties": .object([
      "credential": secret,
      "items": .object(["type": .string("array"), "items": string, "maxItems": .number(1000)]),
    ]),
  ])
  let base: JSONValue = .object([
    "credential": .object(["configured": .bool(true)]), "items": .array([]),
  ])
  let prepared = SchemaDraft.prepared(base, schema: schema)
  #expect(SchemaValidation.protected(prepared["credential"]))
  let changed = SchemaDraft.merged(
    base, .object(["credential": .object(["key": .string("synthetic")])]), schema: schema)
  #expect(changed["credential"].object["configured"] == nil)
  let patch = try SchemaValidation.configurationPatch(base: base, value: changed, schema: schema)
  #expect(patch["credential"]["key"].text == "synthetic")
  #expect(
    try SchemaValidation.configurationPatch(base: base, value: base, schema: schema).object.isEmpty)
  try SchemaValidation.check(
    .array(Array(repeating: .string("ok"), count: 1000)), schema: schema["properties"]["items"])
  #expect(throws: (any Error).self) {
    try SchemaValidation.check(
      .array(Array(repeating: .string("ok"), count: 1001)), schema: schema["properties"]["items"])
  }
  #expect(throws: (any Error).self) {
    try SchemaValidation.check(.string(String(repeating: "中", count: 3000)), schema: string)
  }
  let listSchema: JSONValue = .object([
    "type": .string("object"),
    "properties": .object([
      "items": .object(["type": .string("array"), "items": secret, "maxItems": .number(10)])
    ]),
  ])
  let protectedList: JSONValue = .object(["items": .array([.object(["configured": .bool(true)])])])
  #expect(
    try SchemaValidation.configurationPatch(
      base: protectedList, value: protectedList, schema: listSchema
    ).object.isEmpty)
  #expect(throws: (any Error).self) {
    try SchemaValidation.configurationPatch(
      base: .object(["items": .array([])]), value: protectedList, schema: listSchema)
  }
  let enumSchema: JSONValue = .object([
    "type": .string("integer"), "enum": .array([.number(7), .number(8)]),
  ])
  #expect(SchemaDraft.initial(enumSchema).integer == 7)
}

@Test func enhancedLyricsUseOnlyDeclaredWordTimesAndKeepTranslationOffsets() throws {
  let document: JSONValue = .object([
    "text": .string(
      "[offset:100]\n[00:01.00]<00:01.00>Light <00:01.50>and <00:02.00>Motion\n[00:04.00]Next"),
    "translation": .string("[00:01.10]光与运动"),
  ])
  let lines = try renderLyricDocument(document, language: "")
  #expect(lines[0].text == "Light and Motion")
  #expect(lines[0].words.map(\.time) == [1.1, 1.6, 2.1])
  #expect(lines[0].translation == "光与运动")
  #expect(lines[1].words.isEmpty)
  #expect(parseLyrics("[00:01.00]<00:02.00>later<00:00.50>backwards")[0].words.isEmpty)
  #expect(parseLyrics("[00:01.00][00:02.00]<00:01.00>Repeated").allSatisfy { $0.words.isEmpty })
}

@Test func playerLayoutIsBoundedCompleteAndRejectsCorruptOrDuplicateModules() {
  var layout = PlayerLayout.standard
  layout.move(0, by: 1)
  layout.items[0].column = 1
  #expect(PlayerLayout.read(layout.encoded) == layout)
  var duplicate = layout
  duplicate.items[0] = duplicate.items[1]
  #expect(PlayerLayout.read(duplicate.encoded) == .standard)
  var oversized = layout
  oversized.items[0].height = 9000
  #expect(PlayerLayout.read(oversized.encoded) == .standard)
  #expect(PlayerLayout.read(String(repeating: "x", count: 8193)) == .standard)
}

@Test @MainActor func backgroundPlaybackActivityNeverAccumulatesAcrossRepeatedUpdatesAndTearsDown()
{
  var started = 0
  var ended = 0
  var activity: PlaybackActivity? = PlaybackActivity(
    begin: {
      started += 1
      return NSObject()
    }, end: { _ in ended += 1 })
  for _ in 0..<1000 { activity?.update(playing: true) }
  #expect(started == 1 && ended == 0)
  activity?.update(playing: false)
  activity?.update(playing: false)
  #expect(started == 1 && ended == 1)
  for _ in 0..<100 {
    activity?.update(playing: true)
    activity?.update(playing: false)
  }
  #expect(started == 101 && ended == 101)
  activity?.update(playing: true)
  activity = nil
  #expect(started == 102 && ended == 102)
}

@Test @MainActor func backgroundPlaybackActivityFollowsAllEnabledLocalZonesInsteadOfSelectedIdleZone()
{
  let track = Track(id: "t", title: "QA", artist: "QA", album: "QA", duration: 10,
    format: "flac", sample_rate: 48000, bit_depth: 24)
  func zone(_ id: String, _ endpoint: String, _ status: String, _ hasTrack: Bool = true) -> Zone {
    Zone(id: id, name: id, endpoint_id: endpoint, status: status, track: hasTrack ? track : nil,
      position: 0, volume: 1, muted: false, playback_id: nil, queue_index: 0, queue: [])
  }
  let idle = zone("selected-idle", "local", "stopped")
  let playing = zone("remote-controlled", "local", "playing")
  #expect(PlaybackActivity.hasActiveOutput(zones: [idle, playing], enabledEndpoints: ["local"]))
  #expect(!PlaybackActivity.hasActiveOutput(zones: [idle, playing], enabledEndpoints: ["another"]))
  #expect(!PlaybackActivity.hasActiveOutput(zones: [idle, zone("paused", "local", "paused")], enabledEndpoints: ["local"]))
  #expect(!PlaybackActivity.hasActiveOutput(zones: [zone("empty", "local", "playing", false)], enabledEndpoints: ["local"]))
}

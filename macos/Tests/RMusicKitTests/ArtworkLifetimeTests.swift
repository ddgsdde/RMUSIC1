import AppKit
import Foundation
import Testing

@testable import RMusicKit

@Test @MainActor func currentTrackWithoutCoverResolvesItsExactIdentityAndSharesVisibleRequests() async throws {
  let runtime = SlowArtworkRuntime()
  let currentStore = MusicStore(runtime: runtime)
  let first = Track(id: "missing-a", title: "Song", artist: "Artist", album: "", duration: 1, format: "ogg", sample_rate: 44100, bit_depth: 0)
  let second = Track(id: "missing-b", title: "Song", artist: "Artist", album: "", duration: 1, format: "ogg", sample_rate: 44100, bit_depth: 0)
  #expect(await currentStore.artwork.image(first, store: currentStore) == nil)
  #expect(runtime.started == 0)
  let a = Task { await currentStore.artwork.image(first, store: currentStore, resolveMissing: true) }
  let shared = Task { await currentStore.artwork.image(first, store: currentStore, resolveMissing: true) }
  let b = Task { [second] in await currentStore.artwork.image(second, store: currentStore, resolveMissing: true) }
  let deadline = ContinuousClock.now + .seconds(3)
  while runtime.started < 2 && ContinuousClock.now < deadline { await Task.yield() }
  #expect(runtime.started == 2 && currentStore.artwork.pendingCount == 2)
  #expect(Set(runtime.bodies.compactMap { $0["track_id"].string }) == ["missing-a", "missing-b"])
  a.cancel(); shared.cancel(); b.cancel()
  _ = await (a.value, shared.value, b.value)
  await currentStore.shutdown()
  #expect(runtime.active == 0)
}

@MainActor private final class SlowArtworkRuntime: RuntimeTransport {
  var bodies: [JSONValue] = []
  var started = 0
  var cancelled = 0
  var active = 0
  func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
    guard operation == "artwork" else { return .null }
    bodies.append(body)
    started += 1
    active += 1
    defer { active -= 1 }
    do { try await Task.sleep(for: .seconds(60)) } catch {
      cancelled += 1
      throw error
    }
    return .null
  }
  func shutdown() async {}
}

@Test @MainActor func abandoningAnArtworkViewCancelsItsUnsharedUpstreamRequest() async throws {
  let runtime = SlowArtworkRuntime()
  let store = MusicStore(runtime: runtime)
  var track = Track(
    id: "artwork-a", title: "QA", artist: "QA", album: "QA", duration: 1, format: "wav",
    sample_rate: 48000, bit_depth: 16)
  track.cover_url = "/api/v1/library/tracks/artwork-a/cover"
  let subscriber = Task { await store.artwork.image(track, store: store) }
  for _ in 0..<100 where runtime.started == 0 { try await Task.sleep(for: .milliseconds(2)) }
  #expect(runtime.started == 1)
  subscriber.cancel()
  let deadline = ContinuousClock.now + .seconds(3)
  while runtime.active > 0 && ContinuousClock.now < deadline { await Task.yield() }
  #expect(runtime.cancelled == 1)
  #expect(runtime.active == 0)
  store.artwork.reset()
  _ = await subscriber.value
  #expect(store.artwork.pendingCount == 0)
}

@Test @MainActor func roamingArtworkRejoinsTheRequestAfterAViewRemount() async {
  let runtime = SlowArtworkRuntime()
  let store = MusicStore(runtime: runtime)
  store.outputs.roaming = true
  let track = Track(id: "song", title: "QA", artist: "", album: "", duration: 1, format: "flac",
                    sample_rate: 44100, bit_depth: 16, cover_url: "https://images.example/cover.jpg")
  let first = Task { await store.artwork.image(track, store: store) }
  let deadline = ContinuousClock.now + .seconds(3)
  while runtime.started == 0 && ContinuousClock.now < deadline { await Task.yield() }
  first.cancel()
  _ = await first.value
  #expect(runtime.started == 1 && runtime.cancelled == 0 && store.artwork.pendingCount == 1)
  let second = Task { await store.artwork.image(track, store: store) }
  try? await Task.sleep(for: .milliseconds(400))
  #expect(runtime.started == 1 && runtime.cancelled == 0)
  second.cancel()
  _ = await second.value
  store.artwork.reset()
}

@Test @MainActor func sharedArtworkSurvivesOneSubscriberAndRapidTrackChangesReleaseEveryLease()
  async throws
{
  let runtime = SlowArtworkRuntime()
  let store = MusicStore(runtime: runtime)
  var track = Track(
    id: "shared", title: "QA", artist: "QA", album: "QA", duration: 1, format: "wav",
    sample_rate: 48000, bit_depth: 16)
  track.cover_url = "/synthetic-cover"
  var firstFinished = false
  let first = Task {
    let image = await store.artwork.image(track, store: store)
    firstFinished = true
    return image
  }
  let second = Task { await store.artwork.image(track, store: store) }
  try await Task.sleep(for: .milliseconds(20))
  first.cancel()
  let deadline = ContinuousClock.now + .seconds(3)
  while !firstFinished && ContinuousClock.now < deadline { await Task.yield() }
  #expect(runtime.started == 1 && runtime.cancelled == 0 && firstFinished)
  second.cancel()
  _ = await first.value
  _ = await second.value
  let upstreamDeadline = ContinuousClock.now + .seconds(3)
  while runtime.active > 0 && ContinuousClock.now < upstreamDeadline { await Task.yield() }
  #expect(runtime.cancelled == 1 && store.artwork.pendingCount == 0)
  var tasks: [Task<Void, Never>] = []
  for index in 0..<240 {
    var next = Track(
      id: "rapid-\(index)", title: "QA", artist: "QA", album: "QA", duration: 1, format: "wav",
      sample_rate: 48000, bit_depth: 16)
    next.cover_url = "/synthetic-cover"
    tasks.append(Task { [next] in _ = await store.artwork.image(next, store: store) })
  }
  try await Task.sleep(for: .milliseconds(30))
  #expect(store.artwork.pendingCount <= 128 && runtime.active <= 6)
  tasks.forEach { $0.cancel() }
  for task in tasks { await task.value }
  for _ in 0..<100 where runtime.active > 0 { try await Task.sleep(for: .milliseconds(2)) }
  #expect(store.artwork.pendingCount == 0 && runtime.active == 0)
}

@Test @MainActor func updatedCoverForSameCoreTrackStartsANewRequestAndReleasesBothConsumers()
  async throws
{
  let runtime = SlowArtworkRuntime()
  let store = MusicStore(runtime: runtime)
  var firstTrack = Track(
    id: "same-track", title: "QA", artist: "QA", album: "QA", duration: 1, format: "wav",
    sample_rate: 48000, bit_depth: 16)
  firstTrack.cover_url = "/api/v1/covers/first"
  var secondTrack = firstTrack
  secondTrack.cover_url = "/api/v1/covers/updated"
  let first = Task { [firstTrack] in _ = await store.artwork.image(firstTrack, store: store) }
  let second = Task { [secondTrack] in _ = await store.artwork.image(secondTrack, store: store) }
  let deadline = ContinuousClock.now + .seconds(3)
  while runtime.started < 2 && ContinuousClock.now < deadline { await Task.yield() }
  #expect(runtime.started == 2 && store.artwork.pendingCount == 2)
  first.cancel()
  second.cancel()
  await first.value
  await second.value
  let end = ContinuousClock.now + .seconds(3)
  while runtime.active > 0 && ContinuousClock.now < end { await Task.yield() }
  #expect(runtime.active == 0 && store.artwork.pendingCount == 0)
}

@Test @MainActor func albumAndTrackWithTheSameArtworkShareTheAuthenticatedCoverURLWithoutATrackLookup() async throws {
  let runtime = SlowArtworkRuntime()
  let sharedStore = MusicStore(runtime: runtime)
  var album = Track(id: "album-identity", title: "Album", artist: "Artist", album: "Album", duration: 0, format: "", sample_rate: 0, bit_depth: 0)
  album.cover_url = "https://images.example/cover.jpg"
  let song = Track(id: "song-identity", title: "Song", artist: "Artist", album: "Album", duration: 60, format: "flac", sample_rate: 48000, bit_depth: 24, cover_url: album.cover_url)
  let first = Task { await sharedStore.artwork.image(album, store: sharedStore) }
  let second = Task { await sharedStore.artwork.image(song, store: sharedStore) }
  let deadline = ContinuousClock.now + .seconds(3)
  while runtime.started == 0 && ContinuousClock.now < deadline { await Task.yield() }
  #expect(runtime.started == 1)
  #expect(runtime.bodies.first?["cover_url"].string == album.cover_url)
  #expect(sharedStore.artwork.pendingCount == 1)
  first.cancel(); second.cancel(); _ = await first.value; _ = await second.value
  #expect(sharedStore.artwork.pendingCount == 0)
}

@MainActor private final class FailingArtworkRuntime: RuntimeTransport {
    let status: Int
    var started = 0
    init(status: Int) { self.status = status }
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        guard operation == "artwork" else { return .null }
        started += 1
        if started == 1 { throw RuntimeFailure(message: "synthetic image failure", status: status) }
        return .null
    }
    func shutdown() async {}
}
@Test @MainActor func artworkTransientFailureRetriesOnceAndSharesTheRetry() async {
    let runtime = FailingArtworkRuntime(status: 503)
    let shared = MusicStore(runtime: runtime)
    let track = Track(id: "retry", title: "QA", artist: "QA", album: "QA", duration: 0, format: "", sample_rate: 0, bit_depth: 0, cover_url: "https://example.test/a.jpg")
    async let first = shared.artwork.image(track, store: shared)
    async let second = shared.artwork.image(track, store: shared)
    _ = await (first, second)
    #expect(runtime.started == 2 && shared.artwork.pendingCount == 0)
    await shared.shutdown()
}
@Test @MainActor func artworkMissingImageDoesNotRetryAndCancellationStopsBackoff() async throws {
    let track = Track(id: "missing", title: "QA", artist: "QA", album: "QA", duration: 0, format: "", sample_rate: 0, bit_depth: 0, cover_url: "https://example.test/a.jpg")
    let missing = FailingArtworkRuntime(status: 404), first = MusicStore(runtime: missing)
    _ = await first.artwork.image(track, store: first)
    _ = await first.artwork.image(track, store: first)
    #expect(missing.started == 1)
    let delayed = FailingArtworkRuntime(status: 503), second = MusicStore(runtime: delayed)
    let task = Task { await second.artwork.image(track, store: second) }
    while delayed.started == 0 { await Task.yield() }
    task.cancel(); _ = await task.value
    try await Task.sleep(for: .milliseconds(30))
    #expect(delayed.started == 1 && second.artwork.pendingCount == 0)
    await first.shutdown(); await second.shutdown()
}

@MainActor private final class DecodeArtworkRuntime: RuntimeTransport {
  let good: String
  var bodies: [JSONValue] = []
  init(_ path: String) { good = path }
  func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
    guard operation == "artwork" else { return .null }
    bodies.append(body)
    return .object(["path": .string(bodies.count == 1 ? good + ".missing" : good)])
  }
  func shutdown() async {}
}
@Test @MainActor func artworkDecodeFailureRepairsOnceAndRefreshRevalidatesTheSharedCache() async throws {
  let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  defer { try? FileManager.default.removeItem(at: dir) }
  let image = try #require(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
  let file = dir.appendingPathComponent("fixture.png")
  try #require(image.representation(using: .png, properties: [:])).write(to: file)
  let runtime = DecodeArtworkRuntime(file.path), store = MusicStore(runtime: runtime)
  let track = Track(id: "decode", title: "QA", artist: "QA", album: "QA", duration: 0, format: "", sample_rate: 0, bit_depth: 0, cover_url: "/api/v1/artwork/stable")
  #expect(await store.artwork.image(track, store: store) != nil)
  #expect(runtime.bodies.count == 2 && runtime.bodies[1]["reload"].boolean)
  #expect(await store.artwork.image(track, store: store) != nil)
  // NSCache may evict a decoded entry under system memory pressure; another
  // runtime read reuses its disk cache and is not another upstream download.
  #expect(runtime.bodies.count <= 3)
  let beforeRefresh = runtime.bodies.count
  let old = store.artwork.revision
  store.artwork.reset(revalidate: true)
  #expect(old != store.artwork.revision)
  #expect(await store.artwork.image(track, store: store) != nil)
  #expect(runtime.bodies.count == beforeRefresh + 1 && runtime.bodies.last?["revalidate"].boolean == true)
  await store.shutdown()
}

@MainActor private final class FirstMissArtworkRuntime: RuntimeTransport {
  let path: String
  var calls: [String:Int] = [:]
  init(_ path: String) { self.path = path }
  func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
    guard operation == "artwork" else { return .null }
    let source = body["cover_url"].text
    calls[source,default:0] += 1
    if source == "/first-miss" && calls[source] == 1 { throw RuntimeFailure(message: "首次图片尚不可用", status: 404) }
    return .object(["path": .string(path)])
  }
  func shutdown() async {}
}
@Test @MainActor func retryingOneFirstMissDoesNotDiscardOtherDecodedPictures() async throws {
  let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  defer { try? FileManager.default.removeItem(at: dir) }
  let bitmap = try #require(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
  let path = dir.appendingPathComponent("image.png"); try #require(bitmap.representation(using: .png, properties: [:])).write(to: path)
  let runtime = FirstMissArtworkRuntime(path.path), store = MusicStore(runtime: runtime)
  let first = Track(id: "first", title: "Artist", artist: "Artist", album: "", duration: 0, format: "", sample_rate: 0, bit_depth: 0, cover_url: "/first-miss")
  var other = first; other.cover_url = "/other"
  #expect(await store.artwork.image(other, store: store) != nil)
  #expect(await store.artwork.image(first, store: store) == nil)
  #expect(await store.artwork.image(first, store: store) == nil)
  #expect(runtime.calls["/first-miss"] == 1)
  let revision = store.artwork.revision
  store.artwork.retry(first, store: store)
  #expect(store.artwork.revision == revision) // A single retry must not perform a global reset.
  #expect(await store.artwork.image(first, store: store) != nil)
  #expect(await store.artwork.image(other, store: store) != nil)
  #expect(runtime.calls["/first-miss"] == 2 && (runtime.calls["/other"] ?? 0) <= 2)
  await store.shutdown()
}
@Test func platformQualityDeclarationNeverInventsDetectedAudioParameters() {
  let track = Track(id: "declared", title: "Song", artist: "Artist", album: "Album", duration: 10, format: "unknown", sample_rate: 0, bit_depth: 0, source: "fixture", provider_data: .object(["quality_hint": .object(["label": .string("FLAC 无损"), "verified": .bool(false)])]))
  #expect(track.quality == "FLAC 无损 · 平台声明")
  #expect(track.formatLabel == "FLAC 无损 · 平台声明")
  #expect(track.sample_rate == 0 && track.bit_depth == 0)
}

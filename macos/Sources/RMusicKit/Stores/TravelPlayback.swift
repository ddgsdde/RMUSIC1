import Foundation

/// The travelling Mac owns its queue. Core supplies track metadata and short-lived media URLs.
@MainActor final class TravelPlayback {
  private var queues: [String: Zone] = [:]
  private var known: [String: Track] = [:]
  private(set) var generation = UUID()
  private var preparedID: String?
  private var scope: String?
  private let writer = DispatchQueue(label: "com.rmusic.travel-library", qos: .utility)
  private var changedAt = Date.distantPast
  private var preparing: String?
  private var nextUp: (instance: String, index: Int, track: String)?
  private struct Retry {
    var instance: String
    var position: Double
    var attempts: Int
    var waiting = false
    var due: Date
  }
  private var radioPreview: [Track] = []
  private var radioRetry: (instance: String, due: Date, delay: Double)?
  private var recommendationID = UUID()
  private var skippedRadio = Set<String>()
  private var recommendations: (instance: String, task: Task<[Track], Error>)?
  private var retry: Retry?
  private var heldPosition: (String, Double, Date)?
  private var remotePublish: Task<Void, Never>?
  private var lastPublish = Date.distantPast
  private var remotePublished = false
  private var remoteQueue: [String] = []

  private func remoteID(_ store: MusicStore) -> String {
    let suffix = store.user["id"].text.filter { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") }
    return "portable-" + store.endpointID + "-" + suffix
  }
  private func remoteZone(_ store: MusicStore) -> Zone? {
    let active = queues.values.first { $0.status == "playing" }
    let selected = queues.values.first { $0.id == store.selectedZoneID }
    return active ?? selected ?? store.outputs.enabled.compactMap { queues[$0.id] }.first
  }
  private func snapshot(_ current: Zone, store: MusicStore, includeQueue: Bool = true) throws -> JSONValue {
    var projection = current
    if !includeQueue { projection.queue = [] }
    var value = try JSONValue.encoded(projection).object
    if !includeQueue { value.removeValue(forKey: "queue") }
    value["buffering"] = .bool(current.track != nil && current.track?.id == store.bufferingTrack)
    value["name"] = .string("Mac · " + current.name)
    return .object(value)
  }
  func publishRemote(_ store: MusicStore, force: Bool = false) {
    guard store.connected, store.outputs.roaming, remotePublish == nil, let zone = remoteZone(store) else { return }
    guard force || Date.now.timeIntervalSince(lastPublish) >= (zone.status == "playing" ? 3 : 25) else { return }
    let order = zone.queue.map(\.id)
    guard let state = try? snapshot(zone, store: store, includeQueue: force || !remotePublished || order != remoteQueue) else { return }
    let path = "/portable/\(remoteID(store))/state"
    let body = JSONValue.object(["platform": .string("macos"), "stamp": .number((Date.now.timeIntervalSince1970 * 1000).rounded(.down)), "zone": state])
    lastPublish = .now
    remotePublished = true
    remotePublish = Task { [weak self, weak store] in
      defer { self?.remotePublish = nil }
      do {
        guard let store else { return }
        _ = try await store.api(path, method: "PUT", body: body)
        self?.remoteQueue = order
      } catch {
        self?.remoteQueue = []
        self?.lastPublish = .distantPast
      }
    }
  }
  func withdrawRemote(_ store: MusicStore) async {
    guard remotePublished else { return }
    remotePublished = false
    let pending = remotePublish
    remoteQueue = []
    lastPublish = .distantPast
    let path = "/portable/\(remoteID(store))/state"
    await pending?.value
    guard !remotePublished else { return }
    _ = try? await store.api(path, method: "DELETE")
  }
  func receiveRemote(_ message: JSONValue, store: MusicStore) async {
    guard store.connected, store.outputs.roaming, message["id"].string == remoteID(store),
      message["expires_at"].number >= Date.now.timeIntervalSince1970 * 1000,
      let requestID = message["request_id"].string else { return }
    let output = message["output"].string
    let destination = output.flatMap { queues[$0] } ?? (output == nil || output == remoteID(store) ? remoteZone(store) : nil)
    var ack: [String: JSONValue] = ["request_id": .string(requestID)]
    do {
      guard let zone = destination else { throw RuntimeFailure(message: "该声音输出已断开") }
      let action = message["action"].text
      var body = message["body"]
      if action == "queue", message["method"].text == "DELETE" { body = .object(["track_ids": .array([])]) }
      _ = try await request(zone.id, action: action, method: message["method"].text, body: body, store: store)
      guard let current = queues[zone.endpoint_id] else { return }
      ack["zone"] = try snapshot(current, store: store)
    } catch { ack["error"] = .string(error.localizedDescription) }
    ack["stamp"] = .number((Date.now.timeIntervalSince1970 * 1000).rounded(.down))
    _ = try? await store.api("/portable/\(remoteID(store))/ack", method: "POST", body: .object(ack))
  }
  func receiveLoudness(_ message: JSONValue, store: MusicStore) async {
    guard store.outputs.roaming, let trackID = message["track_id"].string,
          case .number(let measured) = message["integrated_lufs"], measured.isFinite else { return }
    let peak = message.object["true_peak_dbfs"]?.number
    func update(_ track: inout Track) {
      track.integrated_lufs = measured
      track.true_peak_dbfs = peak
    }
    if var cached = known[trackID] { update(&cached); known[trackID] = cached }
    for output in Array(queues.keys) {
      guard var zone = queues[output] else { continue }
      for index in zone.queue.indices where zone.queue[index].id == trackID { update(&zone.queue[index]) }
      let current = zone.track?.id == trackID && zone.playback_id == message["playback_id"].string
      if current, var track = zone.track { update(&track); zone.track = track }
      queues[output] = zone
      if let index = store.zones.firstIndex(where: { $0.id == zone.id }) { store.zones[index] = zone }
      if current, let id = zone.playback_id, zone.dsp?["normalize"].boolean == true {
        _ = try? await store.runtimeCall("stream_control", .object([
          "type":.string("loudness"), "output":.string(output), "playback_id":.string(id),
          "integrated_lufs":.number(measured), "true_peak_dbfs":peak.map(JSONValue.number) ?? .null,
        ]))
      }
    }
  }
  private func bind(_ store: MusicStore) {
    guard let core = store.profile.core_id, let user = store.user["id"].string else { return }
    let key = "travelQueue.v1." + core + "." + user
    guard scope != key else { return }
    scope = key
    if let data = writer.sync(execute: { UserDefaults.standard.data(forKey: key) }),
      let saved = try? JSONDecoder().decode([String: Zone].self, from: data)
    {
      queues = saved
      for id in Array(queues.keys) {
        queues[id]?.playback_id = nil
        let restoredStatus = queues[id]?.track == nil ? "stopped" : "paused"
        queues[id]?.status = restoredStatus
        if let zone = queues[id],
          let cursor = UserDefaults.standard.dictionary(forKey: key + "." + id),
          cursor["track"] as? String == zone.track?.id, let p = cursor["position"] as? Double
        {
          queues[id]?.position = p
        }
        remember(queues[id]?.queue ?? [])
      }
    }
  }
  private func saveQueue() {
    guard let key = scope else { return }
    let copy = queues
    writer.async {
      // Persist display metadata and queue order, never a ticket or provider payload.
      let compact = copy.mapValues { original in
        var zone = original
        zone.queue = zone.queue.map {
          Track(
            id: $0.id, title: $0.title, artist: $0.artist, album: $0.album, duration: $0.duration,
            format: $0.format, sample_rate: $0.sample_rate, bit_depth: $0.bit_depth,
            cover_url: $0.cover_url, source: $0.source)
        }
        zone.track =
          zone.queue.indices.contains(zone.queue_index) ? zone.queue[zone.queue_index] : nil
        zone.playback_id = nil
        return zone
      }
      if let data = try? JSONEncoder().encode(compact) {
        UserDefaults.standard.set(data, forKey: key)
      }
    }
  }
  private func cursor(_ z: Zone) {
    if let key = scope, let id = z.track?.id {
      UserDefaults.standard.set(
        ["track": id, "position": z.position], forKey: key + "." + z.endpoint_id)
    }
  }
  func nextIndex(_ zone: Zone, ended: Bool) -> Int? {
    guard !zone.queue.isEmpty else { return nil }
    if ended && zone.repeat == "one" { return zone.queue_index }
    if zone.queue.indices.contains(zone.queue_index + 1) { return zone.queue_index + 1 }
    return zone.repeat == "all" ? 0 : nil
  }
  private func show(_ zone: Zone, store: MusicStore) {
    queues[zone.endpoint_id] = zone
    if let i = store.zones.firstIndex(where: { $0.id == zone.id }) { store.zones[i] = zone }
    if store.selectedZoneID == zone.id {
      store.presentationClock.record(zone, local: true)
      store.updateSystemPlayback()
    }
  }
  func enter(_ old: Zone?, store: MusicStore) async throws {
    bind(store)
    guard let old, let output = store.outputs.enabled.first(where: { $0.id == old.endpoint_id }),
      let track = old.track, ["playing", "paused"].contains(old.status) || (old.status == "stopped" && old.position > 0)
    else { return }
    var z = old
    if z.queue.indices.contains(z.queue_index), z.queue[z.queue_index].id == track.id {
      z.position = old.status == "playing" ? store.presentationClock.reading(old).position : old.position
    } else {
      z.queue = [track]
      z.queue_index = 0
    }
    z.id = "travel-" + output.id
    z.playback_id = nil
    let playing = old.status == "playing"
    z.status = "paused"
    remember(z.queue)
    queues[output.id] = z
    store.selectedZoneID = z.id
    cursor(z)
    saveQueue()
    if playing { try await start(z, index: z.queue_index, store: store, position: z.position) }
  }

  struct HomeHandoff {
    let zone: Zone
    let index: Int
    let pausedPosition: Double?
  }
  func returning(_ report: JSONValue, selectedOutput: String? = nil) -> HomeHandoff? {
    guard let id = report["output"].string ?? selectedOutput, let z = queues[id],
          z.queue.indices.contains(z.queue_index), z.track?.id == z.queue[z.queue_index].id else { return nil }
    let state = report["report"]
    if let instance = state["core_playback_id"].string, instance != z.playback_id { return nil }
    if state["status"].string == "ended" {
      return nextIndex(z, ended: true).map { HomeHandoff(zone: z, index: $0, pausedPosition: nil) }
    }
    guard z.status == "paused" || state["status"].string == "paused" else { return nil }
    let position = z.status == "paused" ? z.position : state.object["position"]?.number ?? z.position
    return HomeHandoff(zone: z, index: z.queue_index, pausedPosition: position)
  }
  func checkpoint(_ store: MusicStore) {
    for (id, var zone) in queues where zone.track != nil {
      if zone.status == "playing", store.selectedZoneID == zone.id {
        zone.position = store.presentationClock.reading(zone).position
        queues[id] = zone
      }
      cursor(zone)
    }
    if let scope, store.outputs.roaming, let zone = store.localZone,
       zone.track != nil, ["playing", "paused"].contains(zone.status) {
      UserDefaults.standard.set(zone.endpoint_id, forKey: scope + ".pendingHome")
    }
    writer.sync {}
  }
  func savedReturn(_ store: MusicStore) -> HomeHandoff? {
    guard let core = store.profile.core_id, let user = store.user["id"].string else { return nil }
    let key = "travelQueue.v1." + core + "." + user + ".pendingHome"
    guard let output = UserDefaults.standard.string(forKey: key) else { return nil }
    bind(store)
    return returning(.null, selectedOutput: output)
  }
  func finishReturn(_ store: MusicStore) {
    guard let core = store.profile.core_id, let user = store.user["id"].string else { return }
    UserDefaults.standard.removeObject(forKey: "travelQueue.v1." + core + "." + user + ".pendingHome")
  }
  func remember(_ tracks: [Track]) {
    if known.count > 20_000 { known.removeAll() }
    for track in tracks { known[track.id] = track }
  }
  func reset() {
    for z in queues.values { cursor(z) }
    scope = nil
    nextUp = nil
    retry = nil
    preparing = nil
    radioPreview = []; skippedRadio = []; radioRetry = nil; recommendationID = UUID()
    heldPosition = nil
    generation = UUID()
    queues = [:]
    known = [:]
    preparedID = nil
  }
  func empty(_ output: NativeOutput) -> Zone {
    Zone(
      id: "travel-" + output.id, name: output.name, endpoint_id: output.id, status: "stopped",
      track: nil, position: 0, volume: 1, muted: false, playback_id: nil, queue_index: 0, queue: [], autoplay: .object(["enabled": .bool(true), "source": .string("local"), "fallback_local": .bool(true)]))
  }
  func refresh(_ store: MusicStore, report: JSONValue) async throws {
    bind(store)
    let available = store.outputs.enabled.filter(\.available)
    for id in Array(queues.keys)
    where !available.contains(where: { $0.id == id })
      || (report["stream_output"].string == nil && queues[id]?.playback_id != nil
        && queues[id]?.playback_id != preparing)
    {
      if preparedID == queues[id]?.playback_id { preparedID = nil }
      queues[id]?.status = "stopped"
      queues[id]?.playback_id = nil
    }
    for output in available {
      var zone = queues[output.id] ?? empty(output)
      let live = report["stream"]
      if report["stream_output"].string == output.id, let instance = zone.playback_id,
        live["core_playback_id"].string == instance
      {
        if live["type"].text == "error" {
          if live["source_unavailable"].boolean, zone.status == "playing", zone.queue.indices.contains(zone.queue_index + 1) {
            store.error = live["error"].text
            try await start(zone, index: zone.queue_index + 1, store: store)
            continue
          }
          if zone.status == "paused" {
            preparedID = nil
            retry = nil
            zone.playback_id = nil
            queues[output.id] = zone
            continue
          }
          if live["source_failed"].boolean {
            let position = max(0, live.object["position"]?.number ?? zone.position)
            if retry?.instance != instance {
              retry = Retry(
                instance: instance, position: position, attempts: 0,
                due: Date().addingTimeInterval(1))
            }
            retry?.waiting = true
            if let pending = retry, pending.attempts < 4 {
              store.bufferingTrack = zone.track?.id
              if Date() >= pending.due {
                try await start(
                  zone, index: zone.queue_index, store: store, position: pending.position,
                  retryCount: pending.attempts + 1)
              }
              continue
            }
          }
          store.bufferingTrack = nil
          _ = try? await store.runtimeCall("local_transport",.object(["type":.string("stop"),"output":.string(output.id),"playback_id":.string(instance)]))
          guard queues[output.id]?.playback_id==instance else {continue}
          store.error = live["error"].text
          zone.position = live.object["position"]?.number ?? zone.position
          zone.status = "paused"
          zone.playback_id = nil
          preparedID = nil
          retry = nil
        } else if live["status"].text == "ended" {
          guard zone.status == "playing" else { continue }
          if report["home_available"].boolean {
            recommendations?.task.cancel(); recommendations = nil; radioRetry = nil
            zone.status = "stopped"; zone.playback_id = nil; preparedID = nil
            queues[output.id] = zone
            return
          }
          if nextIndex(zone, ended: true) == nil, zone.autoplay?["enabled"].boolean == true {
            if let pending = radioRetry, pending.instance == instance, pending.due > .now { continue }
            prepareRecommendations(zone, instance: instance, store: store)
            if let pending = recommendations, pending.instance == instance {
              let request = recommendationID
              let rows = (try? await pending.task.value) ?? []
              guard recommendationID == request, queues[output.id]?.playback_id == instance,
                    queues[output.id]?.status == "playing", recommendations?.instance == instance else { continue }
              zone = queues[output.id] ?? zone
              let additions = rows.filter { !skippedRadio.contains($0.id) }
              if !additions.isEmpty {
                let trim = min(zone.queue_index, max(0, zone.queue.count - 200))
                zone.queue.removeFirst(trim)
                zone.queue_index -= trim
                zone.queue += additions
              } else {
                let delay = radioRetry.map { min(30, $0.delay * 2) } ?? 5
                radioRetry = (instance, .now.addingTimeInterval(delay), delay)
                recommendations = nil
                store.bufferingTrack = zone.track?.id
                publishRemote(store, force: true)
                continue
              }
              remember(zone.queue)
              queues[output.id] = zone
              saveQueue()
            }
          }
          recommendations = nil; radioPreview = []; skippedRadio = []; radioRetry = nil
          zone.status = "stopped"
          zone.playback_id = nil
          preparedID = nil
          queues[output.id] = zone
          if let next = plannedIndex(zone, instance: instance) ?? nextIndex(zone, ended: true) {
            try await start(zone, index: next, store: store)
            continue
          }
        } else if ["playing", "paused"].contains(live["status"].text) {
          store.bufferingTrack = nil
          let position = live["position"].number
          if zone.track?.duration ?? 0 > 0, (zone.track?.duration ?? 0) - position < 30,
            zone.repeat == "off", zone.queue.count - zone.queue_index <= 4, zone.autoplay?["enabled"].boolean == true {
            prepareRecommendations(zone, instance: instance, store: store)
          }
          if let pending = retry, pending.instance == instance, position > pending.position + 1 {
            retry = nil
          }
          let duration = live.object["duration"]?.number ?? zone.track?.duration ?? 0
          if live["status"].text == "playing", zone.status == "playing", duration > 0,
            duration - position < 12,
            !report["home_available"].boolean, !live["filling_current"].boolean,
            plannedIndex(zone, instance: instance) == nil,
            let next = nextIndex(zone, ended: true)
          {
            let id = zone.queue[next].id
            nextUp = (instance, next, id)
            _ = try? await store.runtimeCall(
              "stream_control",
              .object([
                "type": .string("preload"), "output": .string(output.id),
                "playback_id": .string(instance), "next_track": .string(id),
              ]))
            guard queues[output.id]?.playback_id == instance else { continue }
            // A control can finish while the native prefetch is being scheduled.
            zone = queues[output.id] ?? zone
          }
          let awaiting =
            heldPosition.map {
              $0.0 == instance && Date().timeIntervalSince($0.2) < 2 && abs(position - $0.1) > 1.5
            } ?? false
          if !awaiting && zone.status != "paused" { zone.position = position }
          if Date().timeIntervalSince(changedAt) > 15 {
            cursor(zone)
            changedAt = Date()
          }
          if store.selectedZoneID == zone.id {
            let path = live["signal_path"]
            if path["sample_rate"].integer > 0 {
              store.signal =
                "\(path["sample_rate"].integer) Hz · \(path["bit_depth"].integer) bit · \(path["channels"].integer) 声道"
            }
            store.signalEndpoint = .object(["id": .string(output.id), "playback": live])
          }
        }
      }
      queues[output.id] = zone
    }
    store.zones = available.compactMap { queues[$0.id] } + store.family.zones.filter { $0.id != remoteID(store) }
    publishRemote(store)
    let candidates = store.playbackZones
    store.restorePlaybackSelection(in: candidates, fallback: candidates.first)
    store.onlineEndpoints = Set(available.map(\.id)).union(
      store.family.endpoints.filter { $0["online"].boolean }.map { $0["id"].text })
    if let selected = store.zone, !selected.id.hasPrefix("travel-") {
      store.signalEndpoint =
        store.family.endpoints.first { $0["id"].text == selected.endpoint_id } ?? .null
    }
    if let zone = store.zone { store.presentationClock.record(zone, local: zone.id.hasPrefix("travel-")) }
    store.details.update(store.zone?.track, store: store)
    store.details.prefetch(store.zone, store: store)
    store.updateSystemPlayback()
  }
  private func prepareRecommendations(_ zone: Zone, instance: String, store: MusicStore) {
    guard recommendations?.instance != instance, let seed = zone.queue.last ?? zone.track else { return }
    recommendations?.task.cancel()
    let requestID = UUID(); recommendationID = requestID
    let recent = Array(zone.queue.suffix(200).reversed())
    let excluded = skippedRadio
    let cached = Set(store.downloads.entries.lazy.filter(\.complete).map(\.track_id))
    recommendations = (instance, Task {
      let body = JSONValue.object([
        "seed_track_id": .string(seed.id), "exclude": .array(excluded.sorted().map(JSONValue.string)),
        "context": .object(["tail": .array(recent.prefix(12).map { .string($0.id) }),
                            "recent": .array(recent.map { .string($0.id) })])
      ])
      let request = Task { try await store.api("/recommendations/radio", method: "POST", body: body) }
      let timeout = Task {
        do { try await Task.sleep(for: .seconds(4)); request.cancel() } catch { }
      }
      defer { timeout.cancel() }
      let result = try? await withTaskCancellationHandler { try await request.value } onCancel: { request.cancel() }
      try Task.checkCancellation()
      var tracks = (try? result?["items"].decode([Track].self)) ?? []
      if tracks.isEmpty {
        var seen = excluded
        tracks = Array(recent.reversed().lazy.filter { $0.id != seed.id && cached.contains($0.id) && seen.insert($0.id).inserted }.prefix(5))
        if tracks.isEmpty, cached.contains(seed.id), !excluded.contains(seed.id) { tracks = [seed] }
      }
      if recommendationID == requestID, recommendations?.instance == instance {
        radioPreview = tracks
        store.radioRevision += 1
      }
      return tracks
    })
  }
  private func plannedIndex(_ z: Zone, instance: String) -> Int? {
    guard let n = nextUp, n.instance == instance, z.queue.indices.contains(n.index),
      z.queue[n.index].id == n.track
    else { return nil }
    return n.index
  }
  private func start(
    _ original: Zone, index: Int, store: MusicStore, position: Double = 0, retryCount: Int = 0
  )
    async throws
  {
    guard original.queue.indices.contains(index),
      let output = store.outputs.items.first(where: {
        $0.id == original.endpoint_id && $0.enabled && $0.available
      })
    else { throw RuntimeFailure(message: "先在这台 Mac 上启用输出并选择歌曲。") }
    let stamp = UUID()
    generation = stamp
    let entry = original.queue[index]
    let track = known[entry.id] ?? entry
    let settings = store.effects.zone(for: output, base: original)
    let playback = retryCount > 0 ? (original.playback_id ?? UUID().uuidString) : UUID().uuidString
    nextUp = nil
    retry =
      retryCount == 0
      ? nil
      : Retry(
        instance: playback, position: position, attempts: retryCount,
        due: Date().addingTimeInterval(pow(2, Double(retryCount))))
    var zone = original
    zone.track = track
    zone.dsp = settings.dsp
    zone.transitions = settings.transitions
    zone.queue_index = index
    zone.playback_id = playback
    zone.position = position
    zone.status = "playing"
    for id in Array(queues.keys) where id != output.id {
      queues[id]?.status = "paused"
      queues[id]?.playback_id = nil
    }
    store.bufferingTrack = track.id
    show(zone, store: store)
    preparing = playback
    preparedID = playback
    saveQueue()
    cursor(zone)
    var command: [String: JSONValue] = [
      "type": .string("play"), "output": .string(output.id), "zone_id": .string(original.id),
      "playback_id": .string(playback), "track": try .encoded(track), "position": .number(position),
      "volume": .number(original.volume), "muted": .bool(original.muted),
    ]
    if retryCount > 0 { command["recover"] = .bool(true) }
    if let effects = settings.dsp { command["dsp"] = effects }
    if let transitions = settings.transitions { command["transitions"] = transitions }
    do {
      _ = try await store.runtimeCall("stream_control", .object(command))
    } catch {
      if preparing == playback {
        preparing = nil
        preparedID = nil
        zone.status = "paused"
        zone.playback_id = nil
        show(zone, store: store)
      }
      throw error
    }
    if preparing == playback { preparing = nil }
    guard generation == stamp, store.outputs.roaming else { return }
  }
  func request(_ zoneID: String, action: String, method: String, body: JSONValue, store: MusicStore)
    async throws -> JSONValue
  {
    guard let output = store.outputs.enabled.first(where: { "travel-" + $0.id == zoneID }),
      var z = queues[output.id]
    else { throw RuntimeFailure(message: "此设备已断开。") }
    func control(_ action: String, _ fields: [String: JSONValue] = [:]) async throws {
      _ = try await store.runtimeCall(
        preparing == z.playback_id ? "stream_control" : "local_transport",
        .object(
          fields.merging([
            "type": .string(action), "output": .string(output.id),
            "playback_id": .string(z.playback_id ?? ""),
          ]) { _, v in v }))
    }
    if ["queue", "queue/reorder"].contains(action) || body.object["repeat"] != nil
      || body.object["shuffle"] != nil
    {
      if nextUp != nil, let instance = z.playback_id, preparedID == instance {
        _ = try? await store.runtimeCall(
          "stream_control",
          .object([
            "type": .string("preload"), "output": .string(output.id),
            "playback_id": .string(instance),
          ]))
      }
      nextUp = nil
    }
    if ["queue", "queue/reorder", "play", "pause", "seek", "stop", "previous", "next"].contains(action) || body.object["autoplay"] != nil {
      recommendations?.task.cancel()
      recommendations = nil; radioPreview = []; skippedRadio = []; radioRetry = nil; recommendationID = UUID()
    }
    if action == "radio" {
      if method == "POST", let track = radioPreview.first(where: { !skippedRadio.contains($0.id) }), track.id == body["track_id"].string {
        skippedRadio.insert(track.id)
        if radioPreview.allSatisfy({ skippedRadio.contains($0.id) }), let instance = z.playback_id {
          recommendations?.task.cancel(); recommendations = nil
          prepareRecommendations(z, instance: instance, store: store)
        }
      }
      let items = Array(radioPreview.filter { !skippedRadio.contains($0.id) }.prefix(3))
      return .object(["status": .string(items.isEmpty ? (radioRetry != nil ? "waiting" : recommendations != nil ? "preparing" : "idle") : "ready"), "items": try .encoded(items)])
    }
    switch action {
    case "queue":
      let ids = body["track_ids"].array.compactMap(\.string)
      guard ids.count <= 10_000 else { throw RuntimeFailure(message: "一次最多添加一万首。") }
      let rows = ids.map {
        known[$0]
          ?? Track(
            id: $0, title: $0, artist: "", album: "", duration: 0, format: "", sample_rate: 0,
            bit_depth: 0)
      }
      if method == "POST" {
        z.queue.insert(
          contentsOf: rows,
          at: body["play_next"].boolean ? min(z.queue_index + 1, z.queue.count) : z.queue.count)
      } else {
        if preparedID != nil && preparedID == z.playback_id {
          try await control("stop")
          preparedID = nil
        }
        z.queue = rows
        z.queue_index = max(0, min(body["queue_index"].integer, rows.count - 1))
        z.track = rows.indices.contains(z.queue_index) ? rows[z.queue_index] : nil
        z.status = "stopped"
        z.playback_id = nil
        z.position = 0
      }
    case "queue/reorder":
      let order = body["order"].array.map(\.integer)
      guard order.sorted() == Array(z.queue.indices),
        body["expected_queue"].array.compactMap(\.string) == z.queue.map(\.id)
      else { throw RuntimeFailure(message: "队列已变化，请重新排序。") }
      z.queue = order.map { z.queue[$0] }
      z.queue_index = order.firstIndex(of: z.queue_index) ?? 0
    case "play", "next", "previous":
      if let id = body["track_id"].string {
        if !z.queue.contains(where: { $0.id == id }) {
          z.queue.append(
            try await store.api("/library/tracks/\(id.pathComponentEncoded)").decode(Track.self))
        }
        try await start(z, index: z.queue.firstIndex(where: { $0.id == id })!, store: store)
        return .null
      }
      if action == "play", case .null = body["queue_index"], z.status == "paused",
        preparedID != nil, preparedID == z.playback_id
      {
        z.status = "playing"
        show(z, store: store)
        do { try await control("resume") } catch {
          if queues[output.id]?.playback_id == z.playback_id {
            z.status = "paused"
            show(z, store: store)
          }
          throw error
        }
        return .null
      }
      let index =
        action == "next"
        ? (nextIndex(z, ended: false) ?? z.queue_index)
        : action == "previous"
          ? max(0, z.queue_index - 1) : body.object["queue_index"]?.integer ?? z.queue_index
      try await start(
        z, index: index, store: store,
        position: action == "play" && body.object["queue_index"] == nil && z.status == "paused"
          ? z.position : 0)
      return .null
    case "stop", "pause", "seek":
      generation = UUID()
      let old = z
      var active = preparedID != nil && preparedID == z.playback_id
      if let pending = retry, pending.waiting, pending.instance == z.playback_id {
        retry = nil
        preparedID = nil
        z.playback_id = nil
        active = false
        z.position = pending.position
        if action == "seek" { z.status = "paused" }
      }
      if action == "pause" {
        retry = nil
        store.bufferingTrack = nil
        if active { z.position = store.presentationClock.reading(z).position }
        z.status = "paused"
      }
      if action == "seek" {
        z.position = max(0, body["position"].number)
        heldPosition = (z.playback_id ?? "", z.position, Date())
      }
      if action == "stop" {
        z.status = "stopped"
        z.position = 0
      }
      store.presentationClock.reset()
      show(z, store: store)
      cursor(z)
      do { if active { try await control(action, body.object) } } catch {
        if queues[output.id]?.playback_id == old.playback_id { show(old, store: store); cursor(old) }
        throw error
      }
      guard queues[output.id]?.playback_id == z.playback_id else { return .null }
      if action != "stop" { return .null }
      if preparedID == z.playback_id { preparedID = nil }
      do {
        z.status = "stopped"
        z.position = 0
        z.playback_id = nil
      }

    case "" where method == "PATCH":
      guard body.object.keys.allSatisfy({ ["muted", "volume", "shuffle", "repeat", "autoplay"].contains($0) })
      else {
        throw RuntimeFailure(message: "该设置仅在局域网音区可用。")
      }
      if let v = body.object["autoplay"] { z.autoplay = v }
      if let v = body.object["shuffle"] {
        z.shuffle = v.boolean
        if v.boolean, z.queue.indices.contains(z.queue_index) {
          let tail = z.queue.index(after: z.queue_index)
          z.queue.replaceSubrange(tail..., with: z.queue[tail...].shuffled())
        }
      }
      if let v = body.object["repeat"] { z.repeat = v.string }
      if let v = body.object["muted"] { z.muted = v.boolean }
      if let v = body.object["volume"] { z.volume = v.number }
      if preparedID != nil && preparedID == z.playback_id
        && (body.object["muted"] != nil || body.object["volume"] != nil)
      {
        try await control("volume", ["volume": .number(z.volume), "muted": .bool(z.muted)])
      }
    default: throw RuntimeFailure(message: "此功能需要局域网音区。")
    }
    show(z, store: store)
    saveQueue()
    cursor(z)
    return .null
  }
}

@MainActor final class DeviceEffects {
  private var cache: [String: [String: JSONValue]] = [:]
  private func key(_ uid: String) -> String { "outputEffects.v1." + uid }
  private func read(_ uid: String) -> [String: JSONValue] {
    if let saved = cache[uid] { return saved }
    let fields =
      UserDefaults.standard.data(forKey: key(uid)).flatMap {
        try? JSONDecoder().decode([String: JSONValue].self, from: $0)
      } ?? [:]
    cache[uid] = fields
    return fields
  }
  private func write(_ fields: [String: JSONValue], _ uid: String) throws {
    UserDefaults.standard.set(try JSONEncoder().encode(fields), forKey: key(uid))
    cache[uid] = fields
  }
  func zone(for output: NativeOutput, base: Zone) -> Zone {
    var zone = base
    let fields = read(output.uid)
    zone.transitions = fields["transitions"] ?? initialTransitions(base)
    zone.dsp = fields["dsp"] ?? base.dsp
    if base.id.hasPrefix("travel-") {
      zone.transitions = fields["travel.transitions"] ?? zone.transitions
      zone.dsp = fields["travel.dsp"] ?? zone.dsp
    }
    return zone
  }
  private func initialTransitions(_ base: Zone) -> JSONValue {
    if let saved = base.transitions, saved["fade_in_ms"].integer > 0 || saved["fade_out_ms"].integer > 0 { return saved }
    return PlaybackSettingsDraft.balancedTransitions
  }
  func edit(_ key: String, value: JSONValue, uid: String, portable: Bool = false) throws {
    var fields = read(uid)
    fields[portable ? "travel." + key : key] = value
    if !portable { fields["pending." + key] = .string(UUID().uuidString) }
    try write(fields, uid)
  }
  func reconcile(_ output: NativeOutput, zone: Zone, store: MusicStore) async throws {
    for name in ["transitions", "dsp"] {
      var fields = read(output.uid)
      if name == "transitions", fields[name] == nil {
        fields[name] = initialTransitions(zone)
        if fields[name] != zone.transitions { fields["pending." + name] = .string(UUID().uuidString) }
        try write(fields, output.uid)
      }
      if let version = fields["pending." + name]?.string, let change = fields[name] {
        try await store.requireUngrouped(zone.id)
        _ = try await store.api(
          "/zones/\(zone.id.pathComponentEncoded)" + (name == "dsp" ? "/dsp" : ""),
          method: name == "dsp" ? "PUT" : "PATCH",
          body: name == "dsp" ? change : .object([name: change]))
        fields = read(output.uid)
        if fields["pending." + name]?.string == version {
          fields.removeValue(forKey: "pending." + name)
          try write(fields, output.uid)
        }
      } else if let value = (name == "dsp" ? zone.dsp : zone.transitions), fields[name] != value {
        fields[name] = value
        try write(fields, output.uid)
      }
    }
  }
}

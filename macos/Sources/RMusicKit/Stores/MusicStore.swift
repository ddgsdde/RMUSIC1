import AppKit
import Foundation
import Observation

@MainActor @Observable
public final class MusicStore {
    let collection = CollectionStore()
    let similarSongs = SimilarSongsStore()
    let downloads = DownloadsStore()
    let details = PlaybackDetails()
    let pageSnapshots = PageSnapshots()
    let presentationClock = PlaybackPresentationClock()
    let artwork = ArtworkStore()
    let outputs = OutputsStore()
    let travel = TravelPlayback()
    let playbackCommands = PlaybackCommands()
    let family = TravelFamily()
    let effects = DeviceEffects()
    var streamBuffer = JSONValue.null
    var radioRevision = 0
    var lanSelection: String?
    private var pendingHome: TravelPlayback.HomeHandoff?
    private var checkedSavedHome = false
    let discovery = DiscoveryStore()
    let discoveryFeed = DiscoveryFeedStore()
    var sessionID: UUID { epoch }
    public let bridgeOnly: Bool
    var anonymousMetadata = UserDefaults.standard.bool(forKey: "anonymousQQMetadata")
    var streamingPolicy = StreamingPreferences.load()
    var profile = CoreProfile()
    var connectionIdentity: CoreIdentity?
    var connectionTrust = ConnectionTrustPreferences()
    var lanAddress = ""
    var remoteAddress = ""
    var username = ""
    var password = ""
    var totp = ""
    var user: JSONValue = .null
    var connected = false
    var connecting = false
    private(set) var savedSession: String?
    private(set) var restoreFailed = false
    var workspaceReady: Bool { connected || (!bridgeOnly && (awaitingSavedSession || savedSession != nil) && !restoreFailed) }
    private(set) var awaitingSavedSession = false
    private static let signedOutKey = "coreSessionSignedOut"
    @ObservationIgnored private var persistSessionState = false
    @ObservationIgnored private var readSavedSession: (() async -> String?)?
    @ObservationIgnored private var readingSavedSession = false
    var startupUnavailable: Bool { !connected && savedSession != nil && restoreFailed }
    var systemSleeping = false
    var uiActive = true
    private var queueEpoch: String?
    @ObservationIgnored private var zoneReadSequence = 0
    @ObservationIgnored private var appliedZoneRead = 0
    @ObservationIgnored private var outputRefreshTask: Task<Void, Never>?
    private var presentationVisible: Bool { uiActive || NSApp?.windows.contains(where: { $0.isVisible && $0.occlusionState.contains(.visible) }) == true }
    @ObservationIgnored private var presentationPolling: Task<Void, Never>?
    @ObservationIgnored private var collectionRefreshTask: Task<Void, Never>?
    var coreReachable = false
    var connectionIssue: String?
    var connectionLabel: String { connected ? (coreReachable ? "已连接 · \(username)" : "连接中断 · 正在重试") : "正在发现局域网音乐库" }
    var connectionAddress = ""
    var error: String?
    var notice: String?
    var tracks: [Track] = []
    var total = 0
    var page = 0
    var query = ""
    var artistID: String?
    var artistName: String?
    var catalogAnchor: Track?
    var libraryQueryOverride: String?
    var detailTrack: Track?
    var showLibrary = false
    var showDownloads = false
    var albumID: String?
    var albumName: String?
    var zones: [Zone] = []
    var onlineEndpoints: Set<String> = []
    var endpointSnapshot: [JSONValue] = []
    var playbackZones: [Zone] {
        let held = Set(endpointSnapshot.filter { $0["kind"].text == "group" }.flatMap { $0["group"]["zone_ids"].array.compactMap(\.string) })
        return zones.filter { !held.contains($0.id) }
    }
    let outputLabels = OutputLabels()
    var selectedZoneID = ""
    @ObservationIgnored private var preferredOutput: String?
    var signalEndpoint: JSONValue = .null
    var bufferingTrack:String?
    var signal = "尚无输出报告"
    var dspZoneRequest: String?
    var showConnection = false
    var loading = false
    var endpointID: String
    var zone: Zone? { zones.first { $0.id == selectedZoneID } }
    func selectPlaybackZone(_ id: String) {
        guard let zone = playbackZones.first(where: { $0.id == id }) else { return }
        preferredOutput = zone.endpoint_id
        if let key = playbackOutputPreferenceKey { UserDefaults.standard.set(zone.endpoint_id, forKey: key) }
        applyPlaybackSelection(zone)
    }
    private var playbackOutputPreferenceKey: String? {
        guard !bridgeOnly, let core = profile.core_id, let account = user["id"].string, !account.isEmpty else { return nil }
        return "playbackOutput.\(core).\(account)"
    }
    func loadPlaybackOutputPreference() {
        preferredOutput = playbackOutputPreferenceKey.flatMap { UserDefaults.standard.string(forKey: $0) }
    }
    func restorePlaybackSelection(in candidates: [Zone], fallback: Zone?) {
        let selected = preferredOutput.flatMap { endpoint in candidates.first { $0.endpoint_id == endpoint } }
            ?? candidates.first { $0.id == selectedZoneID }
            ?? fallback
        if let selected { applyPlaybackSelection(selected) }
        else { selectedZoneID = "" }
    }
    private func applyPlaybackSelection(_ zone: Zone) {
        guard zone.id != selectedZoneID else { return }
        selectedZoneID = zone.id
        let local = outputs.contains(zone.endpoint_id)
        presentationClock.reset()
        presentationClock.record(zone, local: local)
        if outputs.roaming {
            signalEndpoint = family.endpoints.first { $0["id"].string == zone.endpoint_id } ?? .null
            details.outputError = nil
            details.recovery = nil
            if !local { family.update(self, force: true) }
        }
        details.update(zone.track, store: self)
        updateSystemPlayback()
    }
    func playbackOutputLabel(_ zone: Zone) -> String {
        outputLabels.label(zone)
    }
    var listeningStatsRefreshID: String {
        "\(sessionID):\(uiActive):\(systemSleeping):\(coreReachable):" + zones.map { "\($0.id):\($0.playback_id ?? ""):\($0.status)" }.joined(separator: "|")
    }
    var localZone: Zone? { LocalPlaybackSelection.resolve(zones: zones, selectedID: selectedZoneID, enabledEndpoints: Set(outputs.enabled.map(\.id))) }
    var canPlay: Bool {
        guard connected, (coreReachable || outputs.roaming), let zone, !connecting, !systemSleeping else { return false }
        if let output = outputs.output(for: zone) { return output.enabled && output.available && output.online }
        return coreReachable && onlineEndpoints.contains(zone.endpoint_id)
    }
    @ObservationIgnored private let runtime: any RuntimeTransport
    @ObservationIgnored private var media: SystemPlayback?
    @ObservationIgnored private var polling: Task<Void, Never>?
    @ObservationIgnored private var searchTask: Task<Void, Never>?
    @ObservationIgnored private var ensuringZones: Set<String> = []
    private enum StartupPage { case library, collection }
    @ObservationIgnored private var pendingStartupPage: StartupPage?
    @ObservationIgnored private var epoch = UUID()
    @ObservationIgnored private var libraryRevision = UUID()

    public convenience init(bridgeOnly: Bool = false) {
        self.init(bridgeOnly: bridgeOnly, runtime: RuntimeClient())
        persistSessionState = !bridgeOnly
        if !bridgeOnly, !UserDefaults.standard.bool(forKey: Self.signedOutKey), let core = profile.core_id, !username.isEmpty {
            savedSession = SavedCoreSession.current.read(core: core, username: username)
        }
    }
    init(bridgeOnly: Bool = false, runtime: any RuntimeTransport, readSavedSession: (() async -> String?)? = nil) {
        self.runtime = runtime
        self.bridgeOnly = bridgeOnly
        endpointID = UserDefaults.standard.string(forKey: "endpointID") ?? "mac-\(bridgeOnly ? "bridge" : "client")-\(UUID().uuidString.lowercased())"
        UserDefaults.standard.set(endpointID, forKey: "endpointID")
        if let data = UserDefaults.standard.data(forKey: "coreProfile"), let saved = try? JSONDecoder().decode(CoreProfile.self, from: data) {
            profile = saved; lanAddress = saved.address; remoteAddress = bridgeOnly ? "" : (saved.alternatives.first { $0.from_core != true }?.address ?? "")
        }
        username = UserDefaults.standard.string(forKey: "username") ?? ""
        connectionTrust = ConnectionTrustPreferences(profile: profile)
        if !bridgeOnly {
            self.readSavedSession = readSavedSession
            awaitingSavedSession = readSavedSession != nil
        }
        if let child=runtime as? RuntimeClient {
            child.onCoreEvent = { [weak self] event in
                guard let self else { return }
                switch event["type"].text {
                case "portable_command": Task { await self.travel.receiveRemote(event["data"], store: self) }
                case "portable_loudness": Task { await self.travel.receiveLoudness(event["data"], store: self) }
                case "connection_ready":
                    self.scheduleCollectionRefresh()
                    self.travel.publishRemote(self, force: true)
                case "playlists": self.scheduleCollectionRefresh()
                case "autoplay": if event["data"]["zone_id"].string == self.selectedZoneID { self.radioRevision += 1 }
                default: break
                }
            }
            child.onTravelTerminal = { [weak self] id in
                Task { @MainActor [weak self] in
                    guard let self,self.connected,self.outputs.roaming,
                          self.zones.contains(where:{$0.playback_id==id}) else{return}
                    await self.perform {try await self.refreshZones()}
                }
            }
        }
    }
    func prepare() async {
        discovery.start(self)
        if !bridgeOnly && media == nil { media = SystemPlayback(store: self) }
        if awaitingSavedSession, let readSavedSession {
            guard !readingSavedSession else { return }
            let current = epoch
            readingSavedSession = true
            let session = await readSavedSession()
            readingSavedSession = false
            guard current == epoch, !Task.isCancelled else { return }
            savedSession = session
            awaitingSavedSession = false
            self.readSavedSession = nil
        }
        if savedSession != nil {
            await retrySavedConnection()
        } else {
            do { try await outputs.refresh(self) } catch { self.error = error.localizedDescription }
        }
    }
    func retrySavedConnection() async {
        guard savedSession != nil, !connected else { return }
        await connect(restore: true)
    }
    func connect(restore: Bool = false) async {
        guard !connecting, !systemSleeping else { return }
        if restore, savedSession == nil { return }
        connecting = true; error = nil; connectionIssue = nil; coreReachable = false; connected = false; zones = []; endpointSnapshot = []; tracks = []; signal = "尚无输出报告"; signalEndpoint = .null; selectedZoneID = ""; ensuringZones = []; polling?.cancel(); epoch = UUID()
        collection.reset(); discoveryFeed.reset(); downloads.reset(); details.reset(); presentationClock.reset(); artwork.reset(); outputs.reset(); travel.reset(); family.reset();bufferingTrack=nil; lanSelection = nil; streamBuffer = .null; pendingHome=nil; checkedSavedHome=false; albumID = nil; albumName = nil; artistID = nil; artistName = nil; catalogAnchor = nil; libraryQueryOverride = nil; detailTrack = nil; total = 0
        media?.update()
        let current = epoch
        defer { if epoch == current { connecting = false } }
        do {
            let primary = CoreConnectionAddress.normalize(lanAddress) ?? lanAddress.trimmingCharacters(in: .whitespacesAndNewlines)
            let remote = bridgeOnly ? "" : remoteAddress.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !primary.isEmpty else { throw RuntimeFailure(message: bridgeOnly ? "请填写同一局域网内的 Core 地址。" : "请填写 Core 的局域网或外部地址。") }
            if !remote.isEmpty, URL(string: remote)?.scheme?.lowercased() != "https" {
                throw RuntimeFailure(message: "外网备用地址必须使用 HTTPS。")
            }
            // A denied/unreachable LAN path may still fall back to the identity-bound
            // HTTPS address. The runtime probes both before sending credentials.
            if remote.isEmpty && (bridgeOnly || profile.alternatives.isEmpty) { try await LocalNetworkReadiness.check(primary) }
            else { try? await LocalNetworkReadiness.check(primary) }
            try Task.checkCancellation()
            let discovered = discovery.cores.first { $0.id == discovery.selectedID }?.addresses ?? []
            var alternatives = Set<String>()
            let addresses = ([remote] + discovered).filter { !$0.isEmpty && $0 != primary && alternatives.insert($0).inserted }
            var candidate = connectionTrust.applying(to: connectionBinding, primary: primary, alternatives: addresses)
            let saved = restore ? savedSession : nil
            let info = try await runtime.call("connect", .object([
                "session_token": saved.map(JSONValue.string) ?? .null, "persist_session": .bool(!bridgeOnly),
                "defer_output_refresh": .bool(true), "anonymous_metadata": .bool(anonymousMetadata), "streaming_policy": try .encoded(streamingPolicy), "profile": try .encoded(candidate), "username": .string(username), "password": .string(password),
                "totp": totp.isEmpty ? .null : .string(totp), "host_id": .string(endpointID),
                "device_name": .string(Host.current().localizedName ?? "Mac"),
                "name": .string(bridgeOnly ? "Mac 菜单栏网桥" : "Mac 客户端")
            ]))
            guard epoch == current else { return }
            candidate.core_id = info["core_id"].string
            if let updated = try? info["profile"].decode(CoreProfile.self) { candidate = updated }
            candidate.core_public_key = info["core_public_key"].string
            profile = candidate; connectionIdentity = nil; lanAddress = primary
            connectionAddress = info["address"].string ?? primary
            if persistSessionState, let core = profile.core_id, let token = info["session_token"].string {
                let storage = SavedCoreSession.current
                if storage.read(core: core, username: username) != token {
                    do {
                        try storage.save(token, core: core, username: username)
                    } catch {
                        notice = "登录成功，但本机无法保存会话。"
                    }
                }
            }
            if persistSessionState { UserDefaults.standard.removeObject(forKey: Self.signedOutKey) }
            restoreFailed = false; savedSession = nil
            user = info["user"]; connected = true; coreReachable = true; password = ""; totp = ""; showConnection = false
            loadPlaybackOutputPreference()
            UserDefaults.standard.set(try JSONEncoder().encode(profile), forKey: "coreProfile")
            UserDefaults.standard.set(username, forKey: "username")
            discovery.stop()
            pendingStartupPage = bridgeOnly ? nil : .library
            do {
                try await outputs.refresh(self)
                try await refreshZones()
                guard epoch == current, connected else { return }
                if localZone?.status == "paused", (localZone?.position ?? 0) > 0 { notice = "已找回上次播放位置，点击播放即可继续。" }
                try await loadPendingStartupPages()
                if !bridgeOnly { try await downloads.refresh(self) }
            } catch {
                guard epoch == current, connected else { return }
                await handle(error)
            }
            if epoch == current, connected { startPolling() }
        } catch {
            if epoch == current {
                connected = false; zones = []
                if let status = (error as? RuntimeFailure)?.status, [401, 403].contains(status) {
                    if restore {
                        if persistSessionState { UserDefaults.standard.set(true, forKey: Self.signedOutKey) }
                        if persistSessionState { SavedCoreSession.current.remove() }
                        savedSession = nil
                    }
                    self.error = restore ? "Core 会话已失效，请重新登录。" : "账号、密码或两步验证码不正确，请检查后重试。"
                } else if restore {
                    restoreFailed = true
                    self.error = "请检查网络与 Core 服务，或选择其他 Core。"
                } else if let status = (error as? RuntimeFailure)?.status, Self.temporaryConnectionFailure(status) {
                    self.error = "连接 Core 超时或中断，请重试。"
                } else { self.error = error.localizedDescription }
            }
        }
    }
    /// Only retry unfinished startup reads; keep a new session separate from stale completions.
    private func loadPendingStartupPages() async throws {
        let owner = epoch
        if pendingStartupPage == .library {
            try await loadLibrary()
            try Task.checkCancellation()
            guard owner == epoch, connected else { return }
            pendingStartupPage = .collection
        }
        if pendingStartupPage == .collection {
            try await collection.refresh(self)
            try Task.checkCancellation()
            guard owner == epoch, connected else { return }
            pendingStartupPage = nil
        }
    }
    func runtimeCall(_ operation: String, _ body: JSONValue = .null) async throws -> JSONValue { try await runtime.call(operation, body) }
    func updateSystemPlayback() { media?.update() }
    func retryOutput() async {
        await perform {
            try await self.outputs.refresh(self)
            if let output = self.outputs.output(for: self.zone), !output.available || !output.enabled { throw RuntimeFailure(message: "请接回或启用这一路设备，再恢复播放。") }
            _ = try await self.runtime.call("clear_error", .object(["id": .string(self.zone?.endpoint_id ?? "")]))
            self.details.outputError = nil; self.details.recovery = nil
            await self.transport("play")
        }
    }
    func api(_ path: String, method: String = "GET", body: JSONValue? = nil) async throws -> JSONValue {
        if path.hasPrefix("/zones/"), outputs.roaming {
            let route = path.prefix { $0 != "?" }
            let parts = route.dropFirst(7).split(separator: "/", maxSplits: 1)
            if let first=parts.first,let id=String(first).removingPercentEncoding,id.hasPrefix("travel-") {
                return try await travel.request(id,action:parts.count==2 ? String(parts[1]):"",method:method,body:body ?? .null,store:self)
            }
        }
        let requestPath = method == "GET" && path.split(separator: "?", maxSplits: 1).first == "/library/tracks" ? path + (path.contains("?") ? "&" : "?") + "view=compact" : path
        var payload: [String: JSONValue] = ["path": .string(requestPath), "method": .string(method)]
        if let body { payload["json"] = body }
        let value = try await runtime.call("request", .object(payload))
        if outputs.roaming, method != "GET", !path.hasSuffix("/visualization"),
           ["/zones", "/outputs", "/devices", "/device-groups", "/sync/starts"].contains(where: { path.hasPrefix($0) }) {
            family.invalidate()
            family.update(self)
        }
        if path.hasPrefix("/catalog/"), method != "GET" || path.contains("refresh=true") { artwork.reset(revalidate: true) }
        return value
    }
    func refreshZones() async throws {
        let current = epoch
        if outputs.roaming {
            if lanSelection == nil {
                let old=localZone, chosen=selectedZoneID
                let wasControllingFamily=zone.map{!outputs.contains($0.endpoint_id)} ?? false
                lanSelection=chosen
                family.accept(zones,endpointSnapshot,store:self)
                signalEndpoint = .null;signal = "本机流式播放"
                try await travel.enter(old,store:self)
                if wasControllingFamily,family.zones.contains(where:{$0.id==chosen}) {selectedZoneID=chosen}
            }
            let before=travel.generation
            let report = try await runtimeCall("status")
            guard before==travel.generation,current==epoch,connected,outputs.roaming else{return}
            streamBuffer = report["stream"]
            try await travel.refresh(self,report:report);family.update(self);return
        }
        if let previous = lanSelection {
            streamBuffer = .null
            let report=try await runtimeCall("status")
            guard current == epoch, connected, !outputs.roaming, lanSelection == previous else { return }
            checkedSavedHome = true
            pendingHome=travel.returning(report["departed_stream"], selectedOutput: localZone?.endpoint_id)
            await travel.withdrawRemote(self)
            guard current == epoch, connected, !outputs.roaming, lanSelection == previous else { return }
            if selectedZoneID.hasPrefix("travel-") || selectedZoneID.isEmpty {selectedZoneID=previous};lanSelection=nil;travel.reset();family.reset();bufferingTrack=nil;notice="已回到局域网，家庭音区已恢复。"
        }
        let controlRevisions = playbackCommands.revisions
        let revisions = Dictionary(uniqueKeysWithValues: zones.compactMap { zone in zone.queue_revision.map { (zone.id, JSONValue.number(Double($0))) } })
        zoneReadSequence += 1
        let readSequence = zoneReadSequence
        let received = try await api("/zones", body: .object(["queue_epoch": queueEpoch.map(JSONValue.string) ?? .null, "queue_revisions": .object(revisions)]))
        guard epoch == current, connected, !outputs.roaming, !Task.isCancelled, readSequence > appliedZoneRead else { return }
        var response = try received.decode(ZonePage.self).items
        for index in response.indices where response[index].queue_unchanged == true {
            if let old = zones.first(where: { $0.id == response[index].id && $0.queue_revision == response[index].queue_revision }) { response[index].queue = old.queue }
        }
        appliedZoneRead = readSequence
        queueEpoch = received["queue_epoch"].string
        zones = playbackCommands.reconcile(response, previous: zones, since: controlRevisions)
        if let zone { presentationClock.record(zone, local: outputs.items.contains { $0.id == zone.endpoint_id }) }
        media?.update()
        let endpoints = try await api("/endpoints")
        guard epoch == current, connected else { return }
        endpointSnapshot = endpoints["items"].array
        if let combined = endpointSnapshot.first(where: { $0["kind"].text == "group" && $0["group"]["zone_ids"].array.contains(where: { $0.text == selectedZoneID }) }), let combinedZone = zones.first(where: { $0.endpoint_id == combined["id"].text }) { selectedZoneID = combinedZone.id }
        let candidates = playbackZones
        let localID = localZone?.id
        let local = candidates.first { $0.id == localID }
        restorePlaybackSelection(in: candidates, fallback: local ?? candidates.first)
        details.update(zone?.track, store: self)
        details.prefetch(zone, store: self)
        onlineEndpoints = Set(endpoints["items"].array.filter { $0["online"].boolean }.compactMap { $0["id"].string })
        await outputLabels.refresh(self)
        guard epoch == current, connected else { return }
        for output in outputs.enabled where !zones.contains(where: { $0.endpoint_id == output.id }) && !ensuringZones.contains(output.id) {
            guard endpoints["items"].array.contains(where: { $0["id"].string == output.id && $0["online"].boolean }) else { continue }
            ensuringZones.insert(output.id)
            do {
                let created = try await api("/zones", method: "POST", body: .object(["name": .string(output.name), "endpoint_id": .string(output.id)])).decode(Zone.self)
                guard epoch == current, connected else { return }
                if !zones.contains(where: { $0.id == created.id }) { zones.append(created) }
                restorePlaybackSelection(in: playbackZones, fallback: created)
            } catch { ensuringZones.remove(output.id); throw error }
            ensuringZones.remove(output.id)
        }
        if !checkedSavedHome {
            checkedSavedHome = true
            pendingHome = travel.savedReturn(self)
        }
        if let handoff = pendingHome, let target = zones.first(where: { $0.endpoint_id == handoff.zone.endpoint_id }),
           onlineEndpoints.contains(target.endpoint_id) {
            pendingHome = nil
            let ids = handoff.zone.queue.map(\.id)
            let sameQueue = target.queue.map(\.id) == ids
            let homeQueueUnchanged = target.queue_revision != nil && target.queue_revision == handoff.zone.queue_revision
            if target.status != "playing" && (target.queue.isEmpty || sameQueue || homeQueueUnchanged) {
                do {
                    try await requireUngrouped(target.id)
                    let path = "/zones/" + target.id.pathComponentEncoded
                    if !sameQueue || (handoff.pausedPosition != nil && target.queue_index != handoff.index) {
                        _ = try await api(path + "/queue", method: "PUT", body: .object([
                            "track_ids": .array(ids.map(JSONValue.string)), "queue_index": .number(Double(handoff.index))]))
                    }
                    if let position = handoff.pausedPosition {
                        if target.status != "paused" { _ = try await api(path + "/pause", method: "POST", body: .object([:])) }
                        _ = try await api(path + "/seek", method: "POST", body: .object(["position": .number(position)]))
                    } else {
                        _ = try await api(path + "/play", method: "POST", body: .object(["queue_index": .number(Double(handoff.index))]))
                    }
                    if selectedZoneID.isEmpty || outputs.contains(zone?.endpoint_id ?? "") { selectedZoneID = target.id }
                } catch {
                    if current == epoch, connected, !outputs.roaming, pendingHome == nil { pendingHome = handoff }
                    throw error
                }
            } else { notice = "外出队列和进度已保存在这台 Mac，家庭音区已有播放或其他队列。" }
            travel.finishReturn(self)
        }
        for output in outputs.enabled {
            if let target=zones.first(where:{$0.endpoint_id==output.id}) {
                try await effects.reconcile(output,zone:target,store:self)
            }
        }
        signalEndpoint = endpoints["items"].array.first { $0["id"].string == zone?.endpoint_id } ?? .null
        if let item = endpoints["items"].array.first(where: { $0["id"].string == zone?.endpoint_id }) {
            let playback = item["playback"]
            if let expected = zone?.playback_id, playback["core_playback_id"].string != expected {
                signal = "正在准备当前曲目的输出格式"; return
            }
            let value = playback["signal_path"]
            if case .object = value {
                struct PathReport: Decodable {
                    let sample_rate: Int; let bit_depth: Int; let channels: Int; let exclusive: Bool
                    let processing: [String]
                }
                let path = try value.decode(PathReport.self)
                let details = path.processing.map { stage -> String in
                    if stage.contains("driver format conversion") { return "HAL 输出路径可能包含驱动格式转换" }
                    if stage.contains("Position:") { return "进度依据 HAL 时间戳与设备报告延迟；硬件尚未验收" }
                    if stage.contains("shared/system mixing") { return "系统共享混音" }
                    if stage.contains("Software volume") { return "软件音量处理" }
                    return stage
                }
                signal = "\(path.sample_rate) Hz · \(path.bit_depth) bit · \(path.channels) 声道\n\(path.exclusive ? "已取得设备独占占用" : "系统共享输出")\n" + details.joined(separator: "\n")
            } else { signal = item["playback"]["error"].string ?? "等待端点提供实际输出格式" }
        }
    }
    func pollOnce(refreshOutputs: Bool = true) async throws {
        let owner = epoch
        if refreshOutputs && outputRefreshTask == nil {
            outputRefreshTask = Task { [weak self] in
                guard let self else { return }
                defer { if self.epoch == owner { self.outputRefreshTask = nil } }
                do { try await self.outputs.refresh(self) }
                catch is CancellationError {} catch {
                    guard self.epoch == owner else { return }
                    self.outputs.error = error.localizedDescription
                    if (error as? RuntimeFailure)?.status == 401 { await self.handle(error) }
                }
            }
        }
        try await refreshZones()
        if !outputs.roaming {try await details.refreshStatus(self)}
        try await loadPendingStartupPages()
        if !bridgeOnly { try await downloads.refreshDuringPlaybackPoll(self) }
        try Task.checkCancellation()
        guard owner == epoch, connected else { return }
        let wasUnavailable = !coreReachable
        coreReachable = true; connectionIssue = nil
        if wasUnavailable { notice = "已重新连接 Core，保留当前队列与播放位置。" }
    }
    func searchChanged() {
        albumID = nil; albumName = nil; artistID = nil; artistName = nil; catalogAnchor = nil; libraryQueryOverride = nil
        searchTask?.cancel(); page = 0
        guard query.isEmpty else { return }
        searchTask = Task { [weak self] in
            do { try await Task.sleep(for: .milliseconds(300)); try Task.checkCancellation(); try await self?.loadLibrary() }
            catch is CancellationError {} catch { self?.error = error.localizedDescription }
        }
    }
    func loadLibrary() async throws {
        guard connected else { return }
        let current = epoch, revision = UUID(); libraryRevision = revision
        loading = true
        defer { if libraryRevision == revision { loading = false } }
        var url = URLComponents(); url.queryItems = [URLQueryItem(name: "q", value: libraryQueryOverride ?? query), URLQueryItem(name: "limit", value: "100"), URLQueryItem(name: "offset", value: String(page * 100))]
        if let artistID { url.queryItems?.append(URLQueryItem(name: "artist_id", value: artistID)) }
        if let albumID { url.queryItems?.append(URLQueryItem(name: "album_id", value: albumID)) }
        let path = "/library/tracks?\(url.percentEncodedQuery ?? "")"
        if let cached = pageSnapshots.value(path, owner: current), let page = try? cached.decode(TrackPage.self) { tracks = page.items; total = page.total }
        let result = try await api(path)
        let response = try result.decode(TrackPage.self)
        guard epoch == current, libraryRevision == revision else { return }
        pageSnapshots.save(result, path: path, owner: current)
        tracks = response.items; total = response.total; if outputs.roaming {travel.remember(tracks)}
    }
    func openAlbum(_ track: Track) async {
        await perform {
            let refs = try await catalogReferences(track, album: true, artistIndex: 0)
            guard refs.count == 1 else { throw RuntimeFailure(message: "无法唯一确认专辑身份。") }
            await openCatalogReference(refs[0], track: track, album: true)
        }
    }
    func closeAlbum() async { albumID = nil; albumName = nil; artistID = nil; artistName = nil; catalogAnchor = nil; libraryQueryOverride = nil; page = 0; await perform { try await loadLibrary() } }
    func play(_ track: Track, exactVersion: Bool = false) async {
        if !exactVersion {await playList(tracks.contains(where: {$0.id==track.id}) ? tracks : [track], starting: track.id);return}
        await perform {
            guard let zone = self.zone else { throw RuntimeFailure(message: "本机输出尚未注册，请稍候。") }
            try await self.requireUngrouped(zone.id)
            _ = try await self.runtime.call("clear_error", .object(["id": .string(zone.endpoint_id)])); self.details.outputError = nil; self.details.recovery = nil
            _ = try await self.api("/zones/\(zone.id.pathComponentEncoded)/play", method: "POST", body: .object(["track_id": .string(track.id), "exact_version": .bool(exactVersion)]))
            try await self.refreshZones()
        }
    }
    func transport(_ action: String, body: [String: JSONValue] = [:], target:Zone?=nil) async {
        if !outputs.roaming, let zone = target ?? self.zone {
            await playbackCommands.send(action, body: body, zone: zone, store: self)
            return
        }
        await perform {
            guard let zone = target ?? self.zone else { return }
            try await self.requireUngrouped(zone.id)
            _ = try await self.api("/zones/\(zone.id.pathComponentEncoded)/\(action)", method: "POST", body: .object(body))
            if !self.outputs.roaming || !zone.id.hasPrefix("travel-") {try await self.refreshZones()}
        }
    }
    @ObservationIgnored private var volumeSampleRevision = 0
    var hardwareVolumeWriting = false
    func readLocalVolume() async throws {
        guard !hardwareVolumeWriting, let endpoint = zone?.endpoint_id else { return }
        let owner = volumeTargetID, revision = volumeSampleRevision
        let value = try await runtime.call("device_volume_state", .object(["id": .string(endpoint)]))
        try Task.checkCancellation()
        guard value["local"].boolean, volumeTargetID == owner, volumeSampleRevision == revision, !hardwareVolumeWriting else { return }
        acceptDeviceVolume(value["state"], endpoint: endpoint)
    }
    func setMuted(_ muted: Bool) async {
        await perform {
            guard let zone = self.zone else { return }
            let owner = volumeTargetID
            if volumeMode == .software {
                try await requireUngrouped(zone.id)
                guard volumeTargetID == owner else { return }
                _ = try await api("/zones/\(zone.id.pathComponentEncoded)", method: "PATCH", body: .object(["muted": .bool(muted)]))
                if volumeTargetID == owner { try await refreshZones() }
            } else if canMuteLocalDevice, !hardwareVolumeWriting, let output = outputs.output(for: zone) {
                hardwareVolumeWriting = true; volumeSampleRevision += 1
                defer { hardwareVolumeWriting = false }
                let actual = try await runtime.call("set_device_mute", .object(["id": .string(output.id), "muted": .bool(muted)]))
                if volumeTargetID == owner { acceptDeviceVolume(actual, endpoint: output.id) }
            }
        }
    }
    func setVolume(_ value: Double) async {
        await perform {
            guard value.isFinite, (0...1).contains(value), let zone = self.zone else { return }
            let owner = volumeTargetID
            if volumeMode == .device, let output = outputs.output(for: zone) {
                guard !hardwareVolumeWriting else { return }
                hardwareVolumeWriting = true; volumeSampleRevision += 1
                defer { hardwareVolumeWriting = false }
                let raw = try await runtime.call("set_device_volume", .object(["id": .string(output.id), "value": .number(value)]))
                guard volumeTargetID == owner else { return }
                if let item = raw["items"].array.first(where: { $0["id"].string == output.id }) { acceptDeviceVolume(item["device_volume"], endpoint: output.id) }
            } else if volumeMode == .device {
                let actual = try await api("/outputs/\(zone.endpoint_id.pathComponentEncoded)/volume", method: "PUT", body: .object(["value": .number(value)]))
                guard volumeTargetID == owner else { return }
                acceptDeviceVolume(actual, endpoint: zone.endpoint_id)
                try await refreshZones()
            } else if volumeMode == .software {
                try await requireUngrouped(zone.id)
                guard volumeTargetID == owner else { return }
                _ = try await api("/zones/\(zone.id.pathComponentEncoded)", method: "PATCH", body: softwareVolumePatch(value))
                if volumeTargetID == owner { try await refreshZones() }
            } else { throw RuntimeFailure(message: "此设备未启用音量调节。") }
        }
    }
    /// Single-zone actions must never silently cancel a group.
    func requireUngrouped(_ zoneID: String) async throws {
        if outputs.roaming && zoneID.hasPrefix("travel-") {return}
        if endpointSnapshot.contains(where: { $0["kind"].text == "group" && $0["group"]["zone_ids"].array.contains(where: { $0.text == zoneID }) }) { throw RuntimeFailure(message: "此输出已加入设备组合，请选择组合设备，或先拆开组合。") }
        var offset = 0
        while true {
            let groups = try await api("/sync/starts?active=true&limit=100&offset=\(offset)")["items"].array
            if groups.contains(where: { $0["members"].array.contains(where: { $0["zone_id"].string == zoneID }) }) {
                throw RuntimeFailure(message: "此音区正由整组控制，请到声音设备的分组控制中操作整组。")
            }
            if groups.count < 100 { return }
            offset += 100
            if offset > 100_000 { throw RuntimeFailure(message: "分组状态过多，暂不能安全控制此音区。") }
        }
    }
    func perform(_ action: () async throws -> Void) async {
        do { try await action(); error = nil } catch { await handle(error) }
    }
    private static func temporaryConnectionFailure(_ status: Int) -> Bool { [408, 429, 502, 503, 504].contains(status) }
    func handle(_ failure: Error) async {
        if (failure as? RuntimeFailure)?.status == 401 {
            await disconnect()
            showConnection = true
            error = "Core 会话已失效，请重新登录。"
        } else if let status = (failure as? RuntimeFailure)?.status, Self.temporaryConnectionFailure(status) {
            coreReachable = false; onlineEndpoints = []
            connectionIssue = "与 Core 的连接已中断，正在自动重试。曲库与播放位置已保留。"
        } else if !(failure is CancellationError) { error = failure.localizedDescription }
    }
    func disconnect() async {
        if persistSessionState { UserDefaults.standard.set(true, forKey: Self.signedOutKey) }
        if persistSessionState { SavedCoreSession.current.remove() }
        await travel.withdrawRemote(self)
        playbackCommands.reset()
        epoch = UUID(); outputRefreshTask?.cancel(); outputRefreshTask = nil; connecting = false; pendingStartupPage = nil; polling?.cancel(); searchTask?.cancel(); error = nil; notice = nil; user = .null
        savedSession = nil; awaitingSavedSession = false; readSavedSession = nil; restoreFailed = false; preferredOutput = nil
        connected = false; coreReachable = false; connectionIssue = nil; zones = []; queueEpoch = nil; tracks = []; selectedZoneID = ""; signal = "尚无输出报告"; signalEndpoint = .null
        collection.reset(); discoveryFeed.reset(); downloads.reset(); details.reset(); presentationClock.reset(); artwork.reset(); outputs.reset(); travel.reset(); family.reset();bufferingTrack=nil; lanSelection = nil; streamBuffer = .null; pendingHome=nil; checkedSavedHome=false; albumID = nil; albumName = nil; artistID = nil; artistName = nil; catalogAnchor = nil; libraryQueryOverride = nil; detailTrack = nil; total = 0
        media?.update()
        _ = try? await runtime.call("disconnect")
        if !bridgeOnly {try? await outputs.refresh(self)}
    }
    private func scheduleCollectionRefresh() {
        guard connected, uiActive, !systemSleeping, !bridgeOnly else { return }
        collectionRefreshTask?.cancel()
        collectionRefreshTask = Task { [weak self] in
            do {
                try await Task.sleep(for: .milliseconds(150))
                guard let self else { return }
                try await self.collection.synchronize(self)
            } catch { }
        }
    }
    func stopPolling() {
        stopPlaybackPolling()
        searchTask?.cancel()
        collectionRefreshTask?.cancel()
        collectionRefreshTask = nil
    }
    private func stopPlaybackPolling() {
        polling?.cancel()
        polling = nil
        presentationPolling?.cancel()
        presentationPolling = nil
    }
    func setUIActive(_ active: Bool) {
        guard uiActive != active else { return }
        uiActive = active
        if connected, !connecting { Task { _ = try? await runtimeCall("ui_visibility", .object(["visible": .bool(active && !systemSleeping && !bridgeOnly)])) } }
        if connected, !connecting, !systemSleeping { startPolling(immediate: active) }
    }
    func startPolling(immediate: Bool = false) {
        stopPlaybackPolling()
        guard connected, !systemSleeping else { return }
        if !bridgeOnly {
            presentationPolling = Task { [weak self] in
                while !Task.isCancelled {
                    guard let self, self.connected, !self.systemSleeping else { return }
                    do {
                        guard let zone = self.zone, zone.status == "playing", self.outputs.items.contains(where: { $0.id == zone.endpoint_id }) else {
                            try await Task.sleep(for: .milliseconds(500)); continue
                        }
                        let began = ProcessInfo.processInfo.systemUptime, owner = self.epoch
                        let report = try await self.runtimeCall("playback_observation", .object(["endpoint_id": .string(zone.endpoint_id), "audio": .bool(false)]))
                        try Task.checkCancellation()
                        guard owner == self.epoch, self.zone?.id == zone.id, self.zone?.playback_id == zone.playback_id else { continue }
                        if report["local"].boolean { self.presentationClock.accept(report, zone: zone, roundTrip: ProcessInfo.processInfo.systemUptime - began) }
                        try await Task.sleep(for: .milliseconds(self.presentationVisible ? 100 : 400))
                    } catch is CancellationError { return }
                    catch { do { try await Task.sleep(for: .seconds(1)) } catch { return } }
                }
            }
        }
        polling = Task { [weak self] in
            let owner = self?.epoch
            var ticks = 0, failures = 0
            if immediate {
                do { try await self?.pollOnce(refreshOutputs: false) }
                catch is CancellationError { return } catch { guard !Task.isCancelled, owner == self?.epoch else { return }; failures = 1; await self?.handle(error) }
            }
            while !Task.isCancelled {
                do {
                    let normal = self?.presentationVisible == false ? 5.0 : 1.0
                    let interval = max(normal, failures > 0 ? min(30, pow(2.0, Double(failures))) : 0)
                    try await Task.sleep(for: .seconds(interval)); try Task.checkCancellation()
                    guard let self else { return }
                    ticks += 1
                    try await self.pollOnce(refreshOutputs: ticks % 4 == 0 || !self.coreReachable)
                    failures = 0
                } catch is CancellationError { break } catch { guard !Task.isCancelled, owner == self?.epoch else { return }; failures = min(5, failures + 1); await self?.handle(error) }
            }
        }
    }
    public func shutdown() async { travel.checkpoint(self); epoch = UUID(); outputRefreshTask?.cancel(); outputRefreshTask = nil; connected = false; systemSleeping = false; discovery.stop(); stopPolling(); media = nil; details.reset(); presentationClock.reset(); artwork.reset(); collection.reset(); discoveryFeed.reset(); downloads.reset(); await runtime.shutdown() }
}

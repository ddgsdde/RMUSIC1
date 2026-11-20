import SwiftUI

struct GroupPlanView: View {
  let store: MusicStore
  @Binding var plan: GroupPlan
  @StoredState private var query = ""
  @StoredState private var results = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var offset = 0
  @StoredState private var submittedQuery = ""
  @StoredState private var searchID = UUID()
  var body: some View {
    DisclosureGroup("歌曲队列与同步策略") {
      HStack {
        TextField("从 Core 搜索 PCM 歌曲", text: $query)
        Button("搜索") {
          submittedQuery = query
          searchID = UUID()
        }.disabled(query.isEmpty)
      }
      ResourceNotice(resource: results)
      WorkflowNotice(action: action)
      ForEach(Array(results.value["items"].array.enumerated()), id: \.offset) { _, item in
        if let track = try? item.decode(Track.self) {
          HStack {
            HStack { Text(track.title); CatalogTextLink(track: track, store: store) }.lineLimit(1)
            Spacer()
            Button("加入整组队列") { choose(track) }.disabled(
              action.busy || plan.queue.count >= 10000
                || ["dsf", "dff"].contains(track.format.lowercased()))
          }
        }
      }
      if plan.queue.isEmpty { Text("使用当前歌曲").font(.caption).foregroundStyle(.secondary) }
      ForEach(Array(plan.queue.dropFirst(offset).prefix(20).enumerated()), id: \.offset) {
        local, track in
        let index = offset + local
        HStack {
          Text("\(index + 1). \(track.title)").lineLimit(1)
          Spacer()
          Button("上移", systemImage: "arrow.up") { plan.queue.swapAt(index, index - 1) }.labelStyle(
            .iconOnly
          ).disabled(index == 0)
          Button("下移", systemImage: "arrow.down") { plan.queue.swapAt(index, index + 1) }
            .labelStyle(.iconOnly).disabled(index + 1 == plan.queue.count)
          Button("移除", systemImage: "minus.circle") {
            plan.queue.remove(at: index)
            offset = min(offset, max(0, (plan.queue.count - 1) / 20 * 20))
          }.labelStyle(.iconOnly)
        }
      }
      OffsetPageControls(offset: $offset, total: plan.queue.count, limit: 20)
      Toggle("监测已知同步误差", isOn: $plan.monitor)
      if plan.monitor {
        HStack {
          Stepper(
            "误差上限 \(plan.errorMicroseconds) µs", value: $plan.errorMicroseconds, in: 100...50000,
            step: 100)
          Stepper(
            "预热 \(plan.warmupMilliseconds) ms", value: $plan.warmupMilliseconds, in: 1000...10000,
            step: 500)
        }
      }
      Toggle("实验 PCM 变速", isOn: $plan.rateEnabled).onChange(of: plan.rateEnabled) {
        if !plan.rateEnabled { plan.automaticRate = false }
      }
      if plan.rateEnabled {
        Toggle("实验自动校正", isOn: $plan.automaticRate)
        Text("会处理 PCM 样本，限幅 ±500 ppm。仅对声明支持的输出开放，不保证严格同步。").font(.caption).foregroundStyle(
          .secondary)
      }
    }.task(id: "\(store.sessionID):\(searchID)") {
      guard !submittedQuery.isEmpty else { return }
      await results.load(
        apiQuery("/library/tracks", ["q": submittedQuery, "limit": "20"]), store: store)
    }.onDisappear { action.cancel() }
  }
  private func choose(_ track: Track) {
    if GroupPlan.pcm(track) {
      plan.queue.append(track)
      return
    }
    action.send(
      "/library/tracks/\(track.id.pathComponentEncoded)/audio-info", method: "GET", store: store
    ) { value in
      do {
        var fields = try JSONValue.encoded(track).object
        for key in ["format", "sample_rate", "bit_depth", "channels", "bitrate"] {
          if value.object[key] != nil { fields[key] = value[key] }
        }
        let resolved = try JSONValue.object(fields).decode(Track.self)
        guard GroupPlan.pcm(resolved) else { throw RuntimeFailure(message: "尚未确认实际 PCM 格式，未加入队列。") }
        plan.queue.append(resolved)
        action.notice = "已加入整组队列"
      } catch { action.error = error.localizedDescription }
    }
  }
}

struct GroupRateView: View {
  let store: MusicStore
  let job: JSONValue
  let blocked: Bool
  let send: (String, [String: JSONValue]) -> Void
  @StoredState private var ppm: [String: Double] = [:]
  var body: some View {
    DisclosureGroup("实验变速状态") {
      if job["rate_adjustment"]["automatic"].boolean {
        StructuredResult(value: job["rate_adjustment"]["controller"])
      }
      ForEach(Array(job["members"].array.enumerated()), id: \.offset) { _, member in
        HStack {
          Text(
            store.zones.first { $0.id == member["zone_id"].text }?.name ?? member["zone_id"].text)
          Text("请求 \(member["requested_rate_ppm"].number.formatted()) ppm").font(.caption)
          if !job["rate_adjustment"]["automatic"].boolean {
            TextField(
              "目标 ppm",
              value: Binding(
                get: { ppm[member["zone_id"].text] ?? member["requested_rate_ppm"].number },
                set: { ppm[member["zone_id"].text] = $0 }), format: .number
            ).frame(width: 100)
            Button("应用") {
              send(
                "/sync/starts/\(job["id"].text.pathComponentEncoded)/rate",
                [
                  "zone_id": member["zone_id"], "playback_id": member["playback_id"],
                  "ppm": .number(
                    ppm[member["zone_id"].text] ?? member["requested_rate_ppm"].number),
                ])
            }.disabled(
              blocked || job["phase"].text != "playing"
                || !(ppm[member["zone_id"].text] ?? 0).isFinite
                || abs(ppm[member["zone_id"].text] ?? 0) > 500)
          }
        }
      }
    }
  }
}

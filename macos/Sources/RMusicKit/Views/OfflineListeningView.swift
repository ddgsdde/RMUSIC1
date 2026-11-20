import SwiftUI

/// Cached originals for the last account on this Mac. Opening this sheet never logs in to a Core.
struct OfflineListeningView: View {
  let store: MusicStore
  @Environment(\.dismiss) private var dismiss
  @StoredState private var username = ""
  @StoredState private var tracks: [CachedTrack] = []
  @StoredState private var outputs: [NativeOutput] = []
  @StoredState private var selectedOutput = ""
  @StoredState private var total = 0
  @StoredState private var current: CachedTrack?
  @StoredState private var instance: String?
  @StoredState private var report: JSONValue = .null
  @StoredState private var seek: Double?
  @StoredState private var busy = false
  @StoredState private var message: String?

  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      HStack {
        VStack(alignment: .leading) {
          Text("离线音乐").font(.title.bold())
          Text(username.isEmpty ? "这台 Mac 上的完整缓存" : "\(username) · \(total) 首已缓存").foregroundStyle(
            .secondary)
        }
        Spacer()
        Button("完成") { dismiss() }.keyboardShortcut(.cancelAction)
      }
      if !tracks.isEmpty {
        Picker("本机输出", selection: $selectedOutput) {
          ForEach(outputs) { output in Text(output.name).tag(output.id) }
        }.disabled(instance != nil)
        List {
          ForEach(tracks) { song in
            Button {
              Task { await play(song) }
            } label: {
              HStack {
                Image(systemName: current?.id == song.id ? "waveform" : "music.note")
                VStack(alignment: .leading) {
                  Text(song.title ?? song.track_id)
                  Text(song.artist ?? "").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Text(song.format.uppercased()).font(.caption)
              }.contentShape(Rectangle())
            }.buttonStyle(.plain).disabled(busy || selectedOutput.isEmpty)
          }
          if tracks.count < total {
            Button("加载更多") { Task { await load(at: tracks.count) } }.disabled(busy)
          }
        }
        if outputs.isEmpty { Text("在登录页的「本机声音设备」中启用输出后即可播放。").foregroundStyle(.secondary) }
      } else if !busy {
        ContentUnavailableView(
          "暂无离线歌曲", systemImage: "arrow.down.circle", description: Text("登录 Core 后缓存完整歌曲，即可从这里收听。"))
      }
      if let current, instance != nil {
        Divider()
        HStack {
          Text(current.title ?? current.track_id).font(.headline)
          Spacer()
          Button(report["status"].text == "paused" ? "继续" : "暂停") {
            Task { await control(report["status"].text == "paused" ? "resume" : "pause") }
          }
          Button("停止") { Task { await control("stop") } }
        }.disabled(busy)
        if report["duration"].number > 0 {
          let limit = report["duration"].number
          Slider(
            value: Binding(
              get: { min(limit, seek ?? report["position"].number) }, set: { seek = $0 }),
            in: 0...limit,
            onEditingChanged: { editing in
              if !editing, let target = seek {
                Task {
                  await control("seek", position: target)
                  seek = nil
                }
              }
            }
          ).accessibilityLabel("离线播放进度")
        }
      }
      if let message { Text(message).foregroundStyle(.secondary) }
    }.padding(24).frame(minWidth: 500, minHeight: 440)
      .task { await load(at: 0) }
      .task(id: instance) {
        guard let expected = instance else { return }
        while !Task.isCancelled, instance == expected {
          do {
            let state = try await store.runtimeCall("offline_status")
            guard !Task.isCancelled, instance == expected else { return }
            if state["core_playback_id"].text == expected {
              report = state
              if state["type"].text == "error" {
                message = state["error"].text
                instance = nil
                return
              }
              if state["status"].text == "ended" {
                instance = nil
                if let i = tracks.firstIndex(where: { $0.id == current?.id }) {
                  if i + 1 == tracks.count, tracks.count < total { await load(at: tracks.count) }
                  if tracks.indices.contains(i + 1) { await play(tracks[i + 1]) }
                }
                return
              }
            }
            try await Task.sleep(for: .milliseconds(500))
          } catch is CancellationError { return } catch {
            message = error.localizedDescription
            return
          }
        }
      }
      .onDisappear { Task { _ = try? await store.runtimeCall("offline_close") } }
  }
  private func load(at offset: Int) async {
    guard !busy else { return }
    busy = true
    defer { busy = false }
    do {
      let result = try await store.runtimeCall(
        "offline_list", .object(["offset": .number(Double(offset))]))
      guard !Task.isCancelled, case .object = result else { return }
      let page = try result["page"]["items"].decode([CachedTrack].self)
      if offset == 0 { tracks = page } else { tracks += page }
      total = result["page"]["total"].integer
      username = result["username"].text
      outputs = try result["outputs"].decode(OutputInventory.self).items.filter {
        $0.enabled && $0.available
      }
      if selectedOutput.isEmpty {
        selectedOutput =
          (outputs.first(where: { $0.followsSystemDefault }) ?? outputs.first)?.id ?? ""
      }
    } catch { message = error.localizedDescription }
  }
  private func play(_ song: CachedTrack) async {
    guard !busy, let output = outputs.first(where: { $0.id == selectedOutput }) else { return }
    busy = true
    defer { busy = false }
    let id = UUID().uuidString
    let effects = store.effects.zone(for: output, base: store.travel.empty(output))
    var command: [String: JSONValue] = [
      "type": .string("play"), "output": .string(output.id), "playback_id": .string(id),
      "track": .object(["id": .string(song.track_id)]),
    ]
    command["dsp"] = effects.dsp
    command["transitions"] = effects.transitions
    do {
      _ = try await store.runtimeCall("offline_control", .object(command))
      report = .null
      message = nil
      current = song
      instance = id
    } catch { message = error.localizedDescription }
  }
  private func control(_ type: String, position: Double? = nil) async {
    guard let id = instance, !busy else { return }
    busy = true
    defer { busy = false }
    var command: [String: JSONValue] = ["type": .string(type), "playback_id": .string(id)]
    if let position { command["position"] = .number(position) }
    do {
      report = try await store.runtimeCall("offline_control", .object(command))
      if type == "stop" { instance = nil }
    } catch { message = error.localizedDescription }
  }
}

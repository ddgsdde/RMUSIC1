import SwiftUI

/// Groups are persistent playback devices. Content is selected from the normal music pages.
struct GroupPlaybackView: View {
  let store: MusicStore
  @StoredState private var groups = ResourceStore()
  @StoredState private var capabilities = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var refresh = UUID()
  @StoredState private var creating = false
  @StoredState private var dissolving: JSONValue?
  @StoredState private var rooms: Set<String> = []
  @StoredState private var source = ""
  @StoredState private var name = "全屋"
  @StoredState private var pending: JSONValue?
  @StoredState private var editingGroup: JSONValue?
  private var supported: Bool { capabilities.value["capabilities"]["device_groups_v1"].boolean }
  private var held: Set<String> { Set(groups.value["items"].array.flatMap { $0["zone_ids"].array.compactMap(\.string) }) }
  private var available: [Zone] {
    store.zones.filter { zone in
      !zone.id.hasPrefix("travel-") && (!held.contains(zone.id) || editingGroup?["zone_ids"].array.contains(.string(zone.id)) == true) && store.endpointSnapshot.contains { endpoint in
        endpoint["id"].text == zone.endpoint_id && GroupRequest.eligible(endpoint)
          && ["coreaudio", "wasapi", "asio", "alsa"].contains(endpoint["kind"].text)
          && endpoint["capabilities"]["platform"].text != "android"
      }
    }
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 24) {
      FeatureHeader(title: "设备组合") { refresh = UUID() }
      Text("把多个房间的输出组合为一个设备，共享同一条播放队列。").font(.callout).foregroundStyle(.secondary)
      ResourceNotice(resource: groups)
      ResourceNotice(resource: capabilities)
      WorkflowNotice(action: action)
      if supported {
        Button("组合设备", systemImage: "plus") { open() }.buttonStyle(.borderedProminent)
        if groups.value["items"].array.isEmpty {
          FeatureCard {
            Image(systemName: "hifispeaker.and.homepod.fill").font(.system(size: 34, weight: .light)).foregroundStyle(MusicStyle.accent)
            Text("让几个房间，一起播放").font(.title3.weight(.semibold))
            Text("组合后，在播放设备列表中选择它即可聆听。拆开组合时，各设备原来的队列都会回来。").font(.callout).foregroundStyle(.secondary)
          }
        }
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 300), spacing: 18)], spacing: 18) {
          ForEach(Array(groups.value["items"].array.enumerated()), id: \.offset) { _, group in
            card(group)
          }
        }
      } else if !capabilities.loading {
        Text("当前 Core 尚未提供设备组合，请更新 Core 后使用。").font(.callout).foregroundStyle(.secondary)
      }
      Text("同步成员支持 Mac、Windows 原生客户端和 Linux 网桥。网页与安卓可以遥控组合，不作为同步输出。").font(.caption).foregroundStyle(.secondary)
    }
    .task(id: "\(store.sessionID):\(refresh)") {
      await capabilities.load("/status", store: store)
      if supported { await groups.load("/device-groups", store: store) }
    }
    .sheet(isPresented: $creating) { editor.padding(28).frame(minWidth: 440, idealWidth: 500, minHeight: 360).interactiveDismissDisabled(action.busy) }
    .sheet(isPresented: Binding(get: { dissolving != nil }, set: { if !$0 { dissolving = nil } })) {
      VStack(alignment: .leading, spacing: 22) {
        Text("拆开 \(dissolving?["name"].text ?? "组合设备")").font(.title2.weight(.semibold))
        Text("组合播放将停止。各设备会恢复组合前的队列、顺序和位置；原来在播放的设备会保持暂停。").foregroundStyle(.secondary)
        WorkflowNotice(action: action)
        HStack { Spacer(); Button("保留组合") { dissolving = nil }.disabled(action.busy); Button("拆开并恢复原队列", role: .destructive) { dissolve() }.buttonStyle(.borderedProminent).disabled(action.busy) }
      }.padding(28).frame(width: 480).interactiveDismissDisabled(action.busy)
    }
    .onChange(of: rooms) { _, value in if !value.contains(source) { source = value.sorted().first ?? "" } }
    .onChange(of: action.status) { _, status in if let status, (400..<500).contains(status) { pending = nil } }
    .onDisappear { action.cancel() }
  }
  private func card(_ group: JSONValue) -> some View {
    let zone = store.zones.first { $0.id == group["zone_id"].text }
    let online = store.onlineEndpoints.contains(group["endpoint_id"].text)
    return FeatureCard {
      HStack(spacing: 14) {
        Image(systemName: "hifispeaker.and.homepod.fill").font(.system(size: 25, weight: .light)).foregroundStyle(MusicStyle.accent)
        VStack(alignment: .leading, spacing: 5) {
          Text(group["name"].text).font(.headline)
          Text("\(group["zone_ids"].array.count) 个输出 · \(online ? zone?.status == "playing" ? "同步播放中" : "在线" : "部分输出离线")").font(.caption).foregroundStyle(.secondary)
        }
        Spacer()
        if store.selectedZoneID == zone?.id { Image(systemName: "checkmark.circle.fill").foregroundStyle(MusicStyle.accent) }
      }
      ForEach(Array(group["zone_ids"].array.enumerated()), id: \.offset) { _, value in
        Label(store.zones.first { $0.id == value.text }.map { store.outputLabels.label($0) } ?? value.text, systemImage: "hifispeaker").font(.caption).foregroundStyle(.secondary)
      }
      if let zone, let track = zone.track { Text(track.title).font(.callout).lineLimit(1); Text("\(zone.queue.count) 首歌曲").font(.caption).foregroundStyle(.secondary) }
      if let error = group["error"].string { Text(error).font(.caption).foregroundStyle(.orange) }
      Divider()
      HStack {
        Button(store.selectedZoneID == group["zone_id"].text ? "当前播放设备" : "在这里听") { store.selectPlaybackZone(group["zone_id"].text) }.disabled(!online || action.busy)
        Spacer()
        Button("调整成员") { open(group) }.disabled(action.busy || (store.outputs.roaming && group["zone_ids"].array.contains{store.family.localMembers.contains($0.text)}))
        Button("拆开", systemImage: "arrow.triangle.branch") { dissolving = group }.disabled(action.busy || (store.outputs.roaming && group["zone_ids"].array.contains{store.family.localMembers.contains($0.text)}))
      }
    }
  }
  private var editor: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 22) {
        HStack { Text(editingGroup == nil ? "组合为一个设备" : "调整组合成员").font(.title2.weight(.semibold)); Spacer(); Button("关闭") { creating = false }.disabled(action.busy) }
        TextField("设备名称，例如客厅与书房", text: $name).disabled(action.busy || pending != nil)
        Text("选择连接到家庭局域网的二至四个输出；可以在外出时遥控组合。").font(.callout).foregroundStyle(.secondary)
        ForEach(available) { zone in
          Toggle(store.outputLabels.label(zone), isOn: Binding(get: { rooms.contains(zone.id) }, set: { if $0 { rooms.insert(zone.id) } else { rooms.remove(zone.id) } })).toggleStyle(.checkbox).disabled(action.busy || pending != nil || rooms.count >= 4 && !rooms.contains(zone.id))
        }
        if available.count < 2 { Text("可组合的在线输出不足两个，请先连接或更新原生客户端／网桥。").font(.caption).foregroundStyle(.secondary) }
        if !rooms.isEmpty && editingGroup == nil {
          ThemePicker("初始队列来自", selection: $source) { ForEach(available.filter { rooms.contains($0.id) }) { Text(store.outputLabels.label($0)).tag($0.id) } }.disabled(action.busy || pending != nil)
          Text("其他成员的队列会单独保留，拆开组合后恢复。").font(.caption).foregroundStyle(.secondary)
        }
        WorkflowNotice(action: action)
        HStack { Spacer(); Button(pending == nil ? editingGroup == nil ? "组合为一个设备" : "保存成员" : "重试本次操作") { create() }.buttonStyle(.borderedProminent).disabled(action.busy || pending == nil && (name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || rooms.count < 2 || source.isEmpty)) }
      }
    }
  }
  private func open(_ group: JSONValue? = nil) {
    editingGroup = group
    pending = nil
    if let group {
      rooms=Set(group["zone_ids"].array.compactMap(\.string));source=group["source_zone_id"].text;name=group["name"].text
    } else {
      rooms=available.contains { $0.id == store.selectedZoneID } ? [store.selectedZoneID] : []
      source=rooms.first ?? "";name="全屋"
    }
    creating = true
  }
  private func create() {
    let body = pending ?? .object(["request_id": .string(UUID().uuidString), "name": .string(name.trimmingCharacters(in: .whitespacesAndNewlines)), "zone_ids": .array(rooms.sorted().map(JSONValue.string)), "source_zone_id": .string(source)])
    let request: JSONValue
    let path: String
    if let group = editingGroup {
      path="/device-groups/\(group["id"].text.pathComponentEncoded)"
      request = .object(["name":.string(name.trimmingCharacters(in:.whitespacesAndNewlines)),"zone_ids":.array(rooms.sorted().map(JSONValue.string)),"expected_revision":group["revision"]])
    } else {path="/device-groups";request=body}
    pending = request
    action.send(path, method:editingGroup == nil ? "POST" : "PUT", body: request, store: store) { value in
      pending = nil; creating = false; store.selectedZoneID = value["zone_id"].text; refresh = UUID()
      Task { try? await store.refreshZones() }
    }
  }
  private func dissolve() {
    guard let group = dissolving else { return }
    action.send("/device-groups/\(group["id"].text.pathComponentEncoded)", method: "DELETE", store: store) { _ in
      if store.selectedZoneID == group["zone_id"].text { store.selectedZoneID = group["source_zone_id"].text }
      dissolving = nil; refresh = UUID(); Task { try? await store.refreshZones() }
    }
  }
}

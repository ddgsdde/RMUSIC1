import SwiftUI

/// The Core inventory is shared by every controller; local HAL controls stay on this Mac.
struct DevicesView: View {
    let store: MusicStore
    @StoredState private var tab = "all"
    @StoredState private var resource = ResourceStore()
    @StoredState private var capabilities = ResourceStore()
    @StoredState private var busy = false
    @StoredState private var filter = "all"
    @StoredState private var renaming: JSONValue?
    @StoredState private var name = ""
    @StoredState private var removing: JSONValue?
    @StoredState private var purging: JSONValue?
    @StoredState private var refreshID = UUID()
    @AppStorage("collapsedDevices") private var collapsed = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {

            ThemePicker("设备范围", selection: $tab) { Text("Core 全部设备").tag("all"); Text("本机 CoreAudio").tag("local"); if store.user["role"].text != "external" { Text("设备组合").tag("groups") } }.pickerStyle(.segmented).frame(width: 430).padding(.horizontal, 28).padding(.top, 20)
            if tab == "groups", store.user["role"].text != "external" { ScrollView { GroupPlaybackView(store: store).padding(28) } }
            else if tab == "local" { OutputsView(store: store) }
            else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 20) {
                        HStack { ThemePicker("设备状态", selection: $filter) { Text("全部").tag("all"); Text("在线").tag("online"); Text("已移除").tag("removed") }.frame(width: 200); Spacer(); Button("刷新", systemImage: "arrow.clockwise") { refreshID = UUID() }.disabled(busy || resource.loading) }
                        ResourceNotice(resource: resource)
                        let items = store.outputs.arrangedDevices(resource.value[filter == "removed" ? "removed" : "items"].array.filter { (!store.outputs.roaming || !store.outputs.isLocalDevice($0)) && (filter != "online" || $0["online"].boolean) })
                        LazyVStack(spacing: 30) { ForEach(Array(items.enumerated()), id: \.offset) { _, device in deviceCard(device, removed: filter == "removed") } }
                        if items.isEmpty && !resource.loading { ContentUnavailableView("暂无设备", systemImage: "hifispeaker.2", description: Text("连接到当前 Core 的客户端与网桥会显示在这里。")) }
                    }.padding(28)
                }
            }

        }.task(id: "\(store.sessionID):\(store.outputs.roaming):\(tab):\(refreshID)") {
            guard store.connected,tab != "local" else { return }
            await capabilities.load("/status", store: store)
            repeat { if store.outputs.roaming {store.family.update(store,force:true)};if tab=="all" {await resource.load("/devices", store: store)}; do { try await Task.sleep(for: .seconds(5)) } catch { return } } while !Task.isCancelled
        }
        .alert("重命名设备", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("设备名称", text: $name)
            Button("取消", role: .cancel) { renaming = nil }
            Button("保存") { if let device = renaming { Task { await mutate("/devices/\(device["id"].text.pathComponentEncoded)", method: "PATCH", body: .object(["name": .string(name)])) } }; renaming = nil }.disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .confirmationDialog("移除此设备并停止其输出？", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
            Button("移除设备", role: .destructive) { if let device = removing { Task { await mutate("/devices/\(device["id"].text.pathComponentEncoded)", method: "DELETE") } }; removing = nil }
        } message: { Text("保留输出设置与队列，可在已移除列表恢复。") }
        .sheet(isPresented: Binding(get: { purging != nil }, set: { if !$0 { purging = nil } })) {
            if let device = purging { DevicePurgeSheet(device: device, store: store, supported: capabilities.value["capabilities"]["device_purge_v1"].boolean) { refreshID = UUID() }.id(store.sessionID) }
        }
    }
    private func deviceCard(_ device: JSONValue, removed: Bool) -> some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(spacing: 12) {
                Button { toggle(device) } label: {
                    Image(systemName: device["platform"].text.contains("android") ? "smartphone" : "desktopcomputer").font(.system(size: 19, weight: .light)).foregroundStyle(MusicStyle.accent).frame(width: 38, height: 38).background(MusicStyle.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
                }.buttonStyle(.plain).accessibilityLabel("\(isCollapsed(device) ? "展开" : "折叠") \(device["name"].text) 的音频设备")
                VStack(alignment: .leading, spacing: 5) {
                    Text(device["name"].text).font(.system(size: 14, weight: .semibold)).lineLimit(1)
                    Text("\(device["platform"].text) · \(isLocal(device) ? "本机" : device["online"].boolean ? "在线" : "离线") · \(device["outputs"].array.count) 路输出").font(.system(size: 10)).foregroundStyle(.secondary)
                }
                .contentShape(Rectangle())
                .gesture(TapGesture(count: 2).exclusively(before: TapGesture()).onEnded { tap in
                    switch tap {
                    case .first: beginRenaming(device)
                    case .second: toggle(device)
                    }
                })
                .help(device["renameable"].boolean ? "双击编辑客户端名称" : "单击展开或折叠")
                Button { toggle(device) } label: {
                    Image(systemName: isCollapsed(device) ? "chevron.right" : "chevron.down").font(.system(size: 10, weight: .semibold)).foregroundStyle(.secondary)
                }.buttonStyle(.plain).accessibilityLabel("\(isCollapsed(device) ? "展开" : "折叠") \(device["name"].text) 的音频设备")
                Spacer()
                if removed {
                    Button("恢复设备") { Task { await mutate("/devices/\(device["id"].text.pathComponentEncoded)/restore", method: "POST") } }
                    if store.user["role"].text == "admin", capabilities.value["capabilities"]["device_purge_v1"].boolean {
                        Button("彻底删除", role: .destructive) { purging = device }
                    }
                }
                else if !device["outputs"].array.contains(where: { output in output["kind"].text == "group" || store.zones.contains(where: { $0.endpoint_id == output["id"].text && !store.playbackZones.map(\.id).contains($0.id) }) }) {
                    Menu("设备操作", systemImage: "ellipsis") {
                        if device["renameable"].boolean { Button("重命名") { beginRenaming(device) } }
                        Button("移除设备", role: .destructive) { removing = device }
                    }.menuStyle(.borderlessButton).fixedSize()
                }
            }.padding(.vertical, 6)
            if !removed && !isCollapsed(device) {
                if isLocal(device) { OutputsView(store: store, embedded: true) }
                else {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 280), spacing: 18)], alignment: .leading, spacing: 18) {
                        ForEach(Array(device["outputs"].array.enumerated()), id: \.offset) { _, output in
                            RemoteOutputCard(output: output, store: store, save: { configure(output, $0) }, control: control, create: { Task { await mutate("/zones", method: "POST", body: .object(["name": output["name"], "endpoint_id": output["id"]])) } })
                        }
                    }
                }
            }
        }.disabled(busy)
    }
    private func isLocal(_ device: JSONValue) -> Bool {
        store.outputs.isLocalDevice(device)
    }
    private func beginRenaming(_ device: JSONValue) {
        guard device["renameable"].boolean else { return }
        renaming = device; name = device["name"].text
    }
    private func configure(_ output: JSONValue, _ changes: [String: JSONValue]) {
        Task {
            if let local = store.outputs.items.first(where: { $0.id == output["id"].text }) {
                await store.perform { try await store.outputs.configure(local, changes: changes, store: store); await resource.load("/devices", store: store) }
            } else {
                var config = output["config"].object
                config.merge(changes) { _, new in new }
                await mutate("/outputs/\(output["id"].text.pathComponentEncoded)/config", method: "PUT", body: .object(config))
            }
        }
    }
    private func control(_ zone: Zone, _ action: String) {
        Task { await store.perform { try await store.requireUngrouped(zone.id); _ = try await store.api("/zones/\(zone.id.pathComponentEncoded)/\(action)", method: "POST", body: .object([:])); try await store.refreshZones() } }
    }
    private func mutate(_ path: String, method: String, body: JSONValue? = nil) async {
        guard !busy else { return }; busy = true; defer { busy = false }
        await store.perform { _ = try await store.api(path, method: method, body: body); try await store.refreshZones(); await resource.load("/devices", store: store) }
    }
    private func isCollapsed(_ device: JSONValue) -> Bool { collapsed.split(separator: "|").contains(Substring(device["id"].text)) }
    private func toggle(_ device: JSONValue) { var ids = Set(collapsed.split(separator: "|").map(String.init)); if !ids.insert(device["id"].text).inserted { ids.remove(device["id"].text) }; collapsed = ids.sorted().joined(separator: "|") }
}

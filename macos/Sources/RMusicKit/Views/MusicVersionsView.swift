import SwiftUI

struct MusicVersionsView: View {
    let track: Track
    let store: MusicStore
    @StoredState private var playbackPreferred: Track?
    @StoredState private var resource = ResourceStore()
    @StoredState private var action = WorkflowAction()
    @StoredState private var offset = 0
    @StoredState private var otherOffset = 0
    @StoredState private var query = ""
    @StoredState private var filter = ""
    @StoredState private var sort = "title"
    @StoredState private var availableOnly = false
    @StoredState private var refresh = UUID()
    @StoredState private var kind = "unspecified"
    @StoredState private var label = ""
    @StoredState private var notes = ""
    @StoredState private var hasDraft = false
    @StoredState private var editionRevision: JSONValue = .null
    private let kinds = [("unspecified", "未标注"), ("original", "原版"), ("live", "现场"), ("remaster", "重制母带"), ("remix", "混音"), ("cover", "翻唱"), ("instrumental", "器乐／伴奏"), ("radio_edit", "电台剪辑"), ("other", "其他")]
    private var path: String { "/library/tracks/\(track.id.pathComponentEncoded)" }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            FeatureHeader(title: "音乐版本", loading: resource.loading) { refresh = UUID() }
            ResourceNotice(resource: resource)
            WorkflowNotice(action: action)
            if resource.value["identity_resolved"].boolean {
                Label("已确认相同录音", systemImage: "checkmark.seal").foregroundStyle(.secondary)
            } else { Label("录音身份尚未确认，同名歌曲保持独立", systemImage: "person.crop.rectangle.badge.questionmark").foregroundStyle(.secondary) }
            if resource.value["candidate_scan_limited"].boolean { Text("关联记录较多，本页仅显示已检查的版本。").font(.caption).foregroundStyle(.secondary) }
            HStack {
                TextField("筛选歌曲、艺术家或专辑", text: $filter).onSubmit { query = filter; offset = 0; otherOffset = 0 }
                Button("筛选") { query = filter; offset = 0; otherOffset = 0 }
                ThemePicker("版本排序", selection: $sort) { Text("名称").tag("title"); Text("来源").tag("source"); Text("最近加入").tag("added") }
                Toggle("仅可用", isOn: $availableOnly).toggleStyle(.checkbox)
            }.disabled(resource.loading || action.busy)
            LazyVStack(spacing: 12) {
                ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
                    if let version = try? item["track"].decode(Track.self) {
                        VStack(alignment: .leading, spacing: 8) {
                            versionRow(version, edition: item["edition"])
                            DisclosureGroup("关系与质量依据") { VersionEvidenceView(evidence: item["quality_evidence"], identity: item["identity"]) }.padding(.horizontal, 16)
                        }
                    }
                }
            }
            OffsetPageControls(offset: $offset, total: resource.value["total"].integer, limit: 20, disabled: resource.loading || action.busy)
            if !resource.value["other_recordings"].array.isEmpty || otherOffset > 0 {
                DisclosureGroup("同一作品的其他录音") {
                    ForEach(Array(resource.value["other_recordings"].array.enumerated()), id: \.offset) { _, value in
                        if let other = try? value.decode(Track.self) { versionRow(other, edition: .null, related: true) }
                    }
                    OffsetPageControls(offset: $otherOffset, total: resource.value["other_page"]["total"].integer, limit: 20, disabled: resource.loading || action.busy)
                }
            }
            VersionPlaybackPolicyView(track: track, store: store, preferred: playbackPreferred)
            if canEdit {
                DisclosureGroup("编辑当前文件的版本资料") {
                    VStack(alignment: .leading, spacing: 14) {
                        ThemePicker("版本类型", selection: $kind) { ForEach(kinds, id: \.0) { Text($0.1).tag($0.0) } }
                        TextField("版本名称", text: $label)
                        TextField("备注", text: $notes, axis: .vertical).lineLimit(2...4)
                        HStack {
                            Button("载入最新资料") { draft() }
                            Spacer()
                            Button("保存资料") {
                                action.send(path + "/edition", method: "PUT", body: .object(["expected_revision": editionRevision, "kind": .string(kind), "label": .string(label), "notes": .string(notes)]), store: store) { value in
                                    editionRevision = value["revision"]; refresh = UUID()
                                }
                            }.disabled(!hasDraft || label.utf8.count > 200 || notes.utf8.count > 2000)
                        }
                    }.padding(.top, 12).disabled(action.busy)
                }
            }
        }.task(id: "\(store.sessionID):\(track.id):\(offset):\(otherOffset):\(query):\(sort):\(availableOnly):\(refresh)") {
            if await resource.load(apiQuery(path + "/versions", ["offset": String(offset), "other_offset": String(otherOffset), "q": query, "sort": sort].merging(availableOnly ? ["available": "true"] : [:]) { _, new in new }), store: store), !hasDraft { draft() }
        }.onChange(of: sort) { offset = 0; otherOffset = 0 }.onChange(of: availableOnly) { offset = 0; otherOffset = 0 }.onDisappear { action.cancel() }
    }
    private var canEdit: Bool { store.user["role"].text == "admin" || store.user["scopes"].array.contains { $0.string == "library" } }
    private func draft() {
        let edition = resource.value["edition"]
        kind = edition["kind"].string ?? "unspecified"; label = edition["label"].text; notes = edition["notes"].text; editionRevision = edition["revision"]; hasDraft = !edition.object.isEmpty
    }
    private func versionRow(_ version: Track, edition: JSONValue, related: Bool = false) -> some View {
        FeatureCard {
            HStack(spacing: 14) {
                ArtworkView(track: version, store: store).frame(width: 52, height: 52)
                VStack(alignment: .leading, spacing: 5) {
                    Text(version.title).font(.headline)
                    HStack { CatalogTextLink(track: version, store: store); CatalogTextLink(track: version, store: store, album: true) }.font(.caption).foregroundStyle(.secondary)
                    Text([edition["label"].text.isEmpty ? kinds.first { $0.0 == edition["kind"].text }?.1 ?? "独立录音" : edition["label"].text, version.quality].joined(separator: " · ")).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("播放此版本", systemImage: "play.fill") { Task { await store.play(version, exactVersion: true) } }.disabled(!store.canPlay || version.available == false)
                Menu("操作") {
                    TrackActions(track: version, store: store)
                    if !related && resource.value["identity_resolved"].boolean { Button("选为播放首选") { playbackPreferred = version } }
                    if !related && resource.value["identity_resolved"].boolean {
                        let preferred = resource.value["preferred"]
                        Button(preferred["track_id"].text == version.id ? "取消推荐偏好" : "推荐优先使用此版本") {
                            action.send(path + "/versions", method: "PUT", body: .object(["expected_revision": preferred["revision"], "track_id": preferred["track_id"].text == version.id ? .null : .string(version.id)]), store: store) { _ in refresh = UUID() }
                        }
                    }
                }.disabled(action.busy)
            }
        }
    }
}
struct WorkflowNotice: View {
    let action: WorkflowAction
    var body: some View {
        if action.busy { ProgressView().controlSize(.small) }
        if let error = action.error { Label(error, systemImage: "exclamationmark.circle").font(.callout).foregroundStyle(.orange).textSelection(.enabled) }
        if let notice = action.notice { Text(notice).font(.caption).foregroundStyle(.secondary) }
    }
}
struct OffsetPageControls: View {
    @Binding var offset: Int
    let total: Int
    let limit: Int
    var disabled = false
    var body: some View {
        if total > limit {
            HStack {
                Button("上一页") { offset = max(0, offset - limit) }.disabled(disabled || offset == 0)
                Spacer(); Text("\(min(total, offset + 1))–\(min(total, offset + limit)) / \(total)").font(.caption).monospacedDigit(); Spacer()
                Button("下一页") { offset += limit }.disabled(disabled || offset + limit >= total)
            }.buttonStyle(MusicControlStyle())
        }
    }
}

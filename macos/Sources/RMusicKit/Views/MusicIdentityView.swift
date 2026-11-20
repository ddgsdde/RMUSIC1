import SwiftUI

struct MusicIdentityView: View {
  let track: Track
  let store: MusicStore
  @StoredState private var identity = ResourceStore()
  @StoredState private var history = ResourceStore()
  @StoredState private var fingerprint = ResourceStore()
  @StoredState private var related = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var draft: [IdentityAssertion] = []
  @StoredState private var editing = false
  @StoredState private var showingHistory = false
  @StoredState private var historyOffset = 0
  @StoredState private var fingerprintOffset = 0
  @StoredState private var entityID = ""
  @StoredState private var relatedOffset = 0
  @StoredState private var refresh = UUID()
  @StoredState private var choosing: UUID?
  @StoredState private var preview = JSONValue.null
  @StoredState private var previewStamp = ""
  @StoredState private var restoreRevision: JSONValue = .null
  @StoredState private var restoreStamp = ""
  private var restoring: Bool { restoreRevision.number > 0 && restoreStamp == draftStamp }
  private var draftStamp: String { let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys; return (try? encoder.encode(JSONValue.array([identity.value["revision"], .array(draft.map(\.value))])).base64EncodedString()) ?? "" }
  private var path: String { "/library/tracks/\(track.id.pathComponentEncoded)" }
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      FeatureHeader(title: "作品、录音与发行", loading: identity.loading) { refresh = UUID() }
      ResourceNotice(resource: identity)
      WorkflowNotice(action: action)
      Text("同名不建立关系。人工确认保留稳定身份，指纹只提供录音候选。").font(.caption).foregroundStyle(.secondary)
      ForEach(Array(identity.value["entities"].array.enumerated()), id: \.offset) { _, entity in
        FeatureCard {
          HStack {
            VStack(alignment: .leading, spacing: 5) {
              Text(entity["title"].text).font(.headline)
              Text(
                "\(label(entity["kind"].text, role: entity["role"].text)) · \(entity["namespace"].text) · \(entity["basis"].text == "manual" ? "人工确认" : "来源资料")"
              ).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Button("浏览关联曲目") {
              entityID = entity["id"].text
              relatedOffset = 0
            }
            if editable {
              Button("确认此身份") {
                if !editing {
                  draft = IdentityAssertion.manual(identity.value)
                  editing = true
                }
                if draft.count < 32 { draft.append(IdentityAssertion(entity)) }
              }
            }
          }
        }
      }
      HStack {
        if editable {
          Button(editing ? "取消编辑" : "编辑人工关系") {
            if !editing { draft = IdentityAssertion.manual(identity.value); restoreRevision = .null; previewStamp = "" }
            editing.toggle()
          }
        }
        Button(showingHistory ? "收起修订记录" : "修订记录") { showingHistory.toggle() }
      }
      if editing { editor }
      if showingHistory {
        ResourceNotice(resource: history)
        ForEach(Array(history.value["items"].array.enumerated()), id: \.offset) { _, item in
          FeatureCard {
            Text(
              "修订 \(item["revision"].integer) · \(Date(timeIntervalSince1970: item["created_at"].number).formatted())"
            ).font(.headline)
            Text(
              item["after"]["entities"].array.filter { $0["basis"].text == "manual" }.map {
                $0["title"].text
              }.joined(separator: " · ")
            ).font(.caption)
            if editable {
              Button("预览恢复此修订") {
                draft = IdentityAssertion.manual(item["after"])
                restoreRevision = item["revision"]; restoreStamp = draftStamp; previewStamp = ""; editing = true
              }
            }
          }
        }
        OffsetPageControls(offset: $historyOffset, total: history.value["total"].integer, limit: 20)
      }
      if !entityID.isEmpty {
        FeatureCard {
          HStack {
            Text(related.value["entity"]["title"].string ?? "关联曲目").font(.headline)
            Spacer()
            Button("收起") { entityID = "" }
          }
          ResourceNotice(resource: related)
          ForEach(Array(related.value["items"].array.enumerated()), id: \.offset) { index, value in
            if let item = try? value.decode(Track.self) {
              MusicTrackRow(track: item, index: relatedOffset + index, store: store)
            }
          }
          OffsetPageControls(
            offset: $relatedOffset, total: related.value["total"].integer, limit: 20)
        }
      }
      FeatureCard {
        HStack {
          Text("音频指纹候选").font(.headline)
          Spacer()
          if editable {
            Button("分析指纹") {
              action.send(path + "/fingerprint", store: store) { _ in
                action.notice = "指纹分析已排队，完成后刷新查看。"
              }
            }
          }
          Button("刷新") { refresh = UUID() }
        }
        ResourceNotice(resource: fingerprint)
        if !fingerprint.value["available"].boolean {
          Text(fingerprint.value["reason"].text).font(.caption).foregroundStyle(.secondary)
        }
        ForEach(Array(fingerprint.value["items"].array.enumerated()), id: \.offset) { index, item in
          if let candidate = try? item["track"].decode(Track.self) {
            HStack {
              MusicTrackRow(track: candidate, index: index, store: store)
              Text("相似度 \((item["similarity"].number * 100).formatted())%").font(.caption)
            }
          }
        }
        HStack {
          Button("上一批") { fingerprintOffset = max(0, fingerprintOffset - 20) }.disabled(
            fingerprintOffset == 0)
          Spacer()
          Button("下一批") { fingerprintOffset = fingerprint.value["next_offset"].integer }.disabled(
            fingerprint.value["next_offset"].number <= Double(fingerprintOffset))
        }
        Text("候选不证明同一母带或发行，不自动合并。").font(.caption).foregroundStyle(.secondary)
      }
      if editable {
        DisclosureGroup("曲库身份维护") {
          Button("更新全曲库身份索引") { action.send("/music/index", store: store) }
        }
      }
    }.task(id: "\(store.sessionID):\(track.id):\(refresh)") {
      await identity.load(path + "/identity", store: store)
    }
    .task(id: "\(showingHistory):\(historyOffset):\(refresh)") {
      if showingHistory {
        await history.load(
          path + "/identity/history?offset=\(historyOffset)&limit=20", store: store)
      }
    }
    .task(id: "\(entityID):\(relatedOffset)") {
      if !entityID.isEmpty {
        await related.load(
          "/music/entities/\(entityID.pathComponentEncoded)?offset=\(relatedOffset)&limit=20",
          store: store)
      }
    }
    .task(id: "\(store.sessionID):\(track.id):\(fingerprintOffset):\(refresh)") {
      await fingerprint.load(path + "/fingerprint?offset=\(fingerprintOffset)", store: store)
    }
    .onDisappear {
      action.cancel()
      draft = []
    }
    .sheet(isPresented: Binding(get: { choosing != nil }, set: { if !$0 { choosing = nil } })) {
      if let index = draft.firstIndex(where: { $0.id == choosing }) {
        MusicEntityPicker(
          store: store, kind: draft[index].kind,
          selected: { value in
            let role = draft[index].role
            draft[index] = IdentityAssertion(value)
            draft[index].role = role
            choosing = nil
          }, close: { choosing = nil })
      }
    }
  }
  private var editor: some View {
    FeatureCard {
      Text("人工确认的关系").font(.headline)
      ForEach($draft) { $item in
        VStack(alignment: .leading, spacing: 10) {
          HStack {
            ThemePicker("类型", selection: $item.kind) {
              ForEach(IdentityAssertion.kinds, id: \.0) { Text($0.1).tag($0.0) }
            }.onChange(of: item.kind) {
              item.role = item.kind == "artist" ? "performer" : item.kind
              item.namespace = "rmusic-manual"
              item.externalID = UUID().uuidString.lowercased()
            }
            TextField("名称", text: $item.title)
          }
          if item.kind == "artist" {
            ThemePicker("参与角色", selection: $item.role) {
              ForEach(IdentityAssertion.roles, id: \.0) { Text($0.1).tag($0.0) }
            }
          }
          HStack {
            Button("从曲库选择同一身份") { choosing = item.id }
            Button("撤回关系", role: .destructive) { draft.removeAll { $0.id == item.id } }
          }
          DisclosureGroup("稳定身份") {
            TextField("身份来源", text: $item.namespace)
            TextField("稳定 ID", text: $item.externalID).textSelection(.enabled)
          }
          Divider().opacity(0.3)
        }
      }
      if previewStamp == draftStamp {
        VStack(alignment: .leading, spacing: 7) {
          Text("变更预览 · 可能关联 \(preview["affected_candidates"].integer) 条记录").font(.headline)
          ForEach(["work", "recording", "release"], id: \.self) { kind in
            let state = preview["resolution"][kind]
            if state["conflict"].boolean { Label("同类身份存在冲突，保存后不会自动合并", systemImage: "exclamationmark.triangle").foregroundStyle(.orange) }
            else if let name = state["selected"]["title"].string { Text("\(label(kind, role: kind)) · \(name)").font(.caption) }
          }
          Text("歌单、音频文件和播放 ID 保留；恢复操作将新增修订。").font(.caption).foregroundStyle(.secondary)
        }
      }
      HStack {
        Button("添加关系") { draft.append(IdentityAssertion()) }.disabled(draft.count >= 32)
        Spacer()
        Button("预览变更") {
          let stamp = draftStamp
          action.send(path + "/identity/preview", method: "POST", body: .object(["expected_revision": identity.value["revision"], "assertions": .array(draft.map(\.value))]), store: store) { preview = $0; previewStamp = stamp; action.notice = "预览完成，尚未保存" }
        }.disabled(action.busy)
        Button("保存人工确认") {
          action.send(
            path + (restoring ? "/identity/restore" : "/identity"), method: restoring ? "POST" : "PUT",
            body: restoring ? .object(["expected_revision": identity.value["revision"], "revision": restoreRevision]) : .object([
              "expected_revision": identity.value["revision"],
              "assertions": .array(draft.map(\.value)),
            ]), store: store
          ) {
            identity.value = $0
            editing = false; previewStamp = ""; restoreRevision = .null
            refresh = UUID()
          }
        }.disabled(
          action.busy || previewStamp != draftStamp
            || draft.contains { $0.title.isEmpty || $0.externalID.isEmpty || $0.namespace.isEmpty })
      }
    }
  }
  private var editable: Bool {
    store.user["role"].text == "admin"
      || store.user["scopes"].array.contains { $0.text == "library" }
  }
  private func label(_ kind: String, role: String) -> String {
    if kind == "artist" { return IdentityAssertion.roles.first { $0.0 == role }?.1 ?? "参与者" }
    return IdentityAssertion.kinds.first { $0.0 == kind }?.1 ?? kind
  }
}
struct MusicEntityPicker: View {
  let store: MusicStore
  let kind: String
  let selected: (JSONValue) -> Void
  let close: () -> Void
  @StoredState private var query = ""
  @StoredState private var offset = 0
  @StoredState private var resource = ResourceStore()
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      HStack {
        Text("选择曲库中的同一身份").font(.title2.bold())
        Spacer()
        Button("取消", action: close)
      }
      TextField("搜索名称或稳定 ID", text: $query)
      ResourceNotice(resource: resource)
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 12) {
          ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, item in
            HStack {
              VStack(alignment: .leading, spacing: 5) {
                Text(item["title"].text)
                Text("\(item["namespace"].text) · \(item["external_id"].text)").font(.caption)
                  .foregroundStyle(.secondary)
              }
              Spacer()
              Button("选择此身份") { selected(item) }
            }
          }
        }
      }
      OffsetPageControls(offset: $offset, total: resource.value["total"].integer, limit: 20)
    }.padding(24).frame(width: 700, height: 500).onChange(of: query) { offset = 0 }
      .task(id: "\(store.sessionID):\(query):\(offset)") {
        do {
          try await Task.sleep(for: .milliseconds(200))
          try Task.checkCancellation()
        } catch { return }
        await resource.load(
          apiQuery("/music/entities", ["q": query, "kind": kind, "offset": String(offset)]),
          store: store)
      }
  }
}

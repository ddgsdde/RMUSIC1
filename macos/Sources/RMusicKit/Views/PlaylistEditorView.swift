import SwiftUI

struct PlaylistEditorView: View {
  let store: MusicStore
  let id: String
  let close: () -> Void
  @StoredState private var draft = PlaylistDraft()
  @StoredState private var source = ResourceStore()
  @StoredState private var page = ResourceStore()
  @StoredState private var search = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var offset = 0
  @StoredState private var query = ""
  @StoredState private var refresh = UUID()
  @StoredState private var searchID = UUID()
  @StoredState private var submittedQuery = ""
  @StoredState private var titles: [String: JSONValue] = [:]
  @StoredState private var failure: String?
  private var path: String { "/playlists/\(id.pathComponentEncoded)" }
  private var visibleIDs: [String] { Array(draft.ids.dropFirst(offset).prefix(50)) }
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      HStack {
        Text("编辑播放列表").font(.title2.bold())
        Spacer()
        Button("关闭", action: close)
      }
      ResourceNotice(resource: source)
      ResourceNotice(resource: page)
      ResourceNotice(resource: search)
      WorkflowNotice(action: action)
      if let failure { Text(failure).font(.caption).foregroundStyle(.orange) }
      TextField("歌单名称", text: $draft.name)
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 10) {
          ForEach(Array(draft.ids.dropFirst(offset).prefix(50).enumerated()), id: \.offset) {
            local, trackID in
            let index = offset + local
            let info = titles[trackID] ?? .null
            HStack {
              TrackOrderHandle(scope: "playlist-draft:\(store.sessionID):\(id)", ids: draft.ids, index: index)
              Text("\(index + 1)").monospacedDigit().foregroundStyle(.secondary).frame(width: 35)
              VStack(alignment: .leading) {
                Text(info["title"].string ?? trackID).lineLimit(1)
                CatalogTextLink(track: catalogSummary(info), store: store).font(.caption).foregroundStyle(.secondary)
              }
              Spacer()
              Button("上移第 \(index + 1) 首", systemImage: "arrow.up") { draft.move(index, by: -1) }
                .labelStyle(.iconOnly).disabled(index == 0)
              Button("下移第 \(index + 1) 首", systemImage: "arrow.down") { draft.move(index, by: 1) }
                .labelStyle(.iconOnly).disabled(index + 1 == draft.ids.count)
              Button("移除第 \(index + 1) 首", systemImage: "minus.circle") {
                draft.ids.remove(at: index)
                offset = min(offset, max(0, (draft.ids.count - 1) / 50 * 50))
              }.labelStyle(.iconOnly)
            }.modifier(TrackOrderDrop(scope: "playlist-draft:\(store.sessionID):\(id)", ids: draft.ids, index: index) { from, to in
              draft.ids = movedTrackIDs(draft.ids, from: from, to: to)
            })
          }
        }
      }
      OffsetPageControls(offset: $offset, total: draft.ids.count, limit: 50)
      HStack {
        TextField("从 Core 曲库搜索", text: $query)
        Button("搜索") {
          submittedQuery = query
          searchID = UUID()
        }.disabled(query.trimmingCharacters(in: .whitespaces).isEmpty)
      }
      if !search.value["items"].array.isEmpty {
        ScrollView {
          LazyVStack {
            ForEach(Array(search.value["items"].array.enumerated()), id: \.offset) { _, item in
              Button {
                if draft.ids.count < 10000 {
                  draft.ids.append(item["id"].text)
                  if visibleIDs.contains(item["id"].text) { titles[item["id"].text] = item }
                }
              } label: {
                HStack {
                  Text("\(item["title"].text) · \(item["artist"].text)")
                  Spacer()
                  Text("添加")
                }
              }.disabled(draft.ids.count >= 10000)
            }
          }
        }.frame(maxHeight: 150)
      }
      HStack {
        Button("读取最新歌单并重新编辑") { refresh = UUID() }
        Spacer()
        Button("保存歌单") { save() }.buttonStyle(.borderedProminent).disabled(
          source.loading || action.busy || draft.name.isEmpty)
      }
    }.padding(24).frame(width: 800, height: 700)
      .task(id: "\(store.sessionID):\(id):\(refresh)") {
        if await source.load(path, store: store),
          let playlist = try? source.value.decode(Playlist.self)
        {
          draft.load(playlist)
          offset = 0
          failure = nil
        }
      }
      .task(id: "\(store.sessionID):\(searchID)") {
        guard !submittedQuery.isEmpty else { return }
        await search.load(
          apiQuery("/library/tracks", ["q": submittedQuery, "limit": "20"]), store: store)
      }
      .task(id: "\(store.sessionID):\(id):\(offset):\(visibleIDs.joined(separator: ","))") {
        let ids = visibleIDs
        guard !ids.isEmpty else {
          titles = [:]
          return
        }
        guard await page.load(path + "/tracks?limit=50&offset=\(offset)", store: store) else {
          return
        }
        var next: [String: JSONValue] = [:]
        for item in page.value["items"].array where ids.contains(item["id"].text) {
          next[item["id"].text] = item
        }
        for key in ids where next[key] == nil { next[key] = titles[key] }
        let missing = Array(Set(ids.filter { next[$0] == nil }))
        // Only the visible page is hydrated, at most four Core requests concurrently.
        for start in stride(from: 0, to: missing.count, by: 4) {
          let values = await withTaskGroup(of: (String, JSONValue?).self) { group in
            for key in missing.dropFirst(start).prefix(4) {
              group.addTask {
                (key, try? await store.api("/library/tracks/\(key.pathComponentEncoded)"))
              }
            }
            var values: [(String, JSONValue?)] = []
            for await item in group { values.append(item) }
            return values
          }
          guard !Task.isCancelled else { return }
          for (key, value) in values { next[key] = value }
        }
        guard !Task.isCancelled else { return }
        titles = next
      }.onDisappear {
        action.cancel()
        titles = [:]
        search.value = .null
      }
  }
  private func save() {
    let expected = draft
    guard let original = try? source.value.decode(Playlist.self) else { return }
    let body: JSONValue
    do { body = try expected.payload(latest: original) } catch {
      failure = error.localizedDescription
      return
    }
    action.send(
      path, method: "PUT", body: body,
      preflight: {
        let latest = try await store.api(path).decode(Playlist.self)
        _ = try expected.payload(latest: latest)
      }, store: store
    ) { _ in
      Task {
        await store.perform {
          try await store.collection.refresh(store)
          if store.collection.selectedID == id { try await store.collection.open(id, store: store, page: store.collection.page) }
        }
      }
      close()
    }
  }
}

import SwiftUI

/// A bounded Core library page. Selection contains only Core track IDs, never local file URLs.
struct DownloadSelectionView: View {
    let store: MusicStore
    let close: () -> Void
    @StoredState private var resource = ResourceStore()
    @StoredState private var query = ""
    @StoredState private var offset = 0
    @StoredState private var selected: Set<String> = []
    @StoredState private var action = false
    private var path: String { apiQuery("/library/tracks", ["q": query, "limit": "50", "offset": String(offset)]) }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack { Text("从 Core 选择下载").font(.title2.bold()); Spacer(); Button("取消", action: close) }
            TextField("搜索 Core 音乐库", text: $query).textFieldStyle(.roundedBorder)
            HStack {
                Text("已选 \(selected.count) 首 / 每批 1000 首").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("选择本页") { selected.formUnion(resource.value["items"].array.filter { (try? $0.decode(Track.self))?.available != false }.compactMap { $0["id"].string }.prefix(max(0, 1000 - selected.count))) }
                Button("清空选择") { selected = [] }.disabled(selected.isEmpty)
            }
            ResourceNotice(resource: resource)
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, value in
                        if let track = try? value.decode(Track.self) {
                            Toggle(isOn: Binding(get: { selected.contains(track.id) }, set: { wanted in if wanted && selected.count < 1000 { selected.insert(track.id) } else { selected.remove(track.id) } })) {
                                VStack(alignment: .leading, spacing: 5) { Text(track.title).lineLimit(1); Text("\(track.artist) · \(track.quality)").font(.caption).foregroundStyle(.secondary) }
                            }.toggleStyle(.checkbox).padding(.vertical, 10).disabled(track.available == false)
                            Divider().opacity(0.3)
                        }
                    }
                }
            }.overlay { if resource.loading { ProgressView() } }
            OffsetPageControls(offset: $offset, total: resource.value["total"].integer, limit: 50, disabled: resource.loading)
            HStack { Text("原格式下载，不转换音频。不覆盖同名文件。").font(.caption).foregroundStyle(.secondary); Spacer(); Button("下载所选 \(selected.count) 首") { start() }.buttonStyle(.borderedProminent).disabled(selected.isEmpty || action) }
        }.padding(24).frame(width: 720, height: 620)
            .task(id: "\(store.sessionID):\(path)") { do { try await Task.sleep(for: .milliseconds(200)); try Task.checkCancellation() } catch { return }; await resource.load(path, store: store) }
            .onChange(of: query) { offset = 0 }
    }
    private func start() {
        action = true
        Task { await store.perform { try await store.downloads.queue(Array(selected), store: store); close() }; action = false }
    }
}

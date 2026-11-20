import SwiftUI
import UniformTypeIdentifiers

struct DownloadsView: View {
    @Bindable var store: MusicStore
    @StoredState private var choosingDirectory = false
    @StoredState private var selecting = false
    @AppStorage("downloadDirectory") private var directory = ""
    @StoredState private var removing: CachedTrack?
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                HStack {
                    VStack(alignment: .leading, spacing: 9) {
                        Text("下载与缓存").font(.system(size: 18, weight: .semibold))
                    }
                    Spacer()
                    Menu {
                        ForEach([1, 5, 10, 20], id: \.self) { size in Button("\(size) GiB") { Task { await store.perform { try await store.downloads.setLimit(UInt64(size) * 1024 * 1024 * 1024, store: store) } } } }
                    } label: { Label("缓存上限", systemImage: "internaldrive") }
                }
                FeatureCard {
                    HStack { Label("原文件下载", systemImage: "folder").font(.headline); Spacer(); Button("批量下载", systemImage: "checklist") { selecting = true }.disabled(!store.connected) }
                    HStack { Text(directory.isEmpty ? "选择下载目录" : directory).font(.caption).foregroundStyle(.secondary).lineLimit(2).textSelection(.enabled); Spacer(); Button(directory.isEmpty ? "选择文件夹" : "更改位置") { choosingDirectory = true } }
                    if !store.downloads.pendingIDs.isEmpty {
                        HStack { Text("已选 \(store.downloads.pendingIDs.count) 首 Core 歌曲").font(.caption); Spacer(); Button("开始下载") { Task { await store.perform { try await store.downloads.queue(Array(store.downloads.pendingIDs), store: store) } } }.disabled(directory.isEmpty || !store.connected); Button("取消选择") { store.downloads.pendingIDs = [] } }
                    }
                }
                if !store.downloads.fileDownloads["items"].array.isEmpty { exportedFiles }
                HStack(spacing: 16) {
                    Image(systemName: "arrow.down.circle.fill").font(.largeTitle).foregroundStyle(MusicStyle.accent)
                    VStack(alignment: .leading, spacing: 6) {
                        Text("自动缓存 \(bytes(store.downloads.used)) / \(bytes(store.downloads.limit))").font(.headline)
                        Text("手动保留 \(bytes(store.downloads.manualBytes))").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                }.padding(22).background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 16))
                if store.downloads.entries.isEmpty && store.downloads.jobs.isEmpty { ContentUnavailableView("暂无下载", systemImage: "arrow.down.to.line", description: Text("通过歌曲菜单下载原文件，下载中断后可继续。")) }
                ForEach(store.downloads.jobs.keys.sorted().filter { id in !store.downloads.entries.contains { $0.track_id == id } }, id: \.self) { id in
                    HStack { ProgressView().controlSize(.small); Text(store.downloads.state(id)); Spacer(); Button("暂停") { cancel(id) } }
                }
                OffsetPageControls(offset: Binding(get: { store.downloads.cacheOffset }, set: { store.downloads.cacheOffset = $0 }), total: store.downloads.cacheTotal, limit: 50)
                LazyVStack(spacing: 8) {
                    ForEach(store.downloads.entries) { entry in downloadRow(entry) }
                }
            }.padding(30)
        }
        .task(id: "\(store.sessionID):\(store.downloads.cacheOffset):\(store.downloads.fileOffset)") { await store.perform { try await store.downloads.refresh(store) } }
        .fileImporter(isPresented: $choosingDirectory, allowedContentTypes: [.folder], allowsMultipleSelection: false) { result in
            switch result { case .success(let urls): if let url = urls.first { directory = url.path }; case .failure(let error): store.error = error.localizedDescription }
        }
        .sheet(isPresented: $selecting) { DownloadSelectionView(store: store, close: { selecting = false }) }
        .confirmationDialog("移除此缓存文件？", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }), titleVisibility: .visible) {
            Button("移除缓存", role: .destructive) { if let entry = removing { Task { await store.perform { try await store.downloads.remove(entry, store: store) } } }; removing = nil }
        } message: { Text("Core 音乐库与歌单不会因此改变。正在使用的缓存文件不能移除。") }
    }
    private var exportedFiles: some View {
        FeatureCard {
            Text("目录下载 · \(store.downloads.fileDownloads["active"].integer) 项进行中").font(.headline)
            LazyVStack(spacing: 12) {
                ForEach(Array(store.downloads.fileDownloads["items"].array.enumerated()), id: \.offset) { _, item in
                    HStack(alignment: .top, spacing: 12) {
                        Image(systemName: item["status"].text == "complete" ? "checkmark.circle.fill" : "arrow.down.circle").foregroundStyle(MusicStyle.accent)
                        VStack(alignment: .leading, spacing: 5) {
                            Text(item["path"].string.map { URL(fileURLWithPath: $0).lastPathComponent } ?? store.downloads.entries.first { $0.track_id == item["track_id"].text }?.title ?? "正在获取歌曲资料").lineLimit(1)
                            Text(item["directory"].text).font(.caption).foregroundStyle(.secondary).lineLimit(1).textSelection(.enabled)
                            if let error = item["error"].string { Text(error).font(.caption).foregroundStyle(.orange) }
                        }
                        Spacer()
                        Text(["queued": "等待下载", "downloading": "下载中", "exporting": "校验并保存", "complete": "已保存", "paused": "已暂停"][item["status"].text] ?? item["status"].text).font(.caption).foregroundStyle(.secondary)
                        if item["status"].text != "complete" {
                            let relocated = item["status"].text == "paused" && !directory.isEmpty && directory != item["directory"].text
                            Button(relocated ? "保存到所选目录" : item["status"].text == "paused" ? "继续" : "暂停") {
                                Task { await store.perform { try await store.downloads.changeFile(item["id"].text, resume: item["status"].text == "paused", directory: relocated ? directory : nil, store: store) } }
                            }
                        }
                    }
                }
            }
            OffsetPageControls(offset: Binding(get: { store.downloads.fileOffset }, set: { store.downloads.fileOffset = $0 }), total: store.downloads.fileDownloads["total"].integer, limit: 50)
        }
    }
    private func downloadRow(_ entry: CachedTrack) -> some View {
        let job = store.downloads.jobs[entry.track_id]
        let active = job?.state == "downloading" || job?.state == "queued"
        return HStack(spacing: 16) {
            Image(systemName: entry.complete ? "checkmark.circle.fill" : "arrow.down.circle").font(.title2).foregroundStyle(entry.complete ? MusicStyle.accent : Color.secondary)
            VStack(alignment: .leading, spacing: 7) {
                Text(entry.title ?? entry.track_id).fontWeight(.medium).lineLimit(1)
                CatalogTextLink(track: Track(id: entry.track_id, title: entry.title ?? "", artist: entry.artist ?? "", album: "", duration: 0, format: "", sample_rate: 0, bit_depth: 0), store: store).font(.caption).foregroundStyle(.secondary)
                Text("\(entry.format.uppercased()) · \(bytes(entry.received)) / \(bytes(entry.total_bytes))\(entry.manual_download ? " · 手动保留" : "")").font(.caption).foregroundStyle(.secondary)
                if !entry.complete { ProgressView(value: Double(entry.received), total: Double(max(1, entry.total_bytes))).tint(MusicStyle.accent) }
                if let error = job?.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            }.frame(maxWidth: .infinity, alignment: .leading)
            if active { Button("暂停") { cancel(entry.track_id) } }
            else if !entry.complete { Button("继续下载") { Task { await store.perform { try await store.downloads.start(entry.track_id, store: store) } } } }
            Menu {
                if entry.complete { Button("播放") { Task { await store.perform { let track = try await store.api("/library/tracks/\(entry.track_id.pathComponentEncoded)").decode(Track.self); await store.play(track) } } } }
                Button("移除缓存", role: .destructive) { removing = entry }.disabled(active)
            } label: { Image(systemName: "ellipsis") }.menuStyle(.borderlessButton).frame(width: 24)
        }.padding(18).background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 12))
    }
    private func cancel(_ id: String) { Task { await store.perform { try await store.downloads.cancel(id, store: store) } } }
    private func bytes(_ n: UInt64) -> String { n == 0 ? "0 B" : ByteCountFormatter.string(fromByteCount: Int64(clamping: n), countStyle: .binary) }
}

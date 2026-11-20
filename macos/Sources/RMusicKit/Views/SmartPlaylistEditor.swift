import SwiftUI

struct SmartPlaylistEditor: View {
    let store: MusicStore
    let playlist: Playlist?
    let close: () -> Void
    let saved: () -> Void
    @StoredState private var name: String
    @StoredState private var draft: SmartPlaylistDraft
    @StoredState private var action = WorkflowAction()
    @StoredState private var preview: JSONValue = .null
    init(store: MusicStore, playlist: Playlist? = nil, initialName: String = "", initialRule: JSONValue = .null, close: @escaping () -> Void, saved: @escaping () -> Void) {
        self.store = store; self.playlist = playlist; self.close = close; self.saved = saved
        _name = StoredState(initialValue: playlist?.name ?? initialName)
        _draft = StoredState(initialValue: SmartPlaylistDraft(playlist?.smart_query ?? initialRule))
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack { Text(playlist == nil ? "创建智能歌单" : "编辑智能歌单").font(.title2.bold()); Spacer(); Button("关闭", action: close) }.padding(24)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    TextField("歌单名称", text: $name)
                    LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 16) {
                        TextField("搜索词", text: field("q"))
                        TextField("音源 ID（留空不限）", text: field("source"))
                        ThemePicker("文件类型", selection: field("storage")) { Text("全部").tag(""); Text("本地原文件").tag("local"); Text("完整缓存").tag("cache") }
                        TextField("格式，例如 flac", text: field("format"))
                        ThemePicker("排序", selection: field("sort")) { Text("歌名").tag(""); Text("最近聆听").tag("recent"); Text("聆听少到多").tag("least_played"); Text("聆听多到少").tag("most_played"); Text("最近加入").tag("recent_added") }
                        TextField("最多选取", text: field("max_results"))
                    }
                    DisclosureGroup("聆听与音质条件") {
                        LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 14) {
                            ForEach(SmartPlaylistDraft.numeric.filter { $0.0 != "max_results" }, id: \.0) { key, title, _ in
                                VStack(alignment: .leading, spacing: 6) { Text(title).font(.caption).foregroundStyle(.secondary); TextField("不限制", text: field(key)) }
                            }
                        }.padding(.top, 12)
                    }
                    Toggle("仅我的收藏", isOn: $draft.favorites)
                    Toggle("排除已加入我的手动歌单的歌曲", isOn: $draft.excludeSaved)
                    Text("聆听次数使用共享 Core 统计；收藏与歌单条件属于当前账号。规则会随曲库更新。").font(.caption).foregroundStyle(.secondary)
                    WorkflowNotice(action: action)
                    HStack {
                        Button("预览匹配") { inspect(0) }
                        Spacer()
                        Button("保存智能歌单") { save() }.buttonStyle(.borderedProminent).disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                    if !preview.object.isEmpty {
                        Text("匹配 \(preview["total"].integer) 首\(preview["truncated"].boolean ? " · 已达到选取上限" : "")").font(.headline)
                        LazyVStack(alignment: .leading, spacing: 12) {
                            ForEach(Array(preview["items"].array.enumerated()), id: \.offset) { _, value in
                                if let track = try? value.decode(Track.self) { HStack { VStack(alignment: .leading) { Text(track.title); HStack { CatalogTextLink(track: track, store: store); CatalogTextLink(track: track, store: store, album: true) }.font(.caption).foregroundStyle(.secondary) }; Spacer(); Button("播放", systemImage: "play.fill") { Task { await store.play(track) } }.labelStyle(.iconOnly).disabled(!store.canPlay) } }
                            }
                        }
                        HStack {
                            Button("上一页") { inspect(max(0, preview["offset"].integer - 30)) }.disabled(preview["offset"].integer == 0)
                            Spacer()
                            Button("下一页") { inspect(preview["offset"].integer + 30) }.disabled(preview["offset"].integer + 30 >= preview["total"].integer)
                        }
                    }
                }.padding(24).disabled(action.busy)
            }
        }.textFieldStyle(.roundedBorder).frame(width: 680, height: 650)
            .onChange(of: draft) { preview = .null }.onDisappear { action.cancel() }
    }
    private func field(_ key: String) -> Binding<String> { Binding(get: { draft.strings[key] ?? "" }, set: { draft.strings[key] = $0 }) }
    private func inspect(_ offset: Int) {
        do { action.send("/smart-playlists/preview", body: .object(["rule": try draft.rule(), "offset": .number(Double(offset))]), store: store) { value in preview = value; action.notice = nil } }
        catch { action.error = error.localizedDescription }
    }
    private func save() {
        do {
            action.send(playlist.map { "/playlists/\($0.id.pathComponentEncoded)" } ?? "/playlists", method: playlist == nil ? "POST" : "PUT", body: .object(["name": .string(name), "smart_query": try draft.rule()]), store: store) { _ in saved(); close() }
        } catch { action.error = error.localizedDescription }
    }
}

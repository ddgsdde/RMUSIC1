import SwiftUI

struct CatalogSortPicker: View {
    let title: String
    @Binding var value: String
    var options: [String]
    var compact = false
    var defaultLabel: String? = nil
    static let labels = ["default": "默认顺序", "title": "名称 A–Z", "title_desc": "名称 Z–A", "artist": "艺术家", "album": "专辑曲序", "added": "最近收录", "release_new": "发行日期：新到旧", "release_old": "发行日期：旧到新", "duration": "时长：短到长", "duration_desc": "时长：长到短", "plays": "聆听次数", "hot": "平台热度"]
    var body: some View {
        let picker = ThemePicker(title, selection: $value) { ForEach(options, id: \.self) { Text(($0 == "default" ? defaultLabel : nil) ?? Self.labels[$0] ?? $0).tag($0) } }
        Group { if compact { picker.labelsHidden() } else { picker } }
            .frame(maxWidth: 300).controlSize(.small).accessibilityLabel(title).help(title)
    }
}

struct LibraryQueryControls: View {
    let store: MusicStore
    @Binding var values: [String: String]
    var grouped = false
    @StoredState private var providers = ResourceStore()
    private func field(_ key: String) -> Binding<String> {
        Binding(get: { values[key] ?? "" }, set: { values[key] = $0 })
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                TextField("搜索曲库", text: field("q")).textFieldStyle(.roundedBorder).frame(maxWidth: 340)
                Spacer()
                CatalogSortPicker(title: "排序", value: Binding(get: {values["sort"] ?? "default"}, set: {values["sort"] = $0}), options: grouped ? ["default","title","title_desc","added","release_new","release_old","plays"] : ["default","title","title_desc","artist","album","added","release_new","release_old","duration","duration_desc","plays"])
            }
            DisclosureGroup("筛选 \(values.filter { !["sort","q"].contains($0.key) && !$0.value.isEmpty }.count > 0 ? "· \(values.filter { !["sort","q"].contains($0.key) && !$0.value.isEmpty }.count)" : "")") {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 220), alignment: .leading)], alignment: .leading, spacing: 16) {
                    ThemePicker("媒体来源", selection: field("source")) {
                        Text("全部平台与文件").tag(""); Text("本地文件").tag("local")
                        ForEach(Array(providers.value["items"].array.filter { $0["capabilities"].array.contains { $0.text == "search" } }.enumerated()), id: \.offset) { _, item in Text(item["name"].text).tag(item["id"].text) }
                    }
                    choices("音频格式", "format", [("","全部格式")] + ["flac","mp3","aac","alac","wav","aiff","ogg","opus","dsf","dff","unknown"].map { ($0,$0 == "unknown" ? "未解析" : $0.uppercased()) })
                    if !grouped { choices("专辑资料", "album_status", [("","全部"),("known","已确认专辑"),("unassigned","未归属专辑")]) }
                    choices("聆听记录", "listening", [("","全部"),("unplayed","未听过"),("played","已听过")])
                    choices("播放列表", "playlist", [("","全部"),("in","在我的播放列表中"),("out","不在我的播放列表中")])
                    choices("封面资料", "artwork", [("","全部"),("with","已有封面地址"),("without","未提供封面")])
                    choices("最低采样率", "min_rate", [("","不限"),("44100","44.1 kHz"),("48000","48 kHz"),("96000","96 kHz"),("192000","192 kHz")])
                    choices("最低位深", "min_depth", [("","不限"),("16","16 bit"),("24","24 bit"),("32","32 bit")])
                    input("发行起始", "released_after", hint: "YYYY-MM-DD")
                    input("发行截止", "released_before", hint: "YYYY-MM-DD")
                    input("最短时长（秒）", "min_seconds", hint: "不限")
                    input("最长时长（秒）", "max_seconds", hint: "不限")
                    Button("重置筛选与排序") { values = [:] }
                }.padding(16).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 14))
            }.font(.callout)
        }.task(id: store.sessionID) { await providers.load("/plugins", store: store) }
    }
    private func choices(_ title: String, _ key: String, _ options: [(String,String)]) -> some View {
        ThemePicker(title, selection: field(key)) { ForEach(options, id: \.0) { Text($0.1).tag($0.0) } }
    }
    private func input(_ title: String, _ key: String, hint: String) -> some View {
        HStack { Text(title).foregroundStyle(.secondary); TextField(hint, text: field(key)).textFieldStyle(.roundedBorder) }
    }
}

extension Track {
    var releaseDate: String {
        let p = provider_data ?? .null
        return [p["catalog_display"]["release_date"].text, p["release_date"].text, p["provider_metadata"]["background"]["release_date"].text].first { !$0.isEmpty } ?? ""
    }
    var isNewRelease: Bool {
        Self.newRelease(releaseDate, now: Date())
    }
    static func newRelease(_ value: String, now: Date) -> Bool {
        guard value.count == 10, let date = ISO8601DateFormatter().date(from: value + "T00:00:00Z") else { return false }
        let day = floor(now.timeIntervalSince1970 / 86400) * 86400
        return date.timeIntervalSince1970 <= day && day - date.timeIntervalSince1970 < 30 * 86400
    }
}

struct NewMusicBadge: View {
    let date: String
    var album = false
    var body: some View {
        TimelineView(.periodic(from: .now, by: 3600)) { tick in
            if Track.newRelease(date, now: tick.date) {
                Text(album ? "新专辑" : "新歌").font(.system(size: 9, weight: .medium)).foregroundStyle(MusicStyle.accent)
                    .padding(.horizontal, 5).padding(.vertical, 2).overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(MusicStyle.accent.opacity(0.5)))
                    .help("发行于 \(date)，30天内\(album ? "新专辑" : "新歌")")
            }
        }
    }
}

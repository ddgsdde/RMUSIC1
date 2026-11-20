import SwiftUI

struct SmartDiscoveryView: View {
    let store: MusicStore
    @StoredState private var selected: Int?
    @StoredState private var refresh = UUID()
    private let names = ["还没认真听过", "久违的音乐", "高解析曲库", "我的收藏"]
    private let rules: [[String: JSONValue]] = [
        ["max_plays": .number(0), "sort": .string("recent_added"), "max_results": .number(100)],
        ["not_played_days": .number(30), "min_plays": .number(1), "sort": .string("least_played"), "max_results": .number(100)],
        ["min_sample_rate": .number(88200), "min_bit_depth": .number(24), "max_results": .number(100)],
        ["favorites_only": .bool(true), "sort": .string("recent"), "max_results": .number(100)]
    ]
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("从曲库发现").font(.headline)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 160))], spacing: 12) {
                ForEach(names.indices, id: \.self) { i in
                    Button { selected = i } label: { HStack { Image(systemName: ["sparkles", "clock.arrow.circlepath", "waveform", "heart"][i]); Text(names[i]); Spacer(); Image(systemName: "arrow.up.right").font(.caption) }.padding(16).background(MusicStyle.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 12)) }.buttonStyle(MusicButtonStyle())
                }
            }
        }.sheet(isPresented: Binding(get: { selected != nil }, set: { if !$0 { selected = nil } })) {
            if let selected { SmartPlaylistEditor(store: store, initialName: names[selected], initialRule: .object(rules[selected]), close: { self.selected = nil }, saved: { refresh = UUID() }) }
        }.task(id: refresh) { try? await store.collection.refresh(store) }
    }
}

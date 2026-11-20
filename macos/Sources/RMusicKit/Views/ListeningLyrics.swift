import SwiftUI

struct ListeningLyrics: View {
    let store: MusicStore
    @AppStorage("lyricLanguage") private var lyricLanguage = ""
    @AppStorage("lyricOffset") private var lyricOffset = 0.0
    @AppStorage("lyricTranslation") private var lyricTranslation = true
    @AppStorage("lyricRomanization") private var lyricRomanization = true
    @AppStorage("lyricMode") private var lyricMode = "bilingual"
    @AppStorage("lyricWords") private var lyricWords = true
    @AppStorage("playerLowPerformance") private var lowPerformance = false
    @AppStorage("lyricFontSize") private var lyricFontSize = 23.0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var phase
    var body: some View {
        TimelineView(.animation(minimumInterval: lowPerformance ? 1.0 / 12 : 1.0 / 30, paused: store.zone?.status != "playing" || phase == .background)) { _ in content }
    }
    private var content: some View {
        VStack(alignment: .leading, spacing: 10) {
          HStack {
            Toggle("显示汉译", isOn: Binding(get: { lyricTranslation && lyricMode != "original" }, set: { enabled in
                lyricTranslation = enabled
                if enabled && lyricMode == "original" { lyricMode = "bilingual" }
            })).toggleStyle(.switch).controlSize(.small).fixedSize()
            Spacer()
            Menu("歌词选项", systemImage: "textformat") {
                ThemePicker("歌词显示", selection: Binding(get: { displayMode }, set: { lyricMode = $0; lyricTranslation = $0 != "original" })) { Text("原文").tag("original"); Text("双语").tag("bilingual"); Text("译文").tag("translation") }
                Toggle("逐词高亮（音源提供时）", isOn: $lyricWords)
                Toggle("显示发音", isOn: $lyricRomanization)
                ThemePicker("翻译版本", selection: $lyricLanguage) { Text("默认").tag(""); ForEach(Array(store.details.document["translations"].array.enumerated()), id: \.offset) { _, item in Text(item["label"].text).tag(item["id"].text) } }
                ThemePicker("字号", selection: $lyricFontSize) { ForEach([18.0, 23, 28, 34], id: \.self) { Text("\(Int($0))").tag($0) } }
                Button("歌词提前 100 ms") { lyricOffset = max(-30000, lyricOffset - 100) }
                Button("歌词延后 100 ms") { lyricOffset = min(30000, lyricOffset + 100) }
                Button("重置歌词偏移（当前 \(Int(lyricOffset)) ms）") { lyricOffset = 0 }
            }.menuStyle(.borderlessButton).fixedSize()
          }
        ScrollViewReader { reader in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 28) {
                    if store.details.loading { ProgressView("正在载入歌词").font(.caption) }
                    else if let error = store.details.lyricError {
                        Text(error).font(.callout).foregroundStyle(.secondary)
                        Button("重试歌词") { store.details.retry(store) }
                    } else if store.details.lines.isEmpty {
                        Text("这首歌暂时没有歌词").font(.caption).foregroundStyle(.tertiary)
                    }
                    ForEach(store.details.lines) { line in
                        Button {
                            if line.time >= 0 { Task { await store.transport("seek", body: ["position": .number(max(0, line.time + offset))]) } }
                        } label: {
                            VStack(alignment: .leading, spacing: 10) {
                                if displayMode == "translation", let translation = line.translation { Text(translation).font(.system(size: lyricFontSize, weight: .bold)).lineSpacing(6) }
                                else { wordText(line).font(.system(size: lyricFontSize, weight: .bold)).lineSpacing(6) }
                                if displayMode == "bilingual", let translation = line.translation { Text(translation).font(.system(size: 14)).lineSpacing(4) }
                                if lyricRomanization, let romanization = line.romanization { Text(romanization).font(.system(size: 12)).foregroundStyle(.secondary) }
                            }.foregroundStyle(active == line.id ? Color.primary : Color.secondary.opacity(0.6))
                                .scaleEffect(active == line.id || reduceMotion || lowPerformance ? 1 : 0.96, anchor: .leading)
                                .frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 5)
                        }.buttonStyle(.plain).disabled(line.time < 0 || !store.canPlay).id(line.id)
                    }
                }.padding(.vertical, 120).padding(.horizontal, 8)
            }.scrollIndicators(.hidden)
                .onChange(of: active) { if let active { withAnimation(reduceMotion || lowPerformance ? nil : .smooth(duration: 0.3)) { reader.scrollTo(active, anchor: .center) } } }
                .onAppear { if let active { reader.scrollTo(active, anchor: .center) } }
                .animation(reduceMotion || lowPerformance ? nil : .easeOut(duration: 0.2), value: active)
        }
        }.onChange(of: lyricLanguage) { store.details.selectLanguage(lyricLanguage) }
    }
    private var displayMode: String { lyricTranslation ? lyricMode : "original" }
    private func wordText(_ line: LyricLine) -> Text {
        guard lyricWords, !line.words.isEmpty, active == line.id else { return Text(line.text.isEmpty ? "♪" : line.text) }
        let position = store.presentationClock.reading(store.zone).position - offset
        var text = AttributedString()
        for word in line.words { var part = AttributedString(word.text); part.foregroundColor = word.time <= position ? Color.primary : Color.secondary.opacity(0.5); text.append(part) }
        return Text(text)
    }
    private var offset: Double { lyricOffset / 1000 + store.details.document["offset_ms"].number / 1000 }
    private var active: Int? { store.details.active(at: store.presentationClock.reading(store.zone).position - offset) }
}

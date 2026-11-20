import SwiftUI
import Charts

struct ListeningStatsView: View {
    let store: MusicStore
    @StoredState private var resource = ResourceStore()
    @StoredState private var refreshID = UUID()
    @SceneStorage("rmusic.statistics.period") private var period = 1
    @SceneStorage("rmusic.statistics.ranking") private var ranking = "tracks"
    @SceneStorage("rmusic.statistics.page") private var page = "overview"
    private var timezone: String { resource.value["timezone"].string ?? "Asia/Shanghai" }
    private var summary: JSONValue { resource.value["summary"] }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                ThemePicker("聆听统计页面", selection: $page) { Text("聆听概览").tag("overview"); Text("聆听历史").tag("history") }.pickerStyle(.segmented).frame(maxWidth: 300)
                if page == "history" {
                    Text("聆听历史").font(.title2.bold())
                    ListeningHistoryView(store: store, timezone: timezone)
                } else {
                HStack {
                    ThemePicker("统计周期", selection: $period) { Text("今天").tag(1); Text("7 天").tag(7); Text("30 天").tag(30); Text("90 天").tag(90); Text("一年").tag(365); Text("历史所有").tag(0) }.pickerStyle(.segmented).frame(maxWidth: 560)
                    Spacer()
                    Button("刷新统计", systemImage: "arrow.clockwise") { refreshID = UUID() }.labelStyle(.iconOnly).disabled(resource.loading)
                }
                ResourceNotice(resource: resource)
                if resource.value["legacy_utc_seconds"].number > 0 { Text("旧记录按原有日期保留，新记录采用核心的日期设置。").font(.caption).foregroundStyle(.secondary) }
                if resource.loading && resource.value["summary"].object.isEmpty { ProgressView().frame(maxWidth: .infinity).padding(40) }
                else {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 220), spacing: 14)], spacing: 14) {
                        metric("聆听时长", listeningTime(summary["listened_seconds"].number), "headphones")
                        metric("有效播放", "\(summary["play_count"].integer) 次", "play.circle")
                        metric("听过歌曲", "\(summary["unique_tracks"].integer) 首", "music.note")
                        metric("听过专辑", "\(summary["unique_albums"].integer) 张", "square.stack")
                        metric("艺术家", "\(summary["unique_artists"].integer) 位", "person")
                        metric("聆听天数", "\(summary["active_days"].integer) 天", "calendar")
                    }
                    if summary["listened_seconds"].number == 0 {
                        ContentUnavailableView("这段时间还没有聆听记录", systemImage: "headphones", description: Text("播放后会按实际聆听时长更新。"))
                    } else {
                        HStack(spacing: 24) {
                            Label("聆听日均 \(listeningTime(summary["listened_seconds"].number / Double(max(1, summary["active_days"].integer))))", systemImage: "clock")
                            Label("最长连续 \(summary["longest_streak"].integer) 天", systemImage: "flame")
                            Label("跳过 \(summary["skip_count"].integer) 次", systemImage: "forward.end")
                        }.font(.caption).foregroundStyle(.secondary)
                        if period != 1 { ListeningTrend(days: resource.value["days"].array, bucketDays: max(1, resource.value["chart_bucket_days"].integer)) }
                        if !resource.value["sources"].array.isEmpty { sources }
                        HStack { Text("常听排行").font(.headline); Spacer(); ThemePicker("排行", selection: $ranking) { Text("歌曲").tag("tracks"); Text("专辑").tag("albums"); Text("艺术家").tag("artists") }.pickerStyle(.segmented).frame(width: 250) }
                        LazyVStack(spacing: 0) {
                            ForEach(Array(resource.value[ranking].array.enumerated()), id: \.offset) { index, item in
                                StatsRankingRow(item: item, index: index, kind: ranking, maximum: max(1, resource.value[ranking].array.first?["listened_seconds"].number ?? 1), store: store)
                                Divider().opacity(0.5)
                            }
                        }
                    }

                }
                }
            }.padding(28)
        }.rememberPageScroll(store, key: "stats:\(page):\(period)").task(id: "\(store.listeningStatsRefreshID):\(page):\(period):\(refreshID)") {
            guard page == "overview" else { await resource.load("/stats?period_days=1", store: store); return }
            await resource.followListeningStats(apiQuery("/stats", ["period_days": String(period)]), store: store)
        }
    }
    private var sources: some View {
        FeatureCard {
            Text("音源分布").font(.headline)
            ForEach(Array(resource.value["sources"].array.enumerated()), id: \.offset) { _, source in
                HStack(spacing: 16) {
                    Text(sourceName(source["source"].text)).frame(width: 100, alignment: .leading)
                    ProgressView(value: max(0, source["listened_seconds"].number), total: max(1, summary["listened_seconds"].number)).tint(MusicStyle.accent)
                    Text(listeningTime(source["listened_seconds"].number)).monospacedDigit().frame(width: 90, alignment: .trailing)
                }.font(.caption)
            }
        }
    }
    private func metric(_ name: String, _ value: String, _ icon: String) -> some View {
        FeatureCard { Label(name, systemImage: icon).font(.caption).foregroundStyle(.secondary); Text(value).font(.system(size: 25, weight: .semibold, design: .rounded)).monospacedDigit() }
    }
    private func sourceName(_ id: String) -> String { ["local": "本地音乐", "qqmusic": "QQ 音乐", "unknown": "来源未记录"][id] ?? id }
}

private struct ListeningTrend: View {
    let days: [JSONValue]
    let bucketDays: Int
    @StoredState private var selected: String?
    var body: some View {
        FeatureCard {
            HStack { Text(bucketDays > 1 ? "聆听轨迹 · 每\(bucketDays)天汇总" : "每日聆听").font(.headline); Spacer(); if let day = days.first(where: { $0["day"].text == selected }) { Text("\(day["day"].text) · \(listeningTime(day["listened_seconds"].number))").font(.caption).foregroundStyle(.secondary) } }
            Chart(Array(days.enumerated()), id: \.offset) { _, day in
                BarMark(x: .value("日期", day["day"].text), y: .value("分钟", day["listened_seconds"].number / 60)).foregroundStyle(MusicStyle.accent.gradient).cornerRadius(3)
                if let selected, selected == day["day"].text { RuleMark(x: .value("日期", selected)).foregroundStyle(.secondary.opacity(0.3)) }
            }.chartXSelection(value: $selected).chartXAxis { AxisMarks(values: .automatic(desiredCount: 6)) { value in AxisValueLabel { if let date = value.as(String.self) { Text(bucketDays > 1 ? String(date.prefix(7)) : String(date.suffix(5))).font(.caption2) } } } }.chartYAxisLabel("分钟").frame(height: 180)
        }
    }
}

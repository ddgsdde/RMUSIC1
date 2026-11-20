import SwiftUI

struct PlayerModulesView<Lyrics: View>: View {
  let store: MusicStore
  let lyrics: Lyrics
  @AppStorage("nativePlayerLayout") private var saved = PlayerLayout.defaultJSON
  @StoredState private var editing = false
  @StoredState private var queueOffset = 0
  private var layout: PlayerLayout { PlayerLayout.read(saved) }
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      HStack {
        Text("我的播放页").font(.headline)
        Spacer()
        Button("调整布局", systemImage: "slider.horizontal.3") { editing = true }
      }
      GeometryReader { geometry in
        ScrollView {
          HStack(alignment: .top, spacing: 24) {
            ForEach(0..<(geometry.size.width >= 700 ? 2 : 1), id: \.self) { column in
              LazyVStack(spacing: 20) {
                ForEach(
                  layout.items.filter {
                    $0.visible && (geometry.size.width < 700 || $0.column == column)
                  }
                ) { item in
                  FeatureCard {
                    Text(item.module.title).font(.caption).foregroundStyle(.secondary)
                    module(item.module).frame(maxWidth: .infinity).frame(height: item.height)
                  }
                }
              }.frame(maxWidth: .infinity)
            }
          }
        }
      }
    }.sheet(isPresented: $editing) { editor }
  }
  @ViewBuilder private func module(_ kind: PlayerModule) -> some View {
    switch kind {
    case .cover:
      ArtworkView(track: store.zone?.track, store: store, radius: 18, resolveMissing: true).aspectRatio(
        1, contentMode: .fit)
    case .info:
      VStack(alignment: .leading, spacing: 12) {
        Text(store.zone?.track?.title ?? "尚未播放").font(.title2.bold())
        Group { if let track = store.zone?.track { CatalogTextLink(track: track, store: store) } }.foregroundStyle(.secondary)
        Group { if let track = store.zone?.track { CatalogTextLink(track: track, store: store, album: true) } }.font(.callout)
        Text(store.zone?.track?.quality ?? "").font(.caption).foregroundStyle(.secondary)
      }.frame(maxWidth: .infinity, alignment: .leading)
    case .controls:
      HStack(spacing: 30) {
        Button("上一首", systemImage: "backward.end.fill") { transport("previous") }
        Button(
          store.zone?.status == "playing" ? "暂停" : "播放",
          systemImage: store.zone?.status == "playing" ? "pause.fill" : "play.fill"
        ) { transport(store.zone?.status == "playing" ? "pause" : "play") }
        Button("下一首", systemImage: "forward.end.fill") { transport("next") }
      }.labelStyle(.iconOnly).font(.title2).disabled(!store.canPlay)
      Text(
        "\(timeLabel(store.zone?.position ?? 0)) / \(timeLabel(store.zone?.track?.duration ?? 0))"
      ).monospacedDigit().font(.caption)
    case .lyrics: lyrics
    case .queue:
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 12) {
          ForEach(
            Array((store.zone?.queue ?? []).dropFirst(queueOffset).prefix(50).enumerated()),
            id: \.offset
          ) { local, track in
            Button {
              Task {
                await store.transport(
                  "play", body: ["queue_index": .number(Double(queueOffset + local))])
              }
            } label: {
              HStack {
                Text("\(queueOffset + local + 1)").monospacedDigit().foregroundStyle(.secondary)
                Text(track.title).lineLimit(1)
                Spacer()
                if store.zone?.queue_index == queueOffset + local {
                  Image(systemName: "waveform").foregroundStyle(MusicStyle.accent)
                }
              }
            }.buttonStyle(.plain).disabled(!store.canPlay)
          }
        }
      }
      OffsetPageControls(offset: $queueOffset, total: store.zone?.queue.count ?? 0, limit: 50)
    case .zones:
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 12) {
          ForEach(store.playbackZones) { zone in
            Button {
              store.selectPlaybackZone(zone.id)
            } label: {
              HStack {
                Image(
                  systemName: zone.id == store.zone?.id ? "checkmark.circle.fill" : "hifispeaker")
                Text(zone.name)
                Spacer()
                Text(zone.status == "playing" ? "播放中" : zone.status == "paused" ? "暂停" : "停止").font(
                  .caption
                ).foregroundStyle(.secondary)
              }
            }.buttonStyle(.plain)
          }
        }
      }
    case .signal:
      let presentation = SignalPresentation(
        zone: store.zone, endpoint: store.signalEndpoint, info: .null)
      ScrollView {
        VStack(alignment: .leading, spacing: 14) {
          ForEach(presentation.stages) { stage in
            HStack(alignment: .top) {
              Image(systemName: stage.icon).foregroundStyle(
                stage.processed ? .orange : MusicStyle.accent
              ).frame(width: 24)
              VStack(alignment: .leading, spacing: 4) {
                Text(stage.name + " · " + stage.status).font(.callout.bold())
                Text(stage.detail).font(.caption).foregroundStyle(.secondary)
              }
            }
          }
        }
      }
    case .meter:
      let playback = store.signalEndpoint["playback"]
      if playback["core_playback_id"].string == store.zone?.playback_id,
        playback["track_id"].string == store.zone?.track?.id,
        playback["meter"]["peak"].number.isFinite, playback["meter"].object["peak"] != nil
      {
        VStack(alignment: .leading, spacing: 12) {
          ProgressView(value: min(1, max(0, playback["meter"]["peak"].number)))
          Text(
            playback["meter"]["peak"].number > 0
              ? "峰值 \((20 * log10(playback["meter"]["peak"].number)).formatted(.number.precision(.fractionLength(1)))) dBFS"
              : "峰值 −∞ dBFS"
          ).font(.caption).monospacedDigit()
          if playback["meter"]["clipped_samples"].integer > 0 {
            Text("削波样本 \(playback["meter"]["clipped_samples"].integer)").foregroundStyle(.orange)
          }
        }
      } else {
        Text("当前播放端尚未报告电平").font(.caption).foregroundStyle(.secondary)
      }
    }
  }
  private func transport(_ action: String) { Task { await store.transport(action) } }
  private var editor: some View {
    VStack(alignment: .leading, spacing: 18) {
      HStack {
        Text("播放页布局").font(.title2.bold())
        Spacer()
        Button("完成") { editing = false }
      }
      ForEach(Array(layout.items.enumerated()), id: \.element.id) { index, item in
        HStack {
          Toggle(
            item.module.title,
            isOn: Binding(
              get: { layout.items[index].visible },
              set: { value in change(index) { $0.visible = value } })
          ).frame(width: 120)
          ThemePicker(
            "位置",
            selection: Binding(
              get: { layout.items[index].column },
              set: { value in change(index) { $0.column = value } })
          ) {
            Text("左列").tag(0)
            Text("右列").tag(1)
          }.labelsHidden().frame(width: 100)
          TextField(
            "高度",
            value: Binding(
              get: { layout.items[index].height },
              set: { value in
                if value.isFinite { change(index) { $0.height = min(800, max(120, value)) } }
              }), format: .number
          ).frame(width: 75)
          Text("pt").font(.caption).foregroundStyle(.secondary)
          Button("上移 \(item.module.title)", systemImage: "arrow.up") {
            var next = layout
            next.move(index, by: -1)
            saved = next.encoded
          }.labelStyle(.iconOnly).disabled(index == 0)
          Button("下移 \(item.module.title)", systemImage: "arrow.down") {
            var next = layout
            next.move(index, by: 1)
            saved = next.encoded
          }.labelStyle(.iconOnly).disabled(index + 1 == layout.items.count)
        }
      }
      Button("恢复默认布局") { saved = PlayerLayout.defaultJSON }
    }.padding(24).frame(width: 650)
  }
  private func change(_ index: Int, _ transform: (inout PlayerLayout.Item) -> Void) {
    var next = layout
    guard next.items.indices.contains(index) else { return }
    transform(&next.items[index])
    saved = next.encoded
  }
}

import SwiftUI

struct VolumeControlView: View {
    let store: MusicStore
    var interactionChanged: (Bool) -> Void = { _ in }
    @StoredState private var control = VolumeControlState()
    @Environment(\.scenePhase) private var phase
    private var context: String { "\(store.volumeTargetID):\(phase == .active):\(store.canMuteLocalDevice)" }
    private var muted: Bool {
        switch store.volumeMode {
        case .device, .fixed: store.deviceVolumeMuted == true
        case .software: store.zone?.muted == true
        }
    }
    private func submit(_ value: Double) {
        let music = store, owner = store.volumeTargetID
        control.submit(value) { [weak music] next in
            guard let music, music.volumeTargetID == owner else { return }
            await music.setVolume(next)
        }
    }
    var body: some View {
        HStack(spacing: 8) {
            Menu {
                Text(store.volumeLabel)
                if store.volumeMode == .device { Text(muted ? "设备已静音，调到非零音量可恢复" : "CoreAudio 设备控件") }
                if store.volumeMode == .software || store.canMuteLocalDevice {
                    Button(muted ? "取消静音" : "静音") {
                        let target = !muted
                        Task { await store.setMuted(target) }
                    }.disabled(store.hardwareVolumeWriting || (store.volumeMode != .software && store.deviceVolumeMuted == nil))
                }
                if store.outputs.output(for: store.zone) != nil {
                    Divider()
                    if store.outputs.output(for: store.zone)?.device_volume?.supported == true {
                        Button("设备音量") { Task { await store.setVolumeMode(.device) } }
                    }
                    Button("固定最大") { Task { await store.setVolumeMode(.fixed) } }
                    Button("软件音量") { Task { await store.setVolumeMode(.software) } }
                }
            } label: { Image(systemName: muted ? "speaker.slash" : store.volumeMode == .fixed || store.volumeValue == 0 ? "speaker" : "speaker.wave.2") }.menuStyle(.borderlessButton).fixedSize().help(store.volumeLabel)
            Slider(value: Binding(get: { min(1, max(0, control.draft ?? store.volumeValue)) }, set: { value in
                submit(value)
            }), in: 0...1) { editing in
                if editing { control.editing = true } else { control.finishEditing() }
            }.frame(width: 82).controlSize(.mini).accessibilityLabel(store.volumeLabel)
                .disabled(store.volumeMode == .fixed || !store.canPlay)
                .help(store.volumeMode == .fixed ? "音量控制已关闭；可在菜单中选择设备或软件音量" : store.volumeMode == .device ? "调节所选设备的音量控件" : "软件增益会处理 PCM 样本")
        }.background(VolumeWheelInput(enabled: store.volumeMode != .fixed && store.canPlay) { delta, precise in
            submit(wheelVolume(control.draft ?? store.volumeValue, delta: delta, precise: precise))
        }).overlay(alignment: .top) {
            if let level = control.draft { Text("\(Int(level * 100))%").font(.system(size: 10).monospacedDigit()).padding(.horizontal, 8).padding(.vertical, 5).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 7)).offset(y: -30).allowsHitTesting(false) }
        }.help("音量 \(Int((control.draft ?? store.volumeValue) * 100))% · 悬停滚轮调节")
        .onChange(of: context) { control.reset() }
            .onChange(of: control.draft != nil || control.editing || control.writing) { _, value in interactionChanged(value) }
            .onDisappear { control.reset(); interactionChanged(false) }
            .task(id: context) {
                guard phase == .active, store.connected, store.outputs.output(for: store.zone) != nil,
                      store.volumeMode == .device || store.canMuteLocalDevice else { return }
                while !Task.isCancelled {
                    try? await store.readLocalVolume()
                    do { try await Task.sleep(for: .milliseconds(500)) } catch { return }
                }
            }
    }
}

import SwiftUI
import UniformTypeIdentifiers

struct DspWorkspaceView: View {
    let store: MusicStore
    var requestedZone: String?
    @StoredState private var chosen = ""
    private var zone: Zone? {
        store.playbackZones.first { $0.id == chosen } ?? store.playbackZones.first { $0.id == store.selectedZoneID } ?? store.playbackZones.first
    }
    var body: some View {
        ScrollView {
            VStack(alignment:.leading,spacing:22) {
                HStack(alignment:.center,spacing:16) {
                    Text("DSP 工作区").font(.system(size:30,weight:.semibold)).tracking(-0.8)
                    Spacer()
                    if let zone {
                        Picker("播放设备", selection:Binding(get:{ zone.id },set:{ chosen = $0 })) {
                            ForEach(store.playbackZones) { entry in Text(store.playbackOutputLabel(entry)).tag(entry.id) }
                        }.labelsHidden().frame(maxWidth:280,alignment:.trailing)
                    }
                }
                if let zone {
                    let local = store.outputs.output(for:zone)
                    let viewZone = local.map {store.effects.zone(for:$0,base:zone)} ?? zone
                    PlaybackSettingsView(store:store,zone:viewZone,localOutput:local).id(zone.id)
                } else {
                    ContentUnavailableView("请选择播放设备",systemImage:"hifispeaker",description:Text("设备连接后即可配置 DSP。"))
                }
            }.padding(24)
        }.scrollIndicators(.hidden)
        .onChange(of:requestedZone,initial:true) { if let requestedZone { chosen = requestedZone } }
    }
}

struct PlaybackSettingsView: View {
    let store: MusicStore
    let zone: Zone
    var localOutput: NativeOutput? = nil

    private enum Module: String, CaseIterable {
        case eq, loudness, fades, gain, rate, stereo, convolution
        var name: String {
            switch self {
            case .eq: "参数均衡"
            case .gain: "增益与余量"
            case .rate: "采样率转换"
            case .stereo: "立体声"
            case .convolution: "卷积"
            case .loudness: "响度均衡"
            case .fades: "淡入淡出"
            }
        }
        var icon: String {
            switch self {
            case .eq: "waveform.path"
            case .gain: "gauge.with.dots.needle.50percent"
            case .rate: "waveform.path.ecg"
            case .stereo: "speaker.wave.2"
            case .convolution: "waveform"
            case .loudness: "speaker.wave.3"
            case .fades: "chart.line.uptrend.xyaxis"
            }
        }
    }

    private struct Slot: Hashable {
        let module: Module
        var index = 0
    }
    @StoredState private var slot = Slot(module:.eq)
    @StoredState private var modular = false
    @StoredState private var draft = PlaybackSettingsDraft()
    @StoredState private var saved = PlaybackSettingsDraft()
    @StoredState private var selectedBand: UUID?
    @StoredState private var importing = false
    @StoredState private var advanced = false
    @StoredState private var impulses: [JSONValue] = []
    @StoredState private var working = false
    @StoredState private var message: String?
    @StoredState private var failed = false
    @StoredState private var loadingResponses = false
    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var endpoint: JSONValue { store.endpointSnapshot.first{$0["id"].string == zone.endpoint_id} ?? .null }
    private var coreDSP: Bool { endpoint["capabilities"]["core_pcm_dsp_v1"].boolean }
    private var portable: Bool { zone.id.hasPrefix("travel-") }
    private var supported: Bool { portable || endpoint["capabilities"]["dsp"] != .bool(false) }
    private var ready: Bool { (store.coreReachable || portable) && !working }
    private var rate: Int { draft.sampleRate > 0 ? draft.sampleRate : max(8000, zone.track?.sample_rate ?? 48000) }
    private var dirty: Bool { (try? draft.payload(rate:rate)) != (try? saved.payload(rate:rate)) }
    private var available: [Module] {
        Module.allCases.filter { portable ? ($0 == .loudness || $0 == .fades) : ($0 != .rate || coreDSP) && ($0 != .stereo || advanced) }
    }
    private var present: Set<String> { draft.modules(portable:portable) }
    private var stages: [PlaybackSettingsDraft.EQStage] { draft.allEQ }
    private var slots: [Slot] {
        available.filter { present.contains($0.rawValue) }.flatMap { kind in
            (0..<(kind == .eq && modular ? stages.count : 1)).map { Slot(module:kind,index:$0) }
        }
    }
    private var addable: [Module] {
        available.filter { !present.contains($0.rawValue) || $0 == .eq && modular && stages.count < 4 }
    }
    private var selection: Slot? { slots.contains(slot) ? slot : slots.first }
    private var activeModule: Module { selection?.module ?? .eq }
    private var activeEQ: Int { activeModule == .eq ? selection?.index ?? 0 : 0 }
    private var eq: PlaybackSettingsDraft.EQStage { stages[activeEQ] }
    private var bandBinding: Binding<[PlaybackSettingsDraft.Band]> {
        Binding(get:{eq.bands},set:{ value in draft.editEQ(at:activeEQ) { $0.bands = value } })
    }
    private var bypassBinding: Binding<Bool> {
        Binding(get:{eq.bypass},set:{ value in draft.editEQ(at:activeEQ) { $0.bypass = value } })
    }
    private var chosenIndex: Int? { eq.bands.firstIndex { $0.id == selectedBand } }
    private var savedResponseIDs: [String] { draft.original["convolution"]?["response_ids"].array.compactMap(\.string) ?? [] }
    private func title(for entry: Slot) -> String {
        entry.module == .eq && stages.count > 1 ? "\(entry.module.name) \(entry.index+1)" : entry.module.name
    }

    var body: some View {
        VStack(alignment:.leading,spacing:16) {
            ViewThatFits(in:.horizontal) {
                HStack(spacing:16) { scope; Spacer(minLength:12); actions }
                VStack(alignment:.leading,spacing:12) { scope; actions }
            }.padding(16)
                .background(surface,in:RoundedRectangle(cornerRadius:20))
                .overlay(frame)
            if let message {
                Label(message,systemImage:failed ? "exclamationmark.circle" : "checkmark.circle")
                    .font(.caption).foregroundStyle(failed ? Color.orange : .secondary)
                    .accessibilityAddTraits(.updatesFrequently)
            }
            ViewThatFits(in:.horizontal) {
                HStack(alignment:.top,spacing:16) {
                    VStack(spacing:6) { moduleList }.frame(width:180)
                    inspector.frame(minWidth:390)
                }
                VStack(alignment:.leading,spacing:14) {
                    ScrollView(.horizontal) { HStack(spacing:6) { moduleList } }.scrollIndicators(.hidden)
                    inspector
                }
            }
        }
        .onAppear { reset() }
        .task(id:"\(zone.endpoint_id):\(coreDSP)") {
            guard coreDSP else { advanced = false; modular = false; return }
            let status = try? await store.api("/status")
            if !Task.isCancelled {
                advanced = status?["capabilities"]["dsp_workbench_v1"].boolean == true
                modular = status?["capabilities"]["dsp_modules_v1"].boolean == true
            }
        }
        .task(id:activeModule) { if activeModule == .convolution { await loadResponses() } }
        .fileImporter(isPresented:$importing,allowedContentTypes:[.plainText,.json,.data]) { result in
            if case .success(let url) = result { importPreset(url) }
        }
    }

    private var scope: some View {
        HStack(spacing:11) {
            Image(systemName:portable ? "laptopcomputer" : "slider.horizontal.3")
                .font(.system(size:16,weight:.medium)).foregroundStyle(MusicStyle.accent)
                .frame(width:38,height:38).background(MusicStyle.accent.opacity(0.12),in:RoundedRectangle(cornerRadius:12))
            VStack(alignment:.leading,spacing:4) {
                Text(portable ? "本机外出播放" : coreDSP ? "Core 音频处理" : "播放设备音频处理")
                    .font(.subheadline.weight(.semibold))
                if portable { Text("响度和淡入淡出在此设备处理，从下一首开始应用。").font(.caption).foregroundStyle(.secondary) }
            }
        }
    }

    private var actions: some View {
        HStack(spacing:9) {
            if supported && !portable { Toggle("启用 DSP",isOn:$draft.enabled).toggleStyle(.switch).controlSize(.small).fixedSize() }
            Button("还原",systemImage:"arrow.uturn.backward") { draft = saved; if let selection { choose(selection) }; message = nil }
                .labelStyle(.iconOnly).buttonStyle(MusicControlStyle()).help("还原未保存的更改").disabled(!dirty || working)
            Button(working ? "正在保存…" : portable ? "保存到本机" : "应用到音区") { apply() }
                .buttonStyle(WorkspacePrimaryButton()).fixedSize().disabled(!dirty || !ready)
        }
    }

    @ViewBuilder private var moduleList: some View {
        Menu { ForEach(addable,id:\.self) { item in Button(item.name) { addModule(item) } } }
        label: { Label("添加模块",systemImage:"plus").font(.subheadline.weight(.medium)).foregroundStyle(MusicStyle.accent) }
        .disabled(addable.isEmpty || working).padding(.vertical,5)
        ForEach(slots,id:\.self) { entry in
            let selected = selection == entry
            Button { withAnimation(reduceMotion ? nil : .easeInOut(duration:0.16)) { choose(entry) } } label: {
                HStack(spacing:10) {
                    Image(systemName:entry.module.icon).font(.system(size:14,weight:.medium))
                        .foregroundStyle(selected ? MusicStyle.accent : .secondary).frame(width:20)
                    VStack(alignment:.leading,spacing:4) {
                        Text(title(for:entry)).font(.subheadline.weight(.semibold)).lineLimit(1)
                        Text(entry.module == .eq ? eqSummary(entry.index) : summary(entry.module))
                            .font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer(minLength:4)
                    if isActive(entry) { Circle().fill(MusicStyle.accent).frame(width:6,height:6).accessibilityHidden(true) }
                }.padding(.horizontal,11).padding(.vertical,10).frame(maxWidth:.infinity,alignment:.leading)
                    .background(selected ? MusicStyle.accent.opacity(0.14) : .clear,in: RoundedRectangle(cornerRadius:12))
                    .contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityAddTraits(selected ? .isSelected : []).disabled(working)
        }
        ForEach([Module.loudness,.fades].filter { addable.contains($0) },id:\.self) { item in
            Button { addModule(item) } label: { Label(item.name,systemImage:"plus.circle").font(.caption) }
                .buttonStyle(MusicControlStyle()).foregroundStyle(.secondary).disabled(working)
                .accessibilityLabel("添加\(item.name)")
        }
    }

    private var inspector: some View {
        VStack(alignment:.leading,spacing:16) {
            if let selection {
                moduleHeader(selection)
                if supported || activeModule == .fades { panel }
                else { Text("此输出的 DSP 由设备管理。").foregroundStyle(.secondary) }
            } else {
                Text("从「添加模块」开始配置音频处理。").foregroundStyle(.secondary)
            }
        }.id(selection).transition(.opacity)
            .padding(18).frame(maxWidth:.infinity,alignment:.leading)
            .background(surface,in:RoundedRectangle(cornerRadius:20)).overlay(frame)
    }

    private func moduleHeader(_ entry:Slot) -> some View {
        HStack {
            Text(title(for:entry)).font(.title3.weight(.semibold))
            Spacer()
            if entry.module == .eq && supported {
                Menu("均衡预设") {
                    Button("十段均衡") { tenBandPreset() }
                    Button("从文件导入…") { importing = true }.disabled(working || !store.coreReachable)
                }.fixedSize()
                Button("添加频段",systemImage:"plus") { add(1000,0) }.disabled(eq.bands.count >= 32 || draft.totalBands >= 64)
                if stages.count > 1 {
                    Button("上移 EQ",systemImage:"arrow.up") { moveEQ(-1) }.labelStyle(.iconOnly).disabled(activeEQ == 0 || working)
                    Button("下移 EQ",systemImage:"arrow.down") { moveEQ(1) }.labelStyle(.iconOnly).disabled(activeEQ == stages.count-1 || working)
                }
            }
            Button("移除模块",systemImage:"trash") { removeModule(entry) }.labelStyle(.iconOnly).disabled(working)
        }
    }
    private func choose(_ entry:Slot) {
        slot = entry
        selectedBand = entry.module == .eq ? stages[entry.index].bands.first?.id : nil
    }
    private func addModule(_ kind:Module) {
        if present.contains(kind.rawValue) {
            guard kind == .eq, modular, stages.count < 4 else { return }
            let index = stages.count
            draft.extraEQ.append(.init())
            choose(Slot(module:.eq,index:index))
        } else {
            draft.installedModules = present.union([kind.rawValue]).sorted()
            choose(Slot(module:kind))
        }
    }
    private func removeModule(_ entry:Slot) {
        if entry.module == .eq && stages.count > 1 {
            var remaining = stages
            remaining.remove(at:entry.index); draft.allEQ = remaining
            choose(Slot(module:.eq,index:max(0,entry.index-1)))
            return
        }
        draft.installedModules = present.subtracting([entry.module.rawValue]).sorted()
        switch entry.module {
        case .eq: draft.allEQ = []
        case .rate: draft.sampleRate = 0
        case .stereo: draft.stereo = nil
        case .gain: draft.preamp = 0; draft.balance = 0
        case .convolution: draft.original["convolution"] = nil
        case .loudness: draft.normalize = false; draft.target = -14
        case .fades: draft.fadeIn = 0; draft.fadeOut = 0
        }
        if let first = slots.first { choose(first) } else { selectedBand = nil }
    }
    private func moveEQ(_ step:Int) {
        let to = activeEQ + step
        var entries = stages
        guard entries.indices.contains(to) else { return }
        entries.swapAt(activeEQ,to); draft.allEQ = entries
        slot = Slot(module:.eq,index:to)
    }
    private func isActive(_ entry:Slot) -> Bool {
        guard entry.module == .eq else { return active(entry.module) }
        let stage = stages[entry.index]
        return draft.enabled && !stage.bypass && stage.bands.contains(where:\.enabled)
    }
    private var surface: Color { MusicStyle.panel(scheme).opacity(0.92) }
    private var frame: some View { RoundedRectangle(cornerRadius:20).strokeBorder(.primary.opacity(0.08)).allowsHitTesting(false) }
    private func eqSummary(_ index: Int) -> String {
        if stages[index].bypass { return "旁路" }
        let count = stages[index].bands.filter(\.enabled).count
        return count > 0 ? "\(count) 段" : "未配置"
    }
    private func summary(_ entry:Module) -> String {
        switch entry {
        case .eq: "\(draft.bands.filter(\.enabled).count) 段"
        case .gain: "\(draft.preamp.formatted()) dB"
        case .rate: draft.sampleRate == 0 ? "跟随音源" : "\((Double(draft.sampleRate)/1000).formatted()) kHz"
        case .stereo: draft.stereo == nil ? "原始声道" : "已配置"
        case .convolution: savedResponseIDs.isEmpty ? "未选响应" : "\(savedResponseIDs.count) 组响应"
        case .loudness: draft.normalize ? "\(draft.target.formatted()) LUFS" : "关闭"
        case .fades: draft.fadeIn == 0 && draft.fadeOut == 0 ? "关闭" : "\(draft.fadeIn) / \(draft.fadeOut) ms"
        }
    }
    private func active(_ entry:Module) -> Bool {
        switch entry {
        case .eq: draft.enabled && !draft.eqBypass && draft.bands.contains(where:\.enabled)
        case .gain: draft.enabled && (draft.preamp != 0 || draft.balance != 0)
        case .rate: draft.enabled && draft.sampleRate > 0
        case .stereo: draft.enabled && draft.stereo != nil
        case .convolution: draft.enabled && !savedResponseIDs.isEmpty
        case .loudness: (draft.enabled || portable) && draft.normalize
        case .fades: draft.fadeIn != 0 || draft.fadeOut != 0
        }
    }
    @ViewBuilder private var panel: some View {
        switch activeModule {
        case .eq: equalizer
        case .gain:
            number("前级增益 (dB)",$draft.preamp)
            HStack { Text("声道平衡"); Slider(value:$draft.balance,in:-1...1); Text("\(Int(draft.balance*100))%").monospacedDigit() }
            let peak = EqualizerResponse.peak(bands:stages.filter{!$0.bypass}.flatMap(\.bands),rate:rate)
            Text("前级与 EQ 的预估峰值 \((peak+draft.preamp).formatted(.number.precision(.fractionLength(1)))) dB。实际余量还受音源、响度和卷积影响。").font(.caption).foregroundStyle(.secondary)
            Button("为 EQ 预留约 1 dB") { draft.preamp = max(-60,floor(-peak-1)); draft.enabled = true }
        case .rate:
            Picker("目标采样率",selection:$draft.sampleRate) {
                Text("保持源采样率").tag(0)
                ForEach(PlaybackSettingsDraft.sampleRates,id:\.self) { rate in Text("\((Double(rate)/1000).formatted()) kHz").tag(rate) }
            }.frame(maxWidth:350)
            Text("由 Core 升频或降频，设备格式上限优先。").font(.caption).foregroundStyle(.secondary)
        case .stereo:
            Toggle("启用立体声处理",isOn:Binding(get:{draft.stereo != nil},set:{draft.stereo = $0 ? .init() : nil}))
            if draft.stereo != nil {
                let stereo = Binding(get:{draft.stereo ?? .init()},set:{draft.stereo = $0})
                HStack { Text("耳机串音"); Slider(value:stereo.crossfeed,in:0...0.5); Text("\(Int(stereo.wrappedValue.crossfeed*100))%") }
                number("串音截止频率 (Hz)",stereo.crossover)
                Toggle("合并为单声道",isOn:stereo.mono)
                Toggle("反转左声道极性",isOn:stereo.invertLeft)
                Toggle("反转右声道极性",isOn:stereo.invertRight)
            }
        case .convolution:
            convolution
        case .loudness:
            Toggle("自动响度均衡",isOn:Binding(get:{draft.normalize},set:{ enabled in
                draft.normalize = enabled
                if enabled { draft.enabled = true }
            }))
            if draft.normalize { number("目标响度 (LUFS)",$draft.target) }
            Text("按歌曲实测响度调节音量。没有分析结果的歌曲按原响度播放。")
                .font(.caption).foregroundStyle(.secondary)
            if let measured = zone.track?.integrated_lufs { Text("当前歌曲实测 \(measured.formatted(.number.precision(.fractionLength(1)))) LUFS").font(.caption).foregroundStyle(.secondary) }
            else { Text("当前歌曲暂无响度分析。").font(.caption).foregroundStyle(.secondary) }
        case .fades:
            if endpoint["capabilities"]["transitions"] == .bool(false) { Text("此输出不支持淡入淡出。").foregroundStyle(.secondary) }
            else {
                Stepper("曲首淡入 \(draft.fadeIn) ms",value:$draft.fadeIn,in:0...10000,step:10)
                Stepper("曲尾淡出 \(draft.fadeOut) ms",value:$draft.fadeOut,in:0...10000,step:10)
                HStack {
                    Button("均衡预设") { draft.fadeIn = PlaybackSettingsDraft.balancedFadeIn; draft.fadeOut = PlaybackSettingsDraft.balancedFadeOut }
                    Button("关闭") { draft.fadeIn = 0; draft.fadeOut = 0 }
                }
            }
        }
    }
    private var equalizer: some View {
        VStack(alignment:.leading,spacing:12) {
            EqualizerGraph(bands:bandBinding,selected:$selectedBand,rate:rate,bypassed:eq.bypass,add:add)
            if advanced { Toggle("旁路参数均衡",isOn:bypassBinding).controlSize(.small) }
            if !eq.bands.isEmpty { bandTabs }
            if let index = chosenIndex { bandInspector(index) }
            if eq.bands.contains(where:{$0.frequency >= Double(rate)/2}) { Text("达到采样率一半的频段会在播放时跳过。").font(.caption).foregroundStyle(.secondary) }
        }
    }
    private var bandTabs: some View {
        ScrollView(.horizontal) {
            HStack(spacing:6) {
                ForEach(Array(eq.bands.enumerated()),id:\.element.id) { index, band in
                    bandTab(index,band)
                }
            }
        }
    }
    private func bandTab(_ index:Int,_ band:PlaybackSettingsDraft.Band) -> some View {
        Button { selectedBand = band.id } label:{
            HStack(spacing:6) {
                Text("\(index+1)").font(.caption2.bold()).foregroundStyle(Color.black).frame(width:19,height:19)
                    .background(EqualizerGraph.colors[index % EqualizerGraph.colors.count],in:Circle())
                Text(frequency(band.frequency)).monospacedDigit().font(.caption)
                Text(band.kind.label).font(.caption2).foregroundStyle(.secondary)
            }.padding(6)
                .background(Color.primary.opacity(selectedBand == band.id ? 0.09 : 0.035),in:RoundedRectangle(cornerRadius:6))
        }.buttonStyle(.plain)
    }
    private func bandInspector(_ index:Int) -> some View {
        ViewThatFits(in:.horizontal) {
            HStack(alignment:.bottom,spacing:10) { bandControls(index) }
            VStack(alignment:.leading,spacing:7) { bandControls(index) }
        }.padding(10).background(Color.primary.opacity(0.04),in:RoundedRectangle(cornerRadius:10))
    }
    @ViewBuilder private func bandControls(_ index:Int) -> some View {
        let band = bandBinding[index]
        let kind = Binding(get:{ band.wrappedValue.kind },set:{ if !$0.isCut { band.slope.wrappedValue = 12 }; band.kind.wrappedValue = $0 })
        VStack(alignment:.leading,spacing:5) {
            Text("滤波器").font(.caption).foregroundStyle(.secondary)
            Picker("滤波器",selection:kind) { ForEach(PlaybackSettingsDraft.Filter.allCases,id:\.self) { Text($0.label).tag($0) } }.labelsHidden().fixedSize()
        }
        bandNumber("频率 (Hz)",band.frequency)
        bandNumber("增益 (dB)",band.gain).disabled(!kind.wrappedValue.hasGain)
        bandNumber("Q 值",band.q)
        if advanced && kind.wrappedValue.isCut {
            VStack(alignment:.leading,spacing:5) {
                Text("斜率").font(.caption).foregroundStyle(.secondary)
                Picker("斜率",selection:band.slope) { ForEach([12,24,36,48],id:\.self) { Text("\($0) dB/oct").tag($0) } }.labelsHidden().fixedSize()
            }
        }
        Toggle("启用频段",isOn:band.enabled).toggleStyle(.checkbox).fixedSize()
        Button("删除频段",systemImage:"trash") {
            draft.editEQ(at:activeEQ) { $0.bands.remove(at:index) }; selectedBand = eq.bands.first?.id
        }.labelStyle(.iconOnly)
    }
    private var convolution: some View {
        VStack(alignment:.leading,spacing:10) {
            Text("每个采样率选择一组响应，保留 WAV 原始增益。").font(.caption).foregroundStyle(.secondary)
            if loadingResponses { ProgressView().controlSize(.small) }
            if impulses.isEmpty && !loadingResponses { Text("响应库为空，可在 Web 的卷积模块导入 WAV。").foregroundStyle(.secondary) }
            ForEach(Array(impulses.enumerated()),id:\.offset) { _, asset in
                let id = asset["id"].text
                HStack {
                    VStack(alignment:.leading,spacing:4) {
                        Text(asset["name"].text)
                        Text("\(frequency(asset["sample_rate"].number)) · \(asset["channels"].integer) 声道").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button(savedResponseIDs.contains(id) ? "移除" : "使用") { chooseResponse(asset) }.disabled(!coreDSP && !endpoint["capabilities"]["convolution_v1"].boolean)
                }.padding(8).background(.primary.opacity(0.035),in:RoundedRectangle(cornerRadius:8))
            }
        }
    }
    private func frequency(_ hz:Double) -> String { hz < 1000 ? "\(Int(hz)) Hz" : "\((hz/1000).formatted(.number.precision(.fractionLength(0...2)))) kHz" }
    private func bandNumber(_ label:String,_ value:Binding<Double>) -> some View {
        VStack(alignment:.leading,spacing:5) {
            Text(label).font(.caption).foregroundStyle(.secondary)
            TextField(label,value:value,format:.number.precision(.fractionLength(0...4))).labelsHidden().monospacedDigit().frame(width:77)
        }
    }
    private func number(_ label:String,_ value:Binding<Double>) -> some View {
        HStack {Text(label).frame(minWidth:130,alignment:.leading);TextField(label,value:value,format:.number.precision(.fractionLength(0...4))).labelsHidden().monospacedDigit().frame(width:100)}
    }
    private func reset() {
        draft = PlaybackSettingsDraft(zone); saved = draft
        choose(Slot(module:portable ? .loudness : .eq))
    }
    private func add(_ hz:Double,_ gain:Double) {
        guard eq.bands.count < 32, draft.totalBands < 64 else { return }
        let band = PlaybackSettingsDraft.Band(frequency:hz,gain:gain)
        draft.editEQ(at:activeEQ) { $0.bands.append(band) }; draft.enabled = true; selectedBand = band.id
    }
    private func tenBandPreset() {
        let frequencies = [31.0,62,125,250,500,1000,2000,4000,8000,16000]
        let bands = frequencies.filter { $0 < Double(rate) / 2 }.map { PlaybackSettingsDraft.Band(frequency:$0) }
        guard draft.totalBands - eq.bands.count + bands.count <= 64 else {
            failed = true; message = "该音区的 EQ 频段合计最多 64 段"; return
        }
        draft.editEQ(at:activeEQ) { $0.bands = bands; $0.bypass = false }
        draft.enabled = true
        selectedBand = eq.bands.first?.id
    }
    private func chooseResponse(_ asset:JSONValue) {
        let id = asset["id"].text, current = savedResponseIDs
        let other = current.filter { old in !impulses.contains { $0["id"].text == old && $0["sample_rate"] == asset["sample_rate"] } }
        let next = current.contains(id) ? other : other + [id]
        draft.original["convolution"] = next.isEmpty ? nil : .object(["response_ids": .array(next.map(JSONValue.string))])
    }
    private func loadResponses() async {
        loadingResponses = true; defer { loadingResponses = false }
        do { impulses = try await store.api("/dsp/impulses")["items"].array }
        catch where !Task.isCancelled { failed = true; message = error.localizedDescription }
        catch {}
    }
    private func importPreset(_ url:URL) {
        Task { @MainActor in
            working = true; defer { working = false }
            do {
                let accessed = url.startAccessingSecurityScopedResource()
                defer { if accessed { url.stopAccessingSecurityScopedResource() } }
                let file = try FileHandle(forReadingFrom:url)
                defer { try? file.close() }
                let data = try file.read(upToCount:65_537) ?? Data()
                guard data.count <= 65_536,let source = String(data:data,encoding:.utf8) else { throw RuntimeFailure(message:"请选择不超过 64 KiB 的 UTF-8 均衡预设。") }
                let preset = try await store.api("/dsp/equalizer/import",method:"POST",body:.object(["text":.string(source)]))
                var bands = preset["bands"].array.map(PlaybackSettingsDraft.Band.decode)
                guard draft.totalBands - eq.bands.count + bands.count <= 64 else { throw RuntimeFailure(message:"该音区的 EQ 频段合计最多 64 段") }
                if !advanced { for index in bands.indices { bands[index].slope = 12 } }
                draft.editEQ(at:activeEQ) { $0.bands = bands; $0.bypass = false }
                draft.preamp = preset["preamp_db"].number
                draft.enabled = true
                selectedBand = eq.bands.first?.id
                failed = false; message = "已导入预设，请应用到音区。"
            } catch { failed = true; message = error.localizedDescription }
        }
    }
    private func apply() {
        Task { @MainActor in
            working = true; defer { working = false }
            do {
                let candidate = draft
                let payload = try candidate.payload(rate:rate)
                let baseline = try saved.payload(rate:rate)
                let path = "/zones/\(zone.id.pathComponentEncoded)"
                if payload["dsp"] != baseline["dsp"] {
                    if let localOutput { try store.effects.edit("dsp",value:payload["dsp"],uid:localOutput.uid,portable:portable) }
                    else { try await store.requireUngrouped(zone.id); _ = try await store.api(path+"/dsp",method:"PUT",body:payload["dsp"]) }
                    let fadeIn = saved.fadeIn, fadeOut = saved.fadeOut
                    saved = candidate; saved.fadeIn = fadeIn; saved.fadeOut = fadeOut
                }
                if payload["transitions"] != baseline["transitions"] {
                    if let localOutput { try store.effects.edit("transitions",value:payload["transitions"],uid:localOutput.uid,portable:portable) }
                    else { try await store.requireUngrouped(zone.id); _ = try await store.api(path,method:"PATCH",body:.object(["transitions":payload["transitions"]])) }
                }
                if !portable && store.connected && store.coreReachable { try await store.refreshZones() }
                saved = candidate; failed = false; message = portable ? "已保存，从下一首开始应用" : "已应用到此音区"
            } catch { failed = true; message = error.localizedDescription }
        }
    }
}

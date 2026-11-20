import SwiftUI
import AppKit

/// One native console edits the existing renderer; preset cards do not create preview contexts.
struct MineradioConsoleView: View {
    @Environment(\.dismiss) private var dismiss
    @StoredState private var tab = "常用"
    @StoredState private var search = ""
    @AppStorage("mineradio.parameters") private var parameters = "{}"
    @AppStorage("mineradio.archives") private var archives = "[]"
    @AppStorage("mineradio.preset") private var preset = 0
    @AppStorage("mineradio.fps") private var fps = 60
    @AppStorage("mineradio.particles") private var particles = 180
    @AppStorage("mineradio.scale") private var scale = 1.0
    @AppStorage("mineradio.intensity") private var intensity = 0.85
    @AppStorage("mineradio.floatLayer") private var floatLayer = false
    @AppStorage("mineradio.backCover") private var backCover = false
    @AppStorage("mineradio.shelf") private var shelf = false
    @AppStorage("mineradio.enabled") private var enabled = true
    @AppStorage("mineradio.reactive") private var reactive = true
    @AppStorage("mineradio.pauseWhenUnfocused") private var pauseWhenUnfocused = true
    @AppStorage("mineradio.autoHideTransport") private var autoHideTransport = true
    private let tabs = ["常用", "界面", "歌词", "动效", "歌单架", "系统"]
    private var values: [String: JSONValue] { (try? JSONDecoder().decode([String: JSONValue].self, from: Data(parameters.utf8))) ?? [:] }
    private var controls: [MineradioControl] { MineradioControl.all.filter { search.isEmpty ? $0.tab == tab : $0.label.localizedCaseInsensitiveContains(search) || $0.group.localizedCaseInsensitiveContains(search) } }
    private var groups: [String] { controls.map(\.group).reduce(into: []) { if !$0.contains($1) { $0.append($1) } } }
    private var saved: [[String: JSONValue]] { (try? JSONDecoder().decode([[String: JSONValue]].self, from: Data(archives.utf8))) ?? [] }
    var body: some View {
        VStack(spacing: 0) {
            HStack { VStack(alignment: .leading, spacing: 4) { Text("视觉控制台").font(.title2.weight(.semibold)); Text("Mineradio").font(.caption).foregroundStyle(.secondary) }; Spacer(); TextField("搜索参数", text: $search).textFieldStyle(.roundedBorder).frame(width: 230); Button("完成") { dismiss() }.keyboardShortcut(.defaultAction) }.padding(24)
            HStack(spacing: 8) { ForEach(tabs, id: \.self) { item in Button { tab = item; search = "" } label: { Text(item).font(.system(size: 13, weight: tab == item ? .semibold : .regular)).padding(.horizontal, 16).padding(.vertical, 9).background(tab == item ? Color.white.opacity(0.14) : .clear, in: Capsule()) }.buttonStyle(.plain) }; Spacer() }.padding(.horizontal, 24).padding(.bottom, 16)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if (search.isEmpty && tab == "界面") || (!search.isEmpty && "自动隐藏播放栏".localizedCaseInsensitiveContains(search)) { panel("播放控制") { Toggle("自动隐藏播放栏", isOn: $autoHideTransport) } }
                    if search.isEmpty {
                        if tab == "常用" { presets; archiveSection }
                        if tab == "动效" { panel("画面图层") { Toggle("浮空粒子层", isOn: $floatLayer); Toggle("封面背光", isOn: $backCover); HStack { Text("律动强度"); Slider(value: $intensity, in: 0...1.5); Text(intensity, format: .number.precision(.fractionLength(2))).monospacedDigit().frame(width: 42) } } }
                        if tab == "歌单架" { panel("显示方式") { Toggle("3D 播放队列", isOn: $shelf) } }
                        if tab == "系统" { performance }
                    }
                    ForEach(groups, id: \.self) { group in
                        MineradioControlGroup(title: group, initiallyOpen: !search.isEmpty || group == groups.first) {
                            ForEach(controls.filter { $0.group == group }) { field in control(field) }
                        }
                        .id("\(group)-search-\(!search.isEmpty)")
                    }
                    if !search.isEmpty && controls.isEmpty { ContentUnavailableView.search(text: search) }
                }.padding(24)
            }
        }.frame(width: 860, height: 680).background(Color(red: 0.045, green: 0.055, blue: 0.075)).environment(\.colorScheme, .dark)
    }
    private var presets: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("视觉预设").font(.headline)
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: 4), spacing: 10) {
                ForEach(MineradioPreset.all, id: \.id) { item in Button { preset = item.id } label: {
                    VStack(alignment: .leading, spacing: 14) {
                        Image(systemName: ["sparkles", "waveform.path", "circle.hexagongrid", "circle.dotted", "circle.lefthalf.filled", "square.stack.3d.up", "moon.stars", "waveform", "water.waves", "circle.circle", "cloud.rain", "squareshape.split.3x3", "aqi.medium"][item.id % 13]).font(.system(size: 24, weight: .ultraLight)).foregroundStyle(Color(hue: Double(item.id) / 18 + 0.48, saturation: 0.28, brightness: 1))
                        Text(item.name.replacingOccurrences(of: " · Sonic Topography", with: "")).font(.system(size: 12, weight: .medium)).lineLimit(1)
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(16).background(LinearGradient(colors: [.white.opacity(preset == item.id ? 0.15 : 0.06), .white.opacity(0.02)], startPoint: .topLeading, endPoint: .bottomTrailing), in: RoundedRectangle(cornerRadius: 14)).overlay(RoundedRectangle(cornerRadius: 14).stroke(.white.opacity(preset == item.id ? 0.6 : 0.08)))
                }.buttonStyle(.plain) }
            }
        }
    }
    private var archiveSection: some View {
        panel("用户存档") {
            HStack { Button("保存当前方案", systemImage: "plus") { var list = saved; list.insert(["parameters": .string(parameters), "preset": .number(Double(preset)), "intensity": .number(intensity), "float": .bool(floatLayer), "back": .bool(backCover), "shelf": .bool(shelf), "autoHideTransport": .bool(autoHideTransport)], at: 0); archives = encode(Array(list.prefix(4))) }; Spacer(); Button("恢复默认") { parameters = "{}"; preset = 0; intensity = 0.85; floatLayer = false; backCover = false; shelf = false; fps = 60; particles = 180; scale = 1; enabled = true; reactive = true; autoHideTransport = true } }
            ForEach(Array(saved.enumerated()), id: \.offset) { index, value in HStack { Text("方案 \(index + 1)").foregroundStyle(.secondary); Spacer(); Button("应用") { parameters = value["parameters"]?.string ?? "{}"; preset = Int(value["preset"]?.number ?? 0); intensity = value["intensity"]?.number ?? 0.85; floatLayer = value["float"]?.boolean ?? false; backCover = value["back"]?.boolean ?? false; shelf = value["shelf"]?.boolean ?? false; autoHideTransport = value["autoHideTransport"]?.boolean ?? true }; Button("移除") { var list = saved; list.remove(at: index); archives = encode(list) } } }
        }
    }
    private var performance: some View {
        panel("性能与调度") {
            Toggle("GPU 动态背景", isOn: $enabled)
            Toggle("跟随实际音频律动", isOn: $reactive).disabled(!enabled)
            Toggle("失去焦点时暂停动效", isOn: $pauseWhenUnfocused)
            Text("最小化或窗口完全不可见时始终暂停绘制，音乐继续。拖动上边缘移动窗口；在舞台中拖动调整视角，滚轮缩放，双击复位。").font(.caption).foregroundStyle(.secondary)
            PreferenceChoice("帧率上限", selection: $fps) { Text("24 fps").tag(24); Text("30 fps").tag(30); Text("60 fps").tag(60) }
            PreferenceChoice("粒子精细度", selection: $particles) { Text("轻量 · 6 千").tag(80); Text("均衡 · 1.4 万").tag(118); Text("细腻 · 3.2 万").tag(180) }
            HStack { Text("渲染比例"); Slider(value: $scale, in: 0.5...1); Text(scale, format: .percent.precision(.fractionLength(0))).monospacedDigit().frame(width: 44) }
        }
    }
    @ViewBuilder private func control(_ field: MineradioControl) -> some View {
        switch field.kind {
        case "toggle": Toggle(field.label, isOn: Binding(get: { values[field.id]?.boolean ?? field.flag ?? false }, set: { set(field.id, .bool($0)) }))
        case "range":
            HStack(spacing: 18) { Text(field.label).frame(width: 130, alignment: .leading); Slider(value: Binding(get: { values[field.id]?.number ?? field.number ?? 0 }, set: { set(field.id, .number(($0 / (field.step ?? 0.01)).rounded() * (field.step ?? 0.01))) }), in: (field.minimum ?? 0)...(field.maximum ?? 1)).accessibilityLabel(field.label); Text(values[field.id]?.number ?? field.number ?? 0, format: .number.precision(.fractionLength((field.step ?? 1) < 1 ? 2 : 0))).font(.system(.caption, design: .monospaced)).foregroundStyle(.secondary).frame(width: 50, alignment: .trailing) }
        case "choice":
            HStack { Text(field.label).frame(width: 130, alignment: .leading); ThemePicker(field.label, selection: Binding(get: { values[field.id]?.string ?? field.text ?? "" }, set: { set(field.id, .string($0)) })) { ForEach(field.choices ?? []) { Text($0.label).tag($0.id) } }.labelsHidden().pickerStyle(.segmented) }
        case "color": ColorPicker(field.label, selection: Binding(get: { color(values[field.id]?.string ?? field.text ?? "#9db8cf") }, set: { value in if let rgb = NSColor(value).usingColorSpace(.deviceRGB) { set(field.id, .string(String(format: "#%02x%02x%02x", Int(rgb.redComponent * 255), Int(rgb.greenComponent * 255), Int(rgb.blueComponent * 255)))) } }), supportsOpacity: false)
        default: EmptyView()
        }
    }
    private func set(_ key: String, _ value: JSONValue) { var data = values; data[key] = value; parameters = encode(data) }
    private func encode<T: Encodable>(_ value: T) -> String { (try? String(decoding: JSONEncoder().encode(value), as: UTF8.self)) ?? "{}" }
    private func color(_ hex: String) -> Color { let value = UInt64(hex.dropFirst(), radix: 16) ?? 0x9db8cf; return Color(red: Double((value >> 16) & 255) / 255, green: Double((value >> 8) & 255) / 255, blue: Double(value & 255) / 255) }
    private func panel<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View { VStack(alignment: .leading, spacing: 18) { Text(title).font(.headline); content() }.padding(20).frame(maxWidth: .infinity, alignment: .leading).background(.white.opacity(0.045), in: RoundedRectangle(cornerRadius: 16)) }
}

private struct MineradioControlGroup<Content: View>: View {
    let title: String
    let content: () -> Content
    @StoredState private var expanded: Bool
    init(title: String, initiallyOpen: Bool, @ViewBuilder content: @escaping () -> Content) {
        self.title = title; self.content = content; _expanded = StoredState(initialValue: initiallyOpen)
    }
    var body: some View { DisclosureGroup(isExpanded: $expanded) { VStack(alignment: .leading, spacing: 18, content: content).padding(.top, 18) } label: { Text(title).font(.headline) }.padding(20).background(.white.opacity(0.045), in: RoundedRectangle(cornerRadius: 16)) }
}

import SwiftUI

/// Local gestures avoid exporting queue contents through the system drag pasteboard.
@MainActor @Observable final class TrackOrderGesture {
    static let shared = TrackOrderGesture()
    struct Item: Equatable { let scope: String; let ids: [String]; let index: Int }
    struct Row { let item: Item; let frame: CGRect; let move: @MainActor (Int, Int) -> Void }
    var active: Item?
    var target: Item?
    var translation: CGFloat = 0
    @ObservationIgnored private var rows: [UUID: Row] = [:]
    func register(_ key: UUID, _ row: Row) { rows[key] = row }
    func remove(_ key: UUID) { rows.removeValue(forKey: key) }
    func update(_ item: Item, location: CGPoint, translation: CGFloat) {
        if active == nil {
            active = item
            if ProcessInfo.processInfo.environment["RMUSIC_DRAG_QA_LOG"] == "1" {
                FileHandle.standardError.write(Data("RMusic drag location=\(location) frames=\(rows.values.map(\.frame))\n".utf8))
            }
        }
        guard active == item else { cancel(); return }
        self.translation = translation
        target = rows.values.first { $0.item.scope == item.scope && $0.item.ids == item.ids && $0.frame.contains(location) }?.item
    }
    func finish() {
        if ProcessInfo.processInfo.environment["RMUSIC_DRAG_QA_LOG"] == "1" { FileHandle.standardError.write(Data("RMusic drag ended target=\(target?.index.description ?? "none")\n".utf8)) }
        let source = active
        let destination = target.flatMap { item in rows.values.first { $0.item == item } }
        cancel()
        guard let source, let destination, source.index != destination.item.index else { return }
        destination.move(source.index, destination.item.index)
    }
    func step(_ item: Item, by distance: Int) {
        let to = item.index + distance
        guard item.ids.indices.contains(to), let row = rows.values.first(where: { $0.item.scope == item.scope && $0.item.ids == item.ids && $0.item.index == to }) else { return }
        row.move(item.index, to)
    }
    func cancel() { active = nil; target = nil; translation = 0 }
}

struct TrackOrderDrop: ViewModifier {
    let scope: String
    let ids: [String]
    let index: Int
    var enabled = true
    let move: @MainActor (Int, Int) -> Void
    @StoredState private var key = UUID()
    @StoredState private var frame = CGRect.zero
    private var item: TrackOrderGesture.Item { .init(scope: scope, ids: ids, index: index) }
    func body(content: Content) -> some View {
        let gesture = TrackOrderGesture.shared
        let active = gesture.active == item
        content.contentShape(Rectangle())
            .background(active ? MusicStyle.accent.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 10))
            .shadow(color: .black.opacity(active ? 0.2 : 0), radius: 12, y: 5)
            .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { value in if gesture.active == nil { frame = value; register(value) } }
            .onChange(of: item) { register(frame) }
            .onChange(of: enabled) { register(frame) }
            .onAppear { register(frame) }
            .onDisappear { TrackOrderGesture.shared.remove(key) }
            .offset(y: offset)
            .zIndex(active ? 1 : 0)
            .animation(active ? nil : .smooth(duration: 0.18), value: offset)
    }
    private var offset: CGFloat {
        let gesture = TrackOrderGesture.shared
        guard let active = gesture.active, active.scope == scope else { return 0 }
        if active == item { return gesture.translation }
        guard let target = gesture.target else { return 0 }
        if active.index < target.index && index > active.index && index <= target.index { return -(frame.height + 4) }
        if active.index > target.index && index < active.index && index >= target.index { return frame.height + 4 }
        return 0
    }
    private func register(_ frame: CGRect) {
        if enabled && !frame.isEmpty { TrackOrderGesture.shared.register(key, .init(item: item, frame: frame, move: move)) }
        else { TrackOrderGesture.shared.remove(key) }
    }
}

struct TrackOrderHandle: View {
    let scope: String
    let ids: [String]
    let index: Int
    private var item: TrackOrderGesture.Item { .init(scope: scope, ids: ids, index: index) }
    var body: some View {
        Button {} label: {
            Image(systemName: "line.3.horizontal").foregroundStyle(.secondary)
                .frame(width: 26, height: 36).contentShape(Rectangle())
        }.buttonStyle(.plain)
            .highPriorityGesture(DragGesture(minimumDistance: 5, coordinateSpace: .global)
                .onChanged { value in TrackOrderGesture.shared.update(item, location: value.location, translation: value.translation.height) }
                .onEnded { _ in TrackOrderGesture.shared.finish() })
            .contextMenu {
                Button("上移") { TrackOrderGesture.shared.step(item, by: -1) }.disabled(index == 0)
                Button("下移") { TrackOrderGesture.shared.step(item, by: 1) }.disabled(index + 1 == ids.count)
            }
            .accessibilityAction(named: Text("上移")) { TrackOrderGesture.shared.step(item, by: -1) }
            .accessibilityAction(named: Text("下移")) { TrackOrderGesture.shared.step(item, by: 1) }
            .help("拖动排序，或右键选择上移、下移").accessibilityLabel("拖动第 \(index + 1) 首排序")
    }
}

func movedTrackIDs(_ ids: [String], from: Int, to: Int) -> [String] {
    guard ids.indices.contains(from), ids.indices.contains(to), from != to else { return ids }
    var result = ids
    result.insert(result.remove(at: from), at: to)
    return result
}

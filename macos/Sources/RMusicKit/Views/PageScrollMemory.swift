import SwiftUI

private struct ScrollMetrics: Equatable { var y: Double; var height: Double; var viewport: Double }
private struct PageScrollMemory: ViewModifier {
    let store: MusicStore
    let key: String
    @StoredState private var position = ScrollPosition(y: 0)
    @StoredState private var restoring: Double?
    init(store: MusicStore, key: String) {
        self.store = store; self.key = key
        _restoring = StoredState(wrappedValue: store.pageSnapshots.scrollOffset(key, owner: store.sessionID))
    }
    func body(content: Content) -> some View {
        content.scrollPosition($position)
            .onScrollGeometryChange(for: ScrollMetrics.self) { geometry in
                ScrollMetrics(y: Double(geometry.contentOffset.y), height: Double(geometry.contentSize.height), viewport: Double(geometry.containerSize.height))
            } action: { _, metrics in
                if let saved = restoring {
                    // Wait for the cached or revalidated content to have enough
                    // height. A user's own scroll immediately takes precedence.
                    if position.isPositionedByUser { restoring = nil }
                    else if metrics.height - metrics.viewport >= saved - 1 {
                        restoring = nil; position.scrollTo(y: saved); return
                    } else { return }
                }
                store.pageSnapshots.saveScroll(max(0, metrics.y), key: key, owner: store.sessionID)
            }
    }
}
extension View {
    func rememberPageScroll(_ store: MusicStore, key: String) -> some View { modifier(PageScrollMemory(store: store, key: key)) }
}

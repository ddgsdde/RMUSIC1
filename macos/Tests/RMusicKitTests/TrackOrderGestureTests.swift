import Foundation
import Testing
@testable import RMusicKit

@Test @MainActor func dragReorderUsesTheVisibleOccurrenceAndRejectsAnotherPageOrAccount() {
    let gesture = TrackOrderGesture(), ids = ["A", "A", "B"]
    var moves: [(Int, Int)] = []
    let source = TrackOrderGesture.Item(scope: "session:playlist", ids: ids, index: 0)
    let target = TrackOrderGesture.Item(scope: source.scope, ids: ids, index: 2)
    let key = UUID()
    gesture.register(key, .init(item: target, frame: CGRect(x: 10, y: 100, width: 300, height: 60), move: { moves.append(($0, $1)) }))
    gesture.update(source, location: CGPoint(x: 30, y: 130), translation: 100)
    gesture.finish()
    #expect(moves.count == 1 && moves[0].0 == 0 && moves[0].1 == 2)
    #expect(gesture.active == nil && gesture.target == nil)
    let other = TrackOrderGesture.Item(scope: "other-session:playlist", ids: ids, index: 0)
    gesture.update(other, location: CGPoint(x: 30, y: 130), translation: 100)
    gesture.finish()
    #expect(moves.count == 1)
    gesture.update(source, location: CGPoint(x: 30, y: 130), translation: 100)
    gesture.remove(key) // A page/queue change removes its targets during the gesture.
    gesture.finish()
    #expect(moves.count == 1)
}

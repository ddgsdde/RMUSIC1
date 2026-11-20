import Foundation
import Testing
@testable import RMusicKit
@Test func catalogNewMusicOnlyUsesConfirmedDatesAndExpires() {
    let now = Date(timeIntervalSince1970: 1793095200) // 2026-10-27 10:00 UTC
    #expect(Track.newRelease("2026-10-20", now: now))
    #expect(!Track.newRelease("2026-10-28", now: now))
    #expect(!Track.newRelease("2026-09-27", now: now))
    #expect(!Track.newRelease("", now: now))
    #expect(!Track.newRelease("not-a-date", now: now))
}

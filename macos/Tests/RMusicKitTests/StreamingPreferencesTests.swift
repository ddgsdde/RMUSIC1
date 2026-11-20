import Foundation
import Testing
@testable import RMusicKit

struct StreamingPreferencesTests {
    @Test func newDefaultsPreserveSourceAndUseWANBuffer() throws {
        let value = StreamingPreferences()
        #expect(value.quality == "original")
        #expect(value.max_sample_rate == 768_000)
        #expect(value.buffer_seconds == 30)
        #expect(try JSONDecoder().decode(StreamingPreferences.self, from: JSONEncoder().encode(value)) == value)
    }
    @Test func legacyDefaultsMigrateAndExplicitLimitsRemainCustom() throws {
        let old = Data(#"{"mode":"auto","max_sample_rate":96000,"max_bit_depth":24,"buffer_seconds":15}"#.utf8)
        let migrated = try JSONDecoder().decode(StreamingPreferences.self, from: old)
        #expect(migrated.quality == "original")
        #expect(migrated.buffer_seconds == 30)
        let custom = Data(#"{"mode":"remote","max_sample_rate":48000,"max_bit_depth":16,"buffer_seconds":20}"#.utf8)
        let retained = try JSONDecoder().decode(StreamingPreferences.self, from: custom)
        #expect(retained.quality == "custom")
        #expect(retained.max_sample_rate == 48000)
        #expect(retained.buffer_seconds == 20)
    }
}

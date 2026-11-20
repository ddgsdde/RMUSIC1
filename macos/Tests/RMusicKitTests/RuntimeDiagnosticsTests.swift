import Testing
@testable import RMusicKit

@Test func signedMediaAndLocalCapabilityAreRemovedFromDiagnostics() {
        let message = #"decode failed (https://music.example/api/v1/media/track?ticket=private-ticket) via http://127.0.0.1:54848/private-capability/audio; core=playing"#
        let saved = RuntimeDiagnostics.redact(message)
        #expect(!saved.contains("private-ticket"))
        #expect(!saved.contains("private-capability"))
        #expect(!saved.contains("127.0.0.1"))
        #expect(saved.contains("core=playing"))
        #expect(RuntimeDiagnostics.redact("CoreAudio queued=8192 played=0 paused=false") == "CoreAudio queued=8192 played=0 paused=false")
}

import Testing
@testable import MineradioNative

@Test func ordinaryLyricsAndTranslationsNeverSweepAcrossAnInferredLineDuration() {
    let line = NativeLyricLine(time:10,text:"甲乙",translation:"Translation",words:[])
    #expect(nativeLyricProgress(line:line,position:9.9,stops:[],translation:false) == 0)
    for position in [10.0,10.1,12,20] { #expect(nativeLyricProgress(line:line,position:position,stops:[],translation:false) == 1) }
}

@Test func realWordDurationsKeepGapsAndDoNotSweepTranslatedCharacters() {
    let line = NativeLyricLine(time:10,text:"甲乙",translation:"Translation",words:[
        NativeLyricWord(time:10.2,text:"甲",duration:0.2),NativeLyricWord(time:12,text:"乙",duration:0.8)])
    #expect(nativeLyricProgress(line:line,position:10.1,stops:[],translation:false) == 0)
    #expect(abs(nativeLyricProgress(line:line,position:10.3,stops:[],translation:false)-0.25) < 0.0001)
    #expect(nativeLyricProgress(line:line,position:11.7,stops:[],translation:false) == 0.5)
    #expect(abs(nativeLyricProgress(line:line,position:12.4,stops:[0,0.4,1],translation:false)-0.7) < 0.0001)
    #expect(nativeLyricProgress(line:line,position:10.3,stops:[],translation:true) == 1)
    let startsOnly=NativeLyricLine(time:10,text:"甲乙",translation:nil,words:[NativeLyricWord(time:10.2,text:"甲"),NativeLyricWord(time:12,text:"乙")])
    #expect(nativeLyricProgress(line:startsOnly,position:10.3,stops:[],translation:false) == 0.5)
}

import Foundation
import Testing
@testable import RMusicKit

@MainActor private final class TravelTransport: RuntimeTransport {
    var commands: [(String,JSONValue)] = []
    var pause: CheckedContinuation<Void,Never>?
    var reply: ((String, JSONValue) throws -> JSONValue)?
    func call(_ operation:String,_ body:JSONValue) async throws ->JSONValue {
        commands.append((operation,body))
        if operation=="local_transport",body["type"].text=="pause" {
            await withCheckedContinuation {pause=$0}
        }
        return try reply?(operation, body) ?? .null
    }
    func shutdown() async {}
}

@Test @MainActor func travelEffectsAreLocalAndUnavailableTrackYieldsToNextSong() async throws {
    let io = TravelTransport(), store = MusicStore(runtime: io)
    let uid = UUID().uuidString
    defer { UserDefaults.standard.removeObject(forKey:"outputEffects.v1." + uid) }
    store.connected = true
    let raw = """
    {"host_id":"travel-test","roaming":true,"items":[{"id":"travel-test-output","uid":"\(uid)","name":"耳机","available":true,"enabled":true,"online":true,"mix_rate":48000,"channels":2,"system_default":false,"config":{"enabled":true,"exclusive":false,"buffer_ms":100}}]}
    """
    store.outputs.accept(try JSONDecoder().decode(OutputInventory.self, from:Data(raw.utf8)))
    try await store.travel.refresh(store,report:.null)
    let local = try #require(store.zone)
    let hardware = try #require(store.outputs.output(for:local))
    let lufs: JSONValue = .object(["enabled":.bool(true),"normalize":.bool(true),"target_lufs":.number(-16)])
    let fades: JSONValue = .object(["fade_in_ms":.number(40),"fade_out_ms":.number(90)])
    try store.effects.edit("dsp",value:lufs,uid:uid,portable:true)
    try store.effects.edit("transitions",value:fades,uid:uid,portable:true)
    var lan = local; lan.id = "lan"
    #expect(store.effects.zone(for:hardware,base:lan).dsp != lufs)
    #expect(store.effects.zone(for:hardware,base:lan).transitions != fades)
    let songs = ["unavailable", "next"].map { Track(id:$0,title:$0,artist:"",album:"",duration:20,format:"flac",sample_rate:48000,bit_depth:16) }
    store.travel.remember(songs)
    let path = "/zones/" + local.id
    _ = try await store.api(path+"/queue",method:"PUT",body:.object(["track_ids":.array(songs.map { .string($0.id) })]))
    _ = try await store.api(path+"/play",method:"POST")
    let id = try #require(store.zone?.playback_id)
    await store.travel.receiveLoudness(.object(["track_id":.string("next"),
        "integrated_lufs":.number(-21),"true_peak_dbfs":.number(-3)]),store:store)
    await store.travel.receiveLoudness(.object(["track_id":.string("unavailable"),"playback_id":.string(id),
        "integrated_lufs":.number(-22),"true_peak_dbfs":.number(-4)]),store:store)
    #expect(io.commands.contains { $0.0 == "stream_control" && $0.1["type"].text == "loudness" && $0.1["playback_id"].text == id })
    try await store.travel.refresh(store,report:.object(["stream_output":.string(hardware.id),
        "stream":.object(["type":.string("error"),"core_playback_id":.string(id),"source_unavailable":.bool(true),"error":.string("没有完整音源")])]))
    let next = try #require(io.commands.last { $0.0 == "stream_control" }?.1)
    #expect(next["track"]["id"].text == "next")
    #expect(next["track"]["integrated_lufs"].number == -21)
    #expect(next["dsp"] == lufs && next["transitions"] == fades)
    #expect(store.zone?.track?.id == "next")
}

@Test @MainActor func travelControlIsLocalImmediatelyAndRestoresQueue() async throws {
    let io=TravelTransport(), store=MusicStore(runtime:io)
    let core=UUID().uuidString, user=UUID().uuidString, scope="travelQueue.v1."+core+"."+user
    defer {UserDefaults.standard.removeObject(forKey:scope);UserDefaults.standard.removeObject(forKey:scope+".test-uid")}
    store.profile.core_id=core;store.user = .object(["id":.string(user)]);store.connected=true;store.coreReachable=false
    let raw = #"{"host_id":"mac-test","roaming":true,"items":[{"id":"test-uid","uid":"test","name":"耳机","available":true,"enabled":true,"online":true,"mix_rate":48000,"channels":2,"system_default":false,"config":{"enabled":true,"exclusive":false,"buffer_ms":100}}]}"#
    store.outputs.accept(try JSONDecoder().decode(OutputInventory.self,from:Data(raw.utf8)))
    try await store.travel.refresh(store,report:.null)
    #expect(store.canPlay)
    let songs=["a","b"].map {Track(id:$0,title:$0,artist:"",album:"",duration:110,format:"flac",sample_rate:48000,bit_depth:16,cover_url:"/api/v1/artwork/cover-"+$0,source:"local")}
    store.travel.remember(songs)
    let path="/zones/travel-test-uid"
    _=try await store.api(path+"/queue?view=state",method:"PUT",body:.object(["track_ids":.array([.string("a"),.string("b")])]))
    _=try await store.api(path+"/play",method:"POST")
    let instance=try #require(store.zone?.playback_id)
    let paused=Task {try await store.api(path+"/pause",method:"POST")}
    while io.pause==nil {await Task.yield()}
    #expect(store.zone?.status=="paused")
    io.pause?.resume();_=try await paused.value
    _=try await store.api(path+"/seek",method:"POST",body:.object(["position":.number(17)]))
    _=try await store.api(path+"/play",method:"POST")
    #expect(store.zone?.playback_id==instance && store.zone?.position==17)
    #expect(io.commands.filter{$0.0=="local_transport"}.map{$0.1["type"].text}==["pause","seek","resume"])
    #expect(io.commands.filter{$0.0=="local_transport"}.allSatisfy{$0.1["playback_id"].text==instance})
    _=try await store.api(path,method:"PATCH",body:.object(["repeat":.string("one")]))
    #expect(store.travel.nextIndex(try #require(store.zone),ended:true)==0)
    store.travel.reset()
    try await store.travel.refresh(store,report:.null)
    #expect(store.zone?.queue.map(\.id)==["a","b"])
    #expect(store.zone?.track?.cover_url==songs[0].cover_url && store.zone?.track?.source=="local")
    #expect(store.zone?.status=="paused" && store.zone?.position==17 && store.zone?.repeat=="one")
    let pausedReport: JSONValue = .object(["output": .string("test-uid"), "report": .object(["status": .string("paused"), "position": .number(17)])])
    let handoff = try #require(store.travel.returning(pausedReport))
    #expect(handoff.pausedPosition == 17 && handoff.zone.track?.id == "a")
    store.travel.checkpoint(store)
    store.travel.reset()
    #expect(store.travel.savedReturn(store)?.pausedPosition == 17)
    store.travel.finishReturn(store)
    #expect(store.travel.savedReturn(store) == nil)
    #expect(store.travel.returning(.null, selectedOutput: "test-uid")?.pausedPosition == 17)
    #expect(store.travel.returning(.object(["output": .string("test-uid"), "report": .object(["core_playback_id": .string("previous"), "status": .string("ended")])])) == nil)

    store.user = .object(["id":.string("different")]);store.travel.reset()
    try await store.travel.refresh(store,report:.null)
    #expect(store.zone?.queue.isEmpty==true)
}

@Test @MainActor func roamingShowsHomeZonesAndKeepsLocalPlaybackSelectedIndependently() async throws {
    let io=TravelTransport(), store=MusicStore(runtime:io)
    store.connected=true;store.coreReachable=true
    let raw = #"{"host_id":"mac-away","roaming":true,"items":[{"id":"mac-away-headphones","uid":"test","name":"耳机","available":true,"enabled":true,"online":true,"mix_rate":48000,"channels":2,"system_default":false,"config":{"enabled":true,"exclusive":false,"buffer_ms":100}}]}"#
    store.outputs.accept(try JSONDecoder().decode(OutputInventory.self,from:Data(raw.utf8)))
    try await store.travel.refresh(store,report:.null)
    let local=try #require(store.zone)
    func familyZone(_ id:String,_ endpoint:String)->Zone {var z=local;z.id=id;z.endpoint_id=endpoint;return z}
    func endpoint(_ id:String)->JSONValue {.object(["id":.string(id),"online":.bool(true)])}
    let home=familyZone("living-room","speaker")
    let stale=familyZone("old-lan-zone","mac-away-previous-output")
    store.family.accept([home,stale],[endpoint("speaker"),endpoint(stale.endpoint_id)],store:store)
    store.selectedZoneID=home.id
    try await store.travel.refresh(store,report:.null)
    #expect(Set(store.zones.map(\.id))==[local.id,home.id])
    #expect(store.zone?.id==home.id && store.canPlay)
    #expect(store.localZone?.id==local.id)
    _=try await store.api("/zones/living-room/pause",method:"POST")
    #expect(io.commands.contains{$0.0=="request" && $0.1["path"].text=="/zones/living-room/pause"})
    #expect(!io.commands.contains{$0.0=="local_transport"})
    store.family.reset()
}

@Test @MainActor func travelPreparesOneSuccessorAndRecoversAtTheHeardPosition() async throws {
    let io=TravelTransport(), store=MusicStore(runtime:io)
    store.connected=true
    let raw = #"{"host_id":"mac-test","roaming":true,"items":[{"id":"headphones","uid":"test","name":"耳机","available":true,"enabled":true,"online":true,"mix_rate":48000,"channels":2,"system_default":false,"config":{"enabled":true,"exclusive":false,"buffer_ms":100}}]}"#
    store.outputs.accept(try JSONDecoder().decode(OutputInventory.self,from:Data(raw.utf8)))
    let songs=["first","second"].map {Track(id:$0,title:$0,artist:"",album:"",duration:80,format:"flac",sample_rate:48000,bit_depth:16)}
    try await store.travel.refresh(store,report:.null)
    store.travel.remember(songs)
    let path="/zones/travel-headphones"
    _=try await store.api(path+"/queue",method:"PUT",body:.object(["track_ids":.array(songs.map{.string($0.id)})]))
    _=try await store.api(path+"/play",method:"POST")
    func report(_ id:String,_ fields:[String:JSONValue])->JSONValue {
        .object(["stream_output":.string("headphones"),"stream":.object(fields.merging(["core_playback_id":.string(id)]){_,v in v})])
    }
    let first=try #require(store.zone?.playback_id)
    let playing=report(first,["type":.string("state"),"status":.string("playing"),"position":.number(73)])
    try await store.travel.refresh(store,report:playing)
    try await store.travel.refresh(store,report:playing)
    #expect(io.commands.filter{$0.1["type"].text=="preload" && $0.1["next_track"].text=="second"}.count==1)
    try await store.travel.refresh(store,report:report(first,["status":.string("ended")]))
    #expect(store.zone?.track?.id=="second")
    let second=try #require(store.zone?.playback_id)
    let failure=report(second,["type":.string("error"),"source_failed":.bool(true),"position":.number(17.25)])
    try await store.travel.refresh(store,report:failure)
    try await Task.sleep(for:.milliseconds(1050))
    try await store.travel.refresh(store,report:failure)
    let resumed=try #require(io.commands.last{$0.1["recover"].boolean}?.1)
    #expect(resumed["track"]["id"].text=="second" && resumed["position"].number==17.25)
    let resumedID=try #require(store.zone?.playback_id)
    #expect(resumedID==second)
    let lostAgain=report(resumedID,["type":.string("error"),"source_failed":.bool(true),"position":.number(17.25)])
    try await store.travel.refresh(store,report:lostAgain)
    _=try await store.api(path+"/pause",method:"POST")
    try await store.travel.refresh(store,report:lostAgain)
    #expect(store.zone?.status=="paused" && io.commands.filter{$0.1["recover"].boolean}.count==1)
}

@Test @MainActor func pausedTravelSeekReturnsToTheHomeOutputWithoutStartingAudio() async throws {
    let io = TravelTransport(), store = MusicStore(runtime: io)
    let core = UUID().uuidString, user = UUID().uuidString
    let key = "travelQueue.v1." + core + "." + user
    defer {
        for suffix in ["", ".home-output", ".pendingHome"] { UserDefaults.standard.removeObject(forKey: key + suffix) }
    }
    store.profile.core_id = core
    store.user = .object(["id": .string(user)])
    store.connected = true
    let inventory = #"{"host_id":"mac-test","roaming":true,"items":[{"id":"home-output","uid":"test","name":"耳机","available":true,"enabled":true,"online":true,"mix_rate":48000,"channels":2,"system_default":false,"config":{"enabled":true,"exclusive":false,"buffer_ms":100}}]}"#
    store.outputs.accept(try JSONDecoder().decode(OutputInventory.self, from: Data(inventory.utf8)))
    let output = try #require(store.outputs.enabled.first)
    let track = Track(id:"cursor", title:"Cursor", artist:"", album:"", duration:120, format:"flac", sample_rate:48000, bit_depth:16)
    var home = store.travel.empty(output)
    home.id = "home-zone"; home.status = "paused"; home.position = 19; home.track = track; home.queue = [track]
    let page = JSONValue.object(["items": .array([try .encoded(home)])])
    io.reply = { _, body in
        switch body["path"].text {
        case "/zones": return page
        case "/endpoints": return .object(["items": .array([.object(["id": .string(output.id), "online": .bool(true)])])])
        default: return .null
        }
    }
    store.zones = [home]; store.selectedZoneID = home.id
    try await store.refreshZones()
    #expect(store.zone?.position == 19)
    _ = try await store.api("/zones/travel-home-output/seek", method:"POST", body:.object(["position":.number(41)]))
    store.outputs.roaming = false
    try await store.refreshZones()
    let writes = io.commands.map(\.1).filter { $0["method"].text == "POST" && ($0["path"].text.removingPercentEncoding ?? "").hasPrefix("/zones/home-zone/") }
    #expect(writes.map { $0["path"].text.removingPercentEncoding } == ["/zones/home-zone/seek"])
    #expect(writes.first?["json"]["position"].number == 41)
}

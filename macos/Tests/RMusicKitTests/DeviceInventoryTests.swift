import Foundation
import Testing
@testable import RMusicKit

@Test @MainActor func DeviceInventoryKeepsLocalHostFirstAndDefaultRouteSeparate() throws {
    let item: [String:JSONValue] = ["id":.string("physical"),"uid":.string("uid"),"name":.string("A DAC"),"available":.bool(true),"enabled":.bool(false),"online":.bool(false),"mix_rate":.number(48000),"channels":.number(2),"system_default":.bool(true),"config":.object(["enabled":.bool(false),"exclusive":.bool(false),"buffer_ms":.number(100),"volume_control":.string("fixed")])]
    var route = item
    route["id"] = .string("follow"); route["uid"] = .string("rmusic:system-default"); route["name"] = .string("系统默认输出"); route["route_name"] = .string("A DAC")
    let store = OutputsStore()
    store.accept(try JSONValue.object(["host_id":.string("this-mac"),"items":.array([.object(item),.object(route)])]).decode(OutputInventory.self))
    #expect(store.items.map(\.id) == ["follow","physical"])
    #expect(store.items[0].route_name == "A DAC" && !store.items[0].enabled)
    let devices: [JSONValue] = [.object(["id":.string("remote-A")]),.object(["id":.string("this-mac"),"outputs":.array([])]),.object(["id":.string("remote-B")])]
    #expect(store.arrangedDevices(devices).map { $0["id"].text } == ["this-mac","remote-A","remote-B"])
    store.reset(); #expect(store.hostID.isEmpty)
}

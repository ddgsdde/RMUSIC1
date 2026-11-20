import Foundation

/// Remote playback controls use Core's regular, account-scoped zones. The travelling output stays private to this Mac.
@MainActor final class TravelFamily {
    private(set) var zones: [Zone] = []
    private(set) var endpoints: [JSONValue] = []
    private(set) var localMembers: Set<String> = []
    private var lastRead = Date.distantPast
    private var work: Task<Void,Never>?
    private var generation = UUID()
    func reset() { generation=UUID();work?.cancel();work=nil;zones=[];endpoints=[];localMembers=[];lastRead = .distantPast }
    func invalidate() {lastRead = .distantPast}
    func update(_ store:MusicStore, force:Bool=false) {
        let watching = !store.selectedZoneID.isEmpty && !store.selectedZoneID.hasPrefix("travel-")
        let interval:TimeInterval = store.uiActive && watching ? 3 : 60
        guard work==nil,force || Date().timeIntervalSince(lastRead)>=interval else{return}
        let epoch=store.sessionID, owner=generation;lastRead=Date()
        work=Task { [weak store] in
            defer {if generation==owner {work=nil}}
            guard let store else{return}
            do {
                async let zonePage=store.api("/zones")
                async let outputs=store.api("/endpoints")
                let (page,inventory)=try await (zonePage,outputs)
                let rows=try page.decode(ZonePage.self).items
                guard !Task.isCancelled,store.connected,store.outputs.roaming,store.sessionID==epoch else{return}
                accept(rows,inventory["items"].array,store:store)
                store.coreReachable=true
            } catch {
                guard !Task.isCancelled,store.sessionID==epoch,store.outputs.roaming else{return}
                if let failure=error as? RuntimeFailure, failure.status==401 {await store.handle(failure)}
                else {endpoints=[];store.onlineEndpoints=Set(store.outputs.enabled.filter(\.available).map(\.id));store.endpointSnapshot=[]}
            }
        }
    }
    func accept(_ rows:[Zone],_ inventory:[JSONValue],store:MusicStore) {
        func own(_ id:String)->Bool {store.outputs.contains(id) || (!store.outputs.hostID.isEmpty && id.hasPrefix(store.outputs.hostID+"-"))}
        localMembers=Set(rows.filter{own($0.endpoint_id)}.map(\.id))
        let blocked=Set(inventory.filter{own($0["id"].text) || !$0["group"]["zone_ids"].array.compactMap(\.string).allSatisfy{!localMembers.contains($0)}}.map{$0["id"].text})
        endpoints=inventory.filter{!blocked.contains($0["id"].text)}
        zones=rows.filter{!own($0.endpoint_id) && !blocked.contains($0.endpoint_id)}
        store.endpointSnapshot=endpoints
        store.zones=store.zones.filter{$0.id.hasPrefix("travel-")} + zones
        store.onlineEndpoints=Set(store.outputs.enabled.filter(\.available).map(\.id)).union(endpoints.filter{$0["online"].boolean}.map{$0["id"].text})
        if let z=store.zone,!z.id.hasPrefix("travel-"){store.presentationClock.record(z)}
    }
}

import SwiftUI

struct MusicSpaceArtistsView: View {
    let store: MusicStore
    @StoredState private var resource = ResourceStore()
    @StoredState private var page = 0
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            ResourceNotice(resource: resource)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 145, maximum: 190), spacing: 24)], alignment: .leading, spacing: 28) {
                ForEach(Array(resource.value["items"].array.enumerated()), id: \.offset) { _, artist in
                    Button { Task { await store.openCatalogReference(CatalogReference(id: artist["id"].text, name: artist["name"].text, detail: artist["source"].text), track: Track(id: artist["id"].text, title: artist["name"].text, artist: artist["name"].text, album: "", duration: 0, format: "", sample_rate: 0, bit_depth: 0), album: false) } } label: {
                        VStack(alignment: .leading, spacing: 10) {
                            ArtworkView(track: Track(id: artist["id"].text, title: artist["name"].text, artist: artist["name"].text, album: "", duration: 0, format: "", sample_rate: 0, bit_depth: 0, cover_url: artist["image_url"].string), store: store, radius: 75).frame(width: 145, height: 145)
                            Text(artist["name"].text).font(.headline).lineLimit(1)
                            Text("\(artist["track_count"].integer) 首").font(.caption).foregroundStyle(.secondary)
                        }.frame(width: 145)
                    }.buttonStyle(MusicButtonStyle())
                }
            }
            OffsetPageControls(offset: Binding(get: { page * 50 }, set: { page = $0 / 50 }), total: resource.value["total"].integer, limit: 50, disabled: resource.loading)
        }.task(id: "\(store.sessionID):\(store.query):\(page)") { await resource.load(apiQuery("/library/artists", ["q": store.query, "limit": "50", "offset": String(page * 50)]), store: store) }.onChange(of: store.query) { page = 0 }
    }
}

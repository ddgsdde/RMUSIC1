import SwiftUI

struct RelatedCatalogLinks: View {
    let track: Track
    let store: MusicStore
    var body: some View {
        HStack(spacing: 18) {
            CatalogTextLink(track: track, store: store, album: true)
            CatalogTextLink(track: track, store: store)
        }.font(.callout)
    }
}

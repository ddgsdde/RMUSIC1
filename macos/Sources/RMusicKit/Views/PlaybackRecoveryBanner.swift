import SwiftUI

struct PlaybackRecoveryBanner: View {
  let store: MusicStore
  @StoredState private var recovering = false
  var body: some View {
    if let state = store.details.recovery {
      HStack(spacing: 10) {
        if state.kind == .buffering {
          ProgressView().controlSize(.mini)
        } else {
          Image(systemName: "exclamationmark.circle").foregroundStyle(.orange)
        }
        Text(state.message).font(.caption).lineLimit(2).textSelection(.enabled)
        Spacer()
        if state.recoverable {
          Button(recovering ? "正在恢复…" : "重新检测并恢复") {
            recovering = true
            Task {
              await store.retryOutput()
              recovering = false
            }
          }.disabled(recovering || !store.coreReachable)
        }
      }.padding(.horizontal, 24).padding(.vertical, 10).background(.bar)
    }
  }
}

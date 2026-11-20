import SwiftUI

struct DevicePurgeSheet: View {
    let device: JSONValue
    let store: MusicStore
    let supported: Bool
    let completed: () -> Void
    @Environment(\.dismiss) private var dismiss
    @StoredState private var confirmation = ""
    @StoredState private var action = WorkflowAction()
    private var target: DevicePurge { DevicePurge(id: device["id"].text, name: device["name"].text) }
    private var payload: JSONValue? { try? target.payload(confirmation: confirmation, admin: store.user["role"].text == "admin", supported: supported) }
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("彻底删除设备 · \(target.name)").font(.title2.bold())
            Text("永久删除该设备的名称、输出配置和已归档音区队列，并移除组合中的对应音区。音乐文件、歌单和聆听历史保留。")
            Text("此操作不可恢复。仍在运行的客户端重新连接后会作为新设备出现。").font(.callout).foregroundStyle(.orange)
            TextField("输入完整设备名称确认", text: $confirmation).textFieldStyle(.roundedBorder)
            WorkflowNotice(action: action)
            HStack {
                Button("取消") { dismiss() }.disabled(action.busy)
                Spacer()
                Button("确认彻底删除", role: .destructive) {
                    guard let payload else { return }
                    action.send("/devices/\(target.id.pathComponentEncoded)/purge", body: payload, store: store) { _ in completed(); dismiss() }
                }.disabled(payload == nil || action.busy)
            }
        }.padding(28).frame(width: 470)
        .interactiveDismissDisabled(action.busy)
        .onDisappear { action.cancel(); confirmation = "" }
    }
}

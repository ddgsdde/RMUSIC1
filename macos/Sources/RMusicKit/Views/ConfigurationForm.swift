import SwiftUI

struct ConfigurationForm: View {
    let store: MusicStore
    let title: String
    var subtitle = ""
    let readPath: String
    var readKey: String?
    let savePath: String
    var method = "PATCH"
    let fields: [ConfigurationField]
    @StoredState private var resource = ResourceStore()
    @StoredState private var draft = ConfigurationDraft()
    @StoredState private var action = WorkflowAction()
    @StoredState private var refreshID = UUID()
    var body: some View {
        FeatureCard {
            FeatureHeader(title: title, subtitle: subtitle, loading: resource.loading) { refreshID = UUID() }
            ResourceNotice(resource: resource)
            WorkflowNotice(action: action)
            if !resource.loading && resource.error == nil {
                ForEach(editableFields) { field in
                    HStack(alignment: .center) {
                        Text(field.title).frame(maxWidth: .infinity, alignment: .leading)
                        editor(field).frame(width: 240)
                    }.font(.system(size: 12)).padding(.vertical, 4)
                }
                Button(action.busy ? "正在保存…" : "保存设置") { save() }.buttonStyle(WorkspacePrimaryButton()).disabled(action.busy || !store.coreReachable)
            }
        }.task(id: "\(store.sessionID):\(readPath):\(refreshID)") { await load() }.onDisappear { action.cancel() }
    }
    @ViewBuilder private func editor(_ field: ConfigurationField) -> some View {
        switch field.kind {
        case .toggle:
            Toggle(field.title, isOn: Binding(get: { draft.get(field.key).boolean }, set: { draft.set(field.key, .bool($0)) })).labelsHidden().toggleStyle(.switch)
        case .number:
            TextField(field.title, value: Binding(get: { draft.get(field.key).number / field.divisor }, set: { draft.set(field.key, .number($0 * field.divisor)) }), format: .number).textFieldStyle(.roundedBorder)
        case .text:
            TextField(field.title, text: Binding(get: { draft.get(field.key).text }, set: { draft.set(field.key, .string($0)) })).textFieldStyle(.roundedBorder)
        case .choice(let choices):
            ThemePicker(field.title, selection: Binding(get: { draft.get(field.key).text }, set: { draft.set(field.key, .string($0)) })) { ForEach(choices.keys.sorted(), id: \.self) { Text(choices[$0]!).tag($0) } }.labelsHidden()
        }
    }
    private var editableFields: [ConfigurationField] {
        fields.filter { if case .null = draft.get($0.key) { return false }; return true }
    }
    private func load() async {
        guard await resource.load(readPath, store: store), !Task.isCancelled else { return }
        draft.values = (readKey.map { resource.value[$0] } ?? resource.value).object
    }
    private func save() {
        do { action.send(savePath, method: method, body: try draft.payload(editableFields), store: store) { _ in refreshID = UUID() } }
        catch { action.error = error.localizedDescription }
    }
}

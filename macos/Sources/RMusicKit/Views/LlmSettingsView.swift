import SwiftUI

struct LlmSettingsView: View {
    let store: MusicStore
    @StoredState private var resource = ResourceStore()
    @StoredState private var algorithm = ResourceStore()
    @StoredState private var refreshID = UUID()
    @StoredState private var action = WorkflowAction()
    @StoredState private var enabled = false
    @StoredState private var address = ""
    @StoredState private var model = ""
    @StoredState private var key = ""
    @StoredState private var clearKey = false
    @StoredState private var models: [String] = []
    var body: some View {
        FeatureCard {
            FeatureHeader(title: "推荐算法", loading: resource.loading || algorithm.loading) { refreshID = UUID() }
            ResourceNotice(resource: resource); ResourceNotice(resource: algorithm); WorkflowNotice(action: action)
            Text("统一用于家庭账号的发现推荐和电台。收藏、聆听记录和推荐结果仍按账号分开。").font(.caption).foregroundStyle(.secondary)
            if algorithm.value["allowed"].boolean {
                ThemePicker("家庭推荐算法", selection: Binding(get: { algorithm.value["advanced"].boolean }, set: selectAlgorithm)) {
                    Text("本地算法").tag(false)
                    Text("进阶算法").tag(true)
                }.frame(maxWidth: 260)
            }
            Text("搜歌时，进阶算法向模型发送参考歌曲的歌名、艺人和曲风。重排时会发送候选曲目编号、歌名、艺人、专辑和简短偏好。模型不可用时，Core 仍可从曲库和音源目录选歌。").font(.caption).foregroundStyle(.secondary)
            Toggle("允许 LLM 推荐", isOn: $enabled)
            TextField("OpenAI 兼容接口地址", text: $address).textContentType(.URL)
                .onChange(of: address) { models = [] }
            SecureField(resource.value["key_configured"].boolean ? "服务密钥：留空保留" : "服务密钥", text: $key)
                .onChange(of: key) { if !key.isEmpty { clearKey = false } }
            if resource.value["key_configured"].boolean { Toggle("清除已保存密钥", isOn: $clearKey) }
            TextField("模型", text: $model)
            if !models.isEmpty {
                ThemePicker("可用模型", selection: $model) { Text("手动输入").tag(models.contains(model) ? "" : model); ForEach(models, id: \.self) { Text($0).tag($0) } }
            }
            HStack {
                Button("获取模型") {
                    var body: [String: JSONValue] = ["base_url": .string(address)]
                    if clearKey || !key.isEmpty { body["key"] = .string(clearKey ? "" : key) }
                    action.send("/llm/models", body: .object(body), store: store) { value in
                        models = value["items"].array.compactMap(\.string)
                        if models.count == 1 { model = models[0] }
                        action.notice = "获取到 \(models.count) 个模型，保存后生效。"
                    }
                }.disabled(address.isEmpty)
                Spacer()
                Button("保存配置") {
                    var body: [String: JSONValue] = ["expected_revision": resource.value["revision"], "enabled": .bool(enabled), "base_url": .string(address), "model": .string(model)]
                    if clearKey || !key.isEmpty { body["key"] = .string(clearKey ? "" : key) }
                    action.send("/llm/settings", method: "PUT", body: .object(body), store: store) { value in
                        resource.value = value; key = ""; clearKey = false
                    }
                }.disabled(resource.value["revision"].string == nil || key.utf8.count > 8192 || model.utf8.count > 200 || address.utf8.count > 2048)
            }
            Text("更换地址需重新填写密钥。").font(.caption).foregroundStyle(.secondary)
        }.disabled(action.busy).task(id: "\(store.sessionID):\(refreshID)") { await load() }
            .onDisappear { action.cancel(); key = "" }
    }
    private func selectAlgorithm(_ advanced: Bool) {
        action.send("/recommendations/settings", method: "PUT", body: .object(["advanced": .bool(advanced)]), store: store) { value in
            algorithm.value = value
            store.discoveryFeed.reset()
            action.notice = "家庭推荐算法已保存。"
        }
    }
    private func load() async {
        async let mode = algorithm.load("/recommendations/settings", store: store)
        if await resource.load("/llm/settings", store: store) {
            enabled = resource.value["enabled"].boolean; address = resource.value["base_url"].text; model = resource.value["model"].text
        }
        _ = await mode
    }
}

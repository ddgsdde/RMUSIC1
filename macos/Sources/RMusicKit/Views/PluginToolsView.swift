import SwiftUI

struct PluginToolsView: View {
  let store: MusicStore
  @StoredState private var plugins = ResourceStore()
  @StoredState private var tools = ResourceStore()
  @StoredState private var extensions = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var export = FileExportAction()
  @StoredState private var selected = ""
  @StoredState private var kind = "extension"
  @StoredState private var identifier = "my-plugin"
  @StoredState private var configID: String?
  @StoredState private var invocation: JSONValue = .null
  @StoredState private var refresh = UUID()
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      FeatureHeader(title: "插件开发与扩展") { refresh = UUID() }
      ResourceNotice(resource: plugins)
      ResourceNotice(resource: tools)
      ResourceNotice(resource: extensions)
      WorkflowNotice(action: action)
      if let error = export.error { Text(error).font(.caption).foregroundStyle(.orange) }
      FeatureCard {
        Text("开发工具").font(.headline)
        Button("导出 OpenAPI 文档") { export.save("/openapi.json", name: "r-music-openapi.json", store: store) }.disabled(export.busy)
        Fact(
          name: "SDK / API",
          value: "\(tools.value["sdk_version"].text) / \(tools.value["api_version"].text)")
        HStack {
          ThemePicker("模板", selection: $kind) {
            Text("只读扩展").tag("extension")
            Text("音源骨架").tag("source")
          }
          TextField("插件 ID", text: $identifier)
        }
        Button("下载开发模板") {
          export.save(
            apiQuery("/plugins/development/template", ["kind": kind, "id": identifier]),
            name: "\(identifier).zip", store: store)
        }.disabled(!validID || export.busy)
        DisclosureGroup("创建、检查与打包") {
          ForEach(tools.value["commands"].array.compactMap(\.string), id: \.self) {
            Text($0).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
          }
        }
        ThemePicker("已安装插件", selection: $selected) {
          Text("选择插件").tag("")
          ForEach(Array(plugins.value["items"].array.enumerated()), id: \.offset) { _, plugin in
            Text("\(plugin["name"].text) · \(plugin["version"].text) · \(plugin["id"].text)").tag(
              plugin["id"].text)
          }
        }
        HStack {
          if plugins.value["items"].array.contains(where: {
            $0["id"].text == selected && !$0["configuration_v1"].object.isEmpty
          }) {
            Button("配置插件") { configID = selected }
          }
          Button("合同检查") {
            action.send("/plugins/\(selected.pathComponentEncoded)/contract-check", store: store) {
              value in action.notice = value["passed"].boolean ? "插件合同检查通过" : "插件合同检查未通过"
            }
          }
        }.disabled(selected.isEmpty || action.busy)
      }
      FeatureCard {
        Text("可用扩展").font(.headline)
        if extensions.value["items"].array.isEmpty {
          Text("安装扩展插件后，可在这里打开对应功能。").font(.caption).foregroundStyle(.secondary)
        }
        ForEach(Array(extensions.value["items"].array.enumerated()), id: \.offset) { _, plugin in
          Text(plugin["name"].text).fontWeight(.medium)
          ForEach(Array(plugin["extensions"].array.enumerated()), id: \.offset) { _, method in
            HStack {
              VStack(alignment: .leading, spacing: 6) {
                Text(method["title"].string ?? method["method"].text)
                Text(method["description"].text).font(.caption).foregroundStyle(.secondary)
              }
              Spacer()
              Button("打开") {
                var item = method.object
                item["plugin_id"] = plugin["id"]
                invocation = .object(item)
              }.disabled(!plugin["enabled"].boolean)
            }
          }
        }
      }
    }.task(id: "\(store.sessionID):\(refresh)") {
      await plugins.load("/plugins", store: store)
      await tools.load("/plugins/development", store: store)
      await extensions.load("/extensions", store: store)
    }
    .onDisappear {
      action.cancel()
      export.cancel()
    }
    .sheet(isPresented: Binding(get: { configID != nil }, set: { if !$0 { configID = nil } })) {
      PluginConfigurationView(store: store, id: configID ?? "", close: { configID = nil })
    }
    .sheet(
      isPresented: Binding(
        get: { invocation["method"].string != nil }, set: { if !$0 { invocation = .null } })
    ) { PluginInvocationView(store: store, method: invocation, close: { invocation = .null }) }
  }
  private var validID: Bool {
    identifier.range(of: "^[a-z][a-z0-9_-]{0,63}$", options: .regularExpression) != nil
  }
}
struct PluginConfigurationView: View {
  let store: MusicStore
  let id: String
  let close: () -> Void
  @StoredState private var resource = ResourceStore()
  @StoredState private var action = WorkflowAction()
  @StoredState private var draft: JSONValue = .object([:])
  @StoredState private var refresh = UUID()
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      HStack {
        Text("插件配置").font(.title2.bold())
        Spacer()
        Button("关闭", action: close)
      }
      ResourceNotice(resource: resource)
      WorkflowNotice(action: action)
      ScrollView {
        SchemaForm(
          schema: resource.value["schema"],
          value: Binding(
            get: { SchemaDraft.merged(resource.value["values"], draft, schema: resource.value["schema"]) },
            set: {
              draft = SchemaDraft.patch(
                base: resource.value["values"], value: $0, schema: resource.value["schema"])
            }))
      }
      HStack {
        Button("读取最新配置") { refresh = UUID() }
        Spacer()
        Button("保存配置") {
          do {
            let schema = resource.value["schema"]
            let base = resource.value["values"]
            let patch = try SchemaValidation.configurationPatch(base: base, value: SchemaDraft.merged(base, draft, schema: schema), schema: schema)
            action.send(
              "/plugins/\(id.pathComponentEncoded)/configuration", method: "PUT",
              body: .object(["expected_revision": resource.value["revision"], "values": patch]),
              store: store
            ) { _ in close() }
          } catch { action.error = error.localizedDescription }
        }.disabled(resource.value["revision"].string == nil || action.busy)
      }
    }.padding(24).frame(width: 650, height: 550).task(id: "\(store.sessionID):\(id):\(refresh)") {
      if await resource.load("/plugins/\(id.pathComponentEncoded)/configuration", store: store) {
        let base = resource.value["values"]
        let schema = resource.value["schema"]
        draft = SchemaDraft.patch(
          base: base, value: SchemaDraft.merged(SchemaDraft.prepared(base, schema: schema), draft, schema: schema),
          schema: schema)
      }
    }.onDisappear {
      action.cancel()
      draft = .null
      resource.value = .null
    }
  }
}
struct PluginInvocationView: View {
  let store: MusicStore
  let method: JSONValue
  let close: () -> Void
  @StoredState private var input: JSONValue = .object([:])
  @StoredState private var result: JSONValue = .null
  @StoredState private var action = WorkflowAction()
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      HStack {
        Text(method["title"].string ?? method["method"].text).font(.title2.bold())
        Spacer()
        Button("关闭", action: close)
      }
      WorkflowNotice(action: action)
      ScrollView {
        VStack(alignment: .leading, spacing: 18) {
          SchemaForm(schema: method["input"], value: $input)
          StructuredResult(value: result)
        }
      }
      if method["effect"].text == "proposal" {
        Text("扩展返回建议，不会自动应用到 Core。").font(.caption).foregroundStyle(.secondary)
      }
      Button(action.busy ? "正在运行…" : "运行扩展") {
        do {
          try SchemaValidation.check(input, schema: method["input"])
          guard try JSONEncoder().encode(input).count <= 65536 else { throw RuntimeFailure(message: "输入超过 64 KiB，未发送。") }
        } catch { action.error = error.localizedDescription; return }
        action.send(
          "/extensions/\(method["plugin_id"].text.pathComponentEncoded)/invoke/\(method["method"].text.pathComponentEncoded)",
          body: input, store: store
        ) {
          result = $0["output"]
          action.notice = "扩展已完成"
        }
      }.disabled(action.busy)
    }.padding(24).frame(width: 700, height: 580).task(id: store.sessionID) {
      input = SchemaDraft.initial(method["input"])
    }.onDisappear {
      action.cancel()
      input = .null
      result = .null
    }
  }
}

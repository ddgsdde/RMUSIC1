import SwiftUI

/// The Core validates this closed schema too. Depth and rendered arrays stay bounded.
struct SchemaForm: View {
  let schema: JSONValue
  @Binding var value: JSONValue
  var depth = 0
  @StoredState private var offset = 0
  var body: AnyView {
    guard depth <= 8 else { return AnyView(Text("嵌套层级超过 Core 上限").foregroundStyle(.orange)) }
    if schema["secret"].boolean {
      var plain = schema.object; plain["secret"] = .bool(false)
      if SchemaValidation.protected(value) {
        return AnyView(HStack { Text("已保护，未修改则保留").font(.caption).foregroundStyle(.secondary); Button("更改受保护值") { value = SchemaDraft.initial(.object(plain)) } })
      }
      if schema["type"].text != "string" { return AnyView(SchemaForm(schema: .object(plain), value: $value, depth: depth)) }
    }
    if !schema["enum"].array.isEmpty {
      return AnyView(ThemePicker("选择", selection: Binding(get: { encoded(value) }, set: { raw in if let item = schema["enum"].array.first(where: { encoded($0) == raw }) { value = item } })) { ForEach(Array(schema["enum"].array.enumerated()), id: \.offset) { _, item in Text(item.string ?? encoded(item)).tag(encoded(item)) } }.labelsHidden())
    }
    switch schema["type"].text {
    case "object":
      return AnyView(
        VStack(alignment: .leading, spacing: 14) {
          ForEach(Array(schema["properties"].object.keys.sorted().prefix(64)), id: \.self) { key in
            let field = schema["properties"][key]
            VStack(alignment: .leading, spacing: 6) {
              Text(
                (field["description"].string ?? key)
                  + (schema["required"].array.contains { $0.string == key } ? " *" : "")
              ).font(.caption).foregroundStyle(.secondary)
              SchemaForm(
                schema: field,
                value: Binding(
                  get: { value[key] },
                  set: {
                    var fields = value.object
                    fields[key] = $0
                    value = .object(fields)
                  }), depth: depth + 1)
            }
          }
        })
    case "array":
      return AnyView(
        VStack(alignment: .leading, spacing: 10) {
          ForEach(Array(value.array.dropFirst(start).prefix(20).enumerated()), id: \.offset) { local, _ in
            let index = start + local
            HStack {
              SchemaForm(
                schema: schema["items"],
                value: Binding(
                  get: { value.array.indices.contains(index) ? value.array[index] : .null },
                  set: {
                    var items = value.array
                    if items.indices.contains(index) {
                      items[index] = $0
                      value = .array(items)
                    }
                  }), depth: depth + 1)
              Button("移除", systemImage: "minus.circle") {
                var items = value.array
                if items.indices.contains(index) {
                  items.remove(at: index)
                  value = .array(items)
                }
              }.labelStyle(.iconOnly)
            }
          }
          OffsetPageControls(offset: $offset, total: value.array.count, limit: 20)
          Button("添加项目") { value = .array(value.array + [SchemaDraft.initial(schema["items"])]); offset = max(0, (value.array.count - 1) / 20 * 20) }
            .disabled(value.array.count >= min(1000, schema["maxItems"].integer))
        })
    case "boolean":
      return AnyView(
        Toggle("启用", isOn: Binding(get: { value.boolean }, set: { value = .bool($0) })).toggleStyle(
          .switch))
    default:
      if schema["secret"].boolean {
        return AnyView(
          SecureField(
            value["configured"].boolean ? "已配置，未修改则保留" : "密钥",
            text: Binding(get: { value.string ?? "" }, set: { value = .string($0) })))
      }
      if ["integer", "number"].contains(schema["type"].text) {
        return AnyView(
          TextField(
            schema["type"].text == "integer" ? "整数" : "数值",
            value: Binding(get: { value.number }, set: { value = .number($0) }), format: .number
          ).textFieldStyle(.roundedBorder))
      }
      return AnyView(
        TextField("填写内容", text: Binding(get: { value.text }, set: { value = .string($0) }))
          .textFieldStyle(.roundedBorder))
    }
  }
  private var start: Int { min(offset, max(0, (value.array.count - 1) / 20 * 20)) }
  private func encoded(_ value: JSONValue) -> String {
    String(data: (try? JSONEncoder().encode(value)) ?? Data(), encoding: .utf8) ?? ""
  }
}
struct StructuredResult: View {
  @StoredState private var offset = 0
  let value: JSONValue
  var depth = 0
  var body: AnyView {
    guard depth <= 8 else { return AnyView(Text("更多嵌套内容已省略").font(.caption)) }
    switch value {
    case .object(let fields):
      return AnyView(
        VStack(alignment: .leading, spacing: 8) {
          ForEach(Array(fields.keys.sorted().prefix(64)), id: \.self) { key in
            DisclosureGroup(key.replacingOccurrences(of: "_", with: " ")) {
              StructuredResult(value: fields[key] ?? .null, depth: depth + 1)
            }
          }
        })
    case .array(let items):
      return AnyView(
        VStack(alignment: .leading, spacing: 10) {
          ForEach(Array(items.dropFirst(offset).prefix(20).enumerated()), id: \.offset) { _, item in
            StructuredResult(value: item, depth: depth + 1)
          }
          if items.count > 20 {
            HStack {
              Button("上一页") { offset = max(0, offset - 20) }.disabled(offset == 0)
              Text("\(offset + 1)–\(min(items.count, offset + 20)) / \(items.count)").font(.caption)
              Button("下一页") { offset += 20 }.disabled(offset + 20 >= items.count)
            }
          }
        })
    case .bool(let flag): return AnyView(Text(flag ? "是" : "否"))
    case .string(let string):
      return AnyView(Text(String(string.prefix(65536))).textSelection(.enabled))
    case .number(let number): return AnyView(Text(number.formatted()))
    case .null: return AnyView(Text("—").foregroundStyle(.secondary))
    }
  }
}

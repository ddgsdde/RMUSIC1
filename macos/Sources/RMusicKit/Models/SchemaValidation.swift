import Foundation

enum SchemaValidation {
  static func protected(_ value: JSONValue) -> Bool {
    value.object.count == 1 && value["configured"].boolean
  }
  static func check(
    _ value: JSONValue, schema: JSONValue, allowProtected: Bool = false, path: String = "配置",
    depth: Int = 0
  ) throws {
    func invalid(_ reason: String) -> RuntimeFailure {
      RuntimeFailure(message: "\(path)：\(reason)")
    }
    guard depth <= 8 else { throw invalid("嵌套超过 Core 支持的八层") }
    if schema["secret"].boolean, protected(value) {
      guard allowProtected else { throw invalid("修改列表后须重新填写受保护字段，未发送修改") }
      return
    }
    if !schema["enum"].array.isEmpty && !schema["enum"].array.contains(where: { equal($0, value) })
    {
      throw invalid("请选择声明中的值")
    }
    switch schema["type"].text {
    case "object":
      guard case .object(let fields) = value, fields.count <= 32 else { throw invalid("需要对象") }
      for key in schema["required"].array.compactMap(\.string) where fields[key] == nil {
        throw invalid("缺少 \(key)")
      }
      for (key, item) in fields {
        guard let field = schema["properties"].object[key] else { throw invalid("包含未声明字段 \(key)") }
        try check(
          item, schema: field, allowProtected: allowProtected, path: path + "." + key,
          depth: depth + 1)
      }
    case "array":
      guard case .array(let values) = value, values.count <= schema["maxItems"].integer else {
        throw invalid("列表超出声明上限")
      }
      for (index, item) in values.enumerated() {
        try check(
          item, schema: schema["items"], allowProtected: allowProtected,
          path: path + "[\(index + 1)]", depth: depth + 1)
      }
    case "string":
      guard let text = value.string, text.utf8.count <= schema["maxLength"].integer else {
        throw invalid("文本超出声明长度")
      }
    case "boolean": guard case .bool = value else { throw invalid("需要布尔值") }
    case "integer", "number":
      guard case .number(let number) = value, number.isFinite else { throw invalid("需要有限数值") }
      if schema["type"].text == "integer",
        number.rounded() != number || abs(number) > 9_007_199_254_740_991
      {
        throw invalid("整数超出安全范围或包含小数")
      }
      if case .number(let minimum) = schema["minimum"], number < minimum {
        throw invalid("小于最小值 \(minimum.formatted())")
      }
      if case .number(let maximum) = schema["maximum"], number > maximum {
        throw invalid("大于最大值 \(maximum.formatted())")
      }
    default: throw invalid("Core 未声明此类型")
    }
  }
  static func equal(_ lhs: JSONValue, _ rhs: JSONValue) -> Bool {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    return (try? encoder.encode(lhs)) == (try? encoder.encode(rhs))
  }
  static func configurationPatch(base: JSONValue, value: JSONValue, schema: JSONValue) throws
    -> JSONValue
  {
    try check(value, schema: schema, allowProtected: true)
    func validateChangedArrays(
      _ base: JSONValue, _ next: JSONValue, _ definition: JSONValue, _ path: String
    ) throws {
      if definition["secret"].boolean && protected(next) { return }
      if definition["type"].text == "array", !equal(base, next) {
        try check(next, schema: definition, path: path)
      }
      if definition["type"].text == "object" {
        for (key, item) in next.object {
          try validateChangedArrays(
            base[key], item, definition["properties"][key], path + "." + key)
        }
      }
    }
    try validateChangedArrays(base, value, schema, "配置")
    let patch = SchemaDraft.patch(base: base, value: value, schema: schema)
    guard try JSONEncoder().encode(patch).count <= 65536 else {
      throw RuntimeFailure(message: "插件配置超过 64 KiB，未发送。")
    }
    return patch
  }
}

import Foundation

struct SchemaDraft {
  var values: [String: JSONValue] = [:]
  static func initial(_ schema: JSONValue) -> JSONValue {
    if let first = schema["enum"].array.first { return first }
    switch schema["type"].text {
    case "object":
      return .object(
        Dictionary(
          uniqueKeysWithValues: schema["required"].array.compactMap { item in
            guard let key = item.string else { return nil }
            return (key, initial(schema["properties"][key]))
          }))
    case "array": return .array([])
    case "boolean": return .bool(false)
    case "integer", "number": return .number(schema["minimum"].number)
    default: return schema["enum"].array.first ?? .string("")
    }
  }
  static func prepared(_ base: JSONValue, schema: JSONValue) -> JSONValue {
    if schema["secret"].boolean && SchemaValidation.protected(base) { return base }
    guard schema["type"].text == "object" else { return base }
    var values = base.object
    for key in schema["properties"].object.keys {
      let field = schema["properties"][key]
      if let existing = values[key] {
        values[key] = prepared(existing, schema: field)
      } else if schema["required"].array.contains(where: { $0.string == key }) {
        values[key] = initial(field)
      }
    }
    return .object(values)
  }
  static func patch(base: JSONValue, value: JSONValue, schema: JSONValue) -> JSONValue {
    var patch: [String: JSONValue] = [:]
    for (key, item) in value.object {
      let field = schema["properties"][key]
      if field["secret"].boolean && SchemaValidation.protected(item) { continue }
      if field["type"].text == "object" && !field["secret"].boolean {
        let nested = Self.patch(base: base[key], value: item, schema: field)
        if !nested.object.isEmpty { patch[key] = nested }
      } else {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        if (try? encoder.encode(item)) != (try? encoder.encode(base[key])) { patch[key] = item }
      }
    }
    return .object(patch)
  }
  static func merged(_ base: JSONValue, _ patch: JSONValue, schema: JSONValue = .null) -> JSONValue {
    if schema["secret"].boolean { return patch }
    guard case .object(var values) = base, case .object(let patch) = patch else { return patch }
    for (key, value) in patch {
      if let existing = values[key], case .object = existing, case .object = value {
        values[key] = merged(existing, value, schema: schema["properties"][key])
      } else {
        values[key] = value
      }
    }
    return .object(values)
  }
}

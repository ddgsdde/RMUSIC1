import Foundation

struct ConfigurationField: Identifiable {
    enum Kind { case toggle, number(ClosedRange<Double>), text, choice([String: String]) }
    let key: String
    let title: String
    let kind: Kind
    var divisor: Double = 1
    var integral = false
    var id: String { key }
}
/// A configuration draft emits only declared editable roots, retaining nested fields from Core.
struct ConfigurationDraft {
    var values: [String: JSONValue] = [:]
    func get(_ key: String) -> JSONValue {
        key.split(separator: ".").reduce(JSONValue.object(values)) { $0[String($1)] }
    }
    mutating func set(_ key: String, _ value: JSONValue) {
        let parts = key.split(separator: ".").map(String.init)
        func replacing(_ object: [String: JSONValue], _ keys: ArraySlice<String>) -> [String: JSONValue] {
            guard let first = keys.first else { return object }
            var copy = object
            copy[first] = keys.count == 1 ? value : .object(replacing(copy[first]?.object ?? [:], keys.dropFirst()))
            return copy
        }
        values = replacing(values, parts[...])
    }
    func payload(_ fields: [ConfigurationField]) throws -> JSONValue {
        for field in fields {
            let value = get(field.key)
            switch field.kind {
            case .number(let bounds):
                guard case .number(let number) = value, number.isFinite, bounds.contains(number) else { throw RuntimeFailure(message: "\(field.title) 应在 \(bounds.lowerBound.formatted())–\(bounds.upperBound.formatted()) 之间。") }
                if field.integral && number.rounded() != number { throw RuntimeFailure(message: "\(field.title) 应为整数。") }
            case .toggle:
                guard case .bool = value else { throw RuntimeFailure(message: "\(field.title) 需要开关值。") }
            case .text:
                guard case .string = value else { throw RuntimeFailure(message: "\(field.title) 需要文本。") }
            case .choice(let choices):
                guard let selected = value.string, choices[selected] != nil else { throw RuntimeFailure(message: "请选择有效的\(field.title)。") }
            }
        }
        let keys = Set(fields.map { String($0.key.split(separator: ".")[0]) })
        return .object(values.filter { keys.contains($0.key) })
    }
}

import Foundation

@MainActor
protocol RuntimeTransport: AnyObject {
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue
    func shutdown() async
}
extension RuntimeTransport {
    func call(_ operation: String) async throws -> JSONValue { try await call(operation, .null) }
}
extension RuntimeClient: RuntimeTransport {}

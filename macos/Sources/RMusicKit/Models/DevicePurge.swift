import Foundation

struct DevicePurge {
    let id: String
    let name: String
    func payload(confirmation: String, admin: Bool, supported: Bool) throws -> JSONValue {
        guard admin, supported, !id.isEmpty, !name.isEmpty, confirmation == name else {
            throw RuntimeFailure(message: "仅管理员可彻底删除已移除设备，请输入完整设备名称确认。")
        }
        return .object(["confirmation": .string(id)])
    }
}

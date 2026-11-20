import Foundation

struct ProviderPagination {
    let page: Int
    let next: Int?
    init(page: Int, response: JSONValue) throws {
        guard page > 0 else { throw RuntimeFailure(message: "无效云端页码。") }
        self.page = page
        if case .null = response["next_page"] { next = nil }
        else {
            guard case .number(let cursor) = response["next_page"], cursor == Double(page + 1) else { throw RuntimeFailure(message: "音源返回无效分页，未将列表视为完整。") }
            next = page + 1
        }
    }
}

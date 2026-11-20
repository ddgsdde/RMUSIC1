import SwiftUI

// The macOS 27 CLT SDK exports both a State macro and the standard State wrapper,
// but does not ship SwiftUIMacros. Resolve the existing system wrapper explicitly;
// this alias adds no state storage or behavior of its own.
public typealias StoredState<Value> = SwiftUI.State<Value>

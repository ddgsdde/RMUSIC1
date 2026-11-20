import SwiftUI

/// Keep the popup's real hit area on the control column instead of the entire labelled row.
struct PreferenceChoice<Selection: Hashable, Options: View>: View {
    let title: String
    @Binding var selection: Selection
    let options: () -> Options
    init(_ title: String, selection: Binding<Selection>, @ViewBuilder options: @escaping () -> Options) { self.title = title; _selection = selection; self.options = options }
    var body: some View {
        HStack(spacing: 20) {
            Text(title).frame(maxWidth: .infinity, alignment: .leading)
            ThemePicker(title, selection: $selection, content: options).labelsHidden().pickerStyle(.menu).frame(width: 220).accessibilityLabel(title)
        }.frame(maxWidth: .infinity)
    }
}

import SwiftUI

/// Tagged choices stay declarative. A single theme surface replaces AppKit popup
/// chrome without introspection, global event monitors or retained menu windows.
struct ThemePicker<Selection: Hashable, Content: View>: View {
    let title: String
    @Binding var selection: Selection
    private let content: Content
    private var showTitle = true
    private var segmented = false
    @StoredState private var open = false
    @StoredState private var focused = 0
    @StoredState private var hovered: Int?
    @FocusState private var listFocused: Bool
    @Environment(\.colorScheme) private var scheme
    @Environment(\.isEnabled) private var enabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    init(_ title: String, selection: Binding<Selection>, @ViewBuilder content: () -> Content) {
        self.title = title; _selection = selection; self.content = content()
    }
    func labelsHidden() -> Self { var copy = self; copy.showTitle = false; return copy }
    func pickerStyle<S: PickerStyle>(_ style: S) -> Self { var copy = self; copy.segmented = style is SegmentedPickerStyle; return copy }
    var body: some View {
        Group(subviews: content) { options in
            HStack(spacing: 10) {
                if showTitle { Text(title).foregroundStyle(.secondary).fixedSize() }
                if segmented {
                    ScrollView(.horizontal) {
                        HStack(spacing: 5) {
                            ForEach(options) { option in
                                if let tag = option.containerValues.tag(for: Selection.self) {
                                    Button { selection = tag } label: {
                                        option.padding(.horizontal, 13).padding(.vertical, 9)
                                            .frame(minHeight: 32).contentShape(Rectangle())
                                            .foregroundStyle(tag == selection ? MusicStyle.accent : Color.secondary)
                                            .background(tag == selection ? MusicStyle.accent.opacity(0.14) : .clear, in: RoundedRectangle(cornerRadius: 10))
                                            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(tag == selection ? MusicStyle.accent.opacity(0.30) : .clear))
                                    }.buttonStyle(.plain).accessibilityAddTraits(tag == selection ? .isSelected : [])
                                }
                            }
                        }.padding(4)
                    }.scrollIndicators(.hidden).fixedSize(horizontal: false, vertical: true)
                        .background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 14))
                } else {
                    Button {
                        focused = options.firstIndex { $0.containerValues.hasTag(selection) } ?? 0
                        open.toggle()
                    } label: {
                        HStack(spacing: 10) {
                            if let selected = options.first(where: { $0.containerValues.hasTag(selection) }) { selected.lineLimit(1) }
                            else { Text("未选择").foregroundStyle(.secondary) }
                            Spacer(minLength: 4)
                            Image(systemName: "chevron.down").font(.system(size: 9, weight: .semibold)).rotationEffect(.degrees(open ? 180 : 0)).foregroundStyle(.secondary)
                        }.padding(.horizontal, 12).padding(.vertical, 9).contentShape(RoundedRectangle(cornerRadius: 11))
                            .background(LinearGradient(colors: [MusicStyle.accent.opacity(open ? 0.15 : 0.065), Color.primary.opacity(0.025)], startPoint: .topLeading, endPoint: .bottomTrailing), in: RoundedRectangle(cornerRadius: 11))
                            .overlay(RoundedRectangle(cornerRadius: 11).strokeBorder(open ? MusicStyle.accent.opacity(0.45) : Color.primary.opacity(0.10)))
                    }.buttonStyle(.plain).accessibilityLabel(title).accessibilityIdentifier("choice.\(title)").accessibilityValue(String(describing: selection))
                        .onKeyPress(.downArrow) { focused = options.firstIndex { $0.containerValues.hasTag(selection) } ?? 0; open = true; return .handled }
                        .popover(isPresented: $open, arrowEdge: .bottom) {
                            ScrollViewReader { reader in
                                ScrollView {
                                    LazyVStack(spacing: 3) {
                                        ForEach(Array(options.enumerated()), id: \.element.id) { index, option in
                                            if let tag = option.containerValues.tag(for: Selection.self) {
                                                Button { selection = tag; open = false } label: {
                                                    HStack(spacing: 10) { option.lineLimit(2); Spacer(); if tag == selection { Image(systemName: "checkmark").font(.caption.weight(.semibold)) } }
                                                        .foregroundStyle(tag == selection ? MusicStyle.accent : Color.primary)
                                                        .padding(.horizontal, 12).padding(.vertical, 10).frame(maxWidth: .infinity, alignment: .leading)
                                                        .contentShape(RoundedRectangle(cornerRadius: 9))
                                                        .background((hovered ?? focused) == index ? MusicStyle.accent.opacity(0.13) : .clear, in: RoundedRectangle(cornerRadius: 9))
                                                }.buttonStyle(.plain).id(index).onHover { hovered = $0 ? index : nil }
                                            }
                                        }
                                    }.padding(8)
                                }.frame(width: 300, height: min(360, CGFloat(options.count) * 43 + 16))
                                    .background(MusicStyle.canvas(scheme).opacity(0.95))
                                    .onAppear { reader.scrollTo(focused, anchor: .center); listFocused = true }
                                    .onChange(of: focused) { reader.scrollTo(focused, anchor: .center) }
                                    .focusable().focused($listFocused).onKeyPress(.downArrow) { hovered = nil; focused = min(max(0, options.count - 1), focused + 1); return .handled }
                                    .onKeyPress(.upArrow) { hovered = nil; focused = max(0, focused - 1); return .handled }
                                    .onKeyPress(.return) { if options.indices.contains(focused), let tag = options[focused].containerValues.tag(for: Selection.self) { selection = tag; open = false }; return .handled }
                                    .onExitCommand { open = false }
                            }.preferredColorScheme(scheme)
                        }
                }
            }.font(.system(size: 12, weight: .medium)).opacity(enabled ? 1 : 0.45)
                .animation(reduceMotion ? nil : .easeOut(duration: 0.14), value: open)
        }.onDisappear { open = false }
    }
}

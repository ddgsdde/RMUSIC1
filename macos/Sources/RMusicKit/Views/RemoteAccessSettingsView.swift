import SwiftUI

/// Core-owned direct access configuration.
struct RemoteAccessSettingsView: View {
    let store: MusicStore
    @StoredState private var resource = ResourceStore()
    @StoredState private var draft = ConfigurationDraft()
    @StoredState private var action = WorkflowAction()
    @StoredState private var refresh = UUID()
    @StoredState private var confirmation = false
    @StoredState private var password = ""
    @StoredState private var code = ""
    @StoredState private var cloudflare = ""
    @StoredState private var eab = ""
    @StoredState private var directURLs = ""
    @StoredState private var ingressExpanded = true
    @StoredState private var priorityExpanded = true
    private var transport: String? { resource.value["core_settings_transport"].string }
    private var secure: Bool { transport == "secure" || transport == nil }
    private var canSave: Bool { secure }
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            FeatureCard {
                FeatureHeader(title: "网络与连接", subtitle: "在家优先连接局域网。外出时使用 Core 提供的 HTTPS 入口。", loading: resource.loading) { refresh = UUID() }
                ResourceNotice(resource: resource)
                WorkflowNotice(action: action)
                if !resource.loading && resource.error == nil {
                    DisclosureGroup("配置 Core 对外入口", isExpanded: $ingressExpanded) {
                        if !secure { Text("请重新连接 Core，建立加密通道后即可保存。").font(.caption) }
                        RemoteIngressFields(draft: $draft)
                        DisclosureGroup("HTTPS 连接优先级", isExpanded: $priorityExpanded) {
                            Text("每行一个，最多五个，第一行优先。支持端口和反代路径。自管 HTTPS 或反代主地址如未列出，排在这些地址之后。").font(.caption).foregroundStyle(.secondary)
                            TextEditor(text: $directURLs).font(.system(.body, design: .monospaced)).frame(minHeight: 76)
                            ForEach(Array(resource.value["status"]["direct_urls"].array.enumerated()), id: \.offset) { _, entry in
                                Label(entry["url"].text, systemImage: entry["verified"].boolean ? "checkmark.circle" : "clock")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            let dns = resource.value["status"]["core_ingress"]["ddns"]
                            if let message = dns["message"].string {
                                Text(message).font(.caption).foregroundStyle(.secondary)
                            }
                            if dns["state"].text == "ok" {
                                Text(dns["records"].array.compactMap { $0["address"].string }.joined(separator: " · "))
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        if canSave && draft.get("core.mode").text == "managed_https" {
                            DisclosureGroup("验证服务凭据") {
                                SecureField(resource.value["secrets"]["core_cloudflare"].boolean ? "Cloudflare Token · 已保存，留空保持" : "Cloudflare API Token", text: $cloudflare)
                                SecureField("EAB 密钥 · 可选", text: $eab)
                            }
                        }
                        Button("保存连接配置") { password = ""; code = ""; confirmation = true }.buttonStyle(WorkspacePrimaryButton()).disabled(action.busy || !canSave)
                    }
                }
            }
        }
        .task(id: "\(store.sessionID):\(refresh)") { if await resource.load("/remote-access", store: store), !Task.isCancelled { draft.values = resource.value["configuration"].object; directURLs = draft.get("direct_urls").array.compactMap(\.string).joined(separator: "\n") } }
        .sheet(isPresented: $confirmation) { confirmationForm }
        .onDisappear { action.cancel(); password = ""; code = ""; cloudflare = ""; eab = "" }
    }
    private var confirmationForm: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("验证管理员身份").font(.title2.bold())
            Text("保存后，已连接的设备自动同步有效入口。").foregroundStyle(.secondary)
            SecureField("当前 Core 密码", text: $password)
            if store.user["totp_enabled"].boolean { SecureField("动态码或恢复码", text: $code) }
            WorkflowNotice(action: action)
            HStack { Button("取消") { confirmation = false; password = ""; code = "" }; Spacer(); Button("验证并保存") { save() }.buttonStyle(WorkspacePrimaryButton()).disabled(password.isEmpty || action.busy) }
        }.padding(28).frame(width: 420).interactiveDismissDisabled(action.busy)
    }
    private func save() {
        let entries = directURLs.components(separatedBy: .newlines).map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        draft.set("direct_urls", .array(entries.map(JSONValue.string)))

        var credentials: [String: JSONValue] = ["current_password": .string(password)]
        if !code.isEmpty { credentials["totp"] = .string(code) }
        var secrets: [String: JSONValue] = [:]
        if !cloudflare.isEmpty { secrets["core_cloudflare"] = .string(cloudflare) }
        if !eab.isEmpty { secrets["core_eab"] = .string(eab) }
        guard canSave else { return }
        let edit: [String: JSONValue] = ["expected_revision": resource.value["configuration"]["revision"], "configuration": .object(draft.values), "credentials": .object(credentials), "secrets": .object(secrets)]
        action.send("/remote-access", method: "PUT", body: .object(edit), store: store) { _ in
            password = ""; code = ""; cloudflare = ""; eab = ""; confirmation = false; refresh = UUID()
        }
    }
}

struct RemoteIngressFields: View {
    @Binding var draft: ConfigurationDraft
    private func key(_ value: String) -> String { "core.\(value)" }
    private func text(_ value: String, optional: Bool = false) -> Binding<String> { Binding(get: { draft.get(key(value)).text }, set: { draft.set(key(value), optional && $0.isEmpty ? .null : .string($0)) }) }
    private func flag(_ value: String) -> Binding<Bool> { Binding(get: { draft.get(key(value)).boolean }, set: { draft.set(key(value), .bool($0)) }) }
    private var ipv6Source: Binding<String> {
        Binding(get: {
            if draft.get(key("ddns.ipv6")).string != nil { return "manual" }
            guard draft.get(key("ddns.auto_ipv6")).boolean else { return "off" }
            return draft.get(key("ddns.ipv6_lookup.kind")).string ?? "interface"
        }, set: { choice in
            draft.set(key("ddns.ipv6"), choice == "manual" ? .string("") : .null)
            draft.set(key("ddns.auto_ipv6"), .bool(!["manual", "off"].contains(choice)))
            switch choice {
            case "hostname": draft.set(key("ddns.ipv6_lookup"), .object(["kind": .string(choice), "hostname": .string("")]))
            case "https": draft.set(key("ddns.ipv6_lookup"), .object(["kind": .string(choice), "urls": .array([.string("https://api6.ipify.org")])]))
            default: draft.set(key("ddns.ipv6_lookup"), .null)
            }
        })
    }
    @ViewBuilder private var ipv6Fields: some View {
        ThemePicker("IPv6 地址来源", selection: ipv6Source) {
            Text("本机网卡 · 当前路由").tag("interface")
            Text("参考域名 AAAA").tag("hostname")
            Text("HTTPS 公网查询").tag("https")
            Text("手动指定").tag("manual")
            Text("不更新 AAAA").tag("off")
        }
        switch ipv6Source.wrappedValue {
        case "hostname": TextField("参考域名", text: text("ddns.ipv6_lookup.hostname"))
        case "manual": TextField("公网 IPv6", text: text("ddns.ipv6"))
        case "https":
            TextField("IPv6 查询地址，每行一个", text: Binding(get: {
                draft.get(key("ddns.ipv6_lookup.urls")).array.compactMap(\.string).joined(separator: "\n")
            }, set: { input in
                draft.set(key("ddns.ipv6_lookup.urls"), .array(input.components(separatedBy: .newlines).map(JSONValue.string)))
            }), axis: .vertical).lineLimit(1...3)
        default: EmptyView()
        }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 15) {
            ThemePicker("Core 对外访问方式", selection: text("mode")) {
                Text("局域网入口").tag("local_relay")
                Text("Core 自管 HTTPS").tag("managed_https")
                Text("已有公网入口／反代").tag("reverse_proxy")
            }
            if draft.get(key("mode")).text == "local_relay" { Text("本机只启动现有局域网监听。可在下方发布已经可用的 HTTPS 地址。").font(.caption).foregroundStyle(.secondary) }
            if draft.get(key("mode")).text == "reverse_proxy" {
                TextField("外部 HTTPS 地址 · 含端口和路径", text: text("public_url", optional: true)).textFieldStyle(.roundedBorder)
                Text("IPv6 写成 https://[IPv6]:端口。此模式关闭本节点的 DDNS、证书任务及自管 HTTPS 监听，保留原有后端。").font(.caption).foregroundStyle(.secondary)
                TextField("可信反代的实际 IP · 逗号分隔", text: Binding(get: { draft.get(key("trusted_proxy_addresses")).array.compactMap(\.string).joined(separator: ", ") }, set: { draft.set(key("trusted_proxy_addresses"), .array($0.split(separator: ",").map { .string($0.trimmingCharacters(in: .whitespaces)) })) })).textFieldStyle(.roundedBorder)
                Text("仅信任这里明确填写的代理来源。公网直接映射不需要填写。").font(.caption).foregroundStyle(.secondary)
            }
            if draft.get(key("mode")).text == "managed_https" {
                TextField("HTTPS 域名", text: text("domain")).textFieldStyle(.roundedBorder)
                TextField("监听地址", text: text("listen_address")).textFieldStyle(.roundedBorder)
                DisclosureGroup("证书与续期") {
                    VStack(alignment: .leading, spacing: 12) {
                        ThemePicker("验证方式", selection: text("certificate.challenge")) {
                            Text("Cloudflare DNS · 不依赖 80／443").tag("dns_cloudflare")
                            Text("HTTP · 公网 80").tag("http01")
                            Text("TLS-ALPN · 公网 443").tag("tls_alpn01")
                            Text("导入 PEM 证书").tag("imported")
                            Text("外部文件管理续期").tag("external")
                        }
                        if ["imported", "external"].contains(draft.get(key("certificate.challenge")).text) {
                            TextField("证书文件路径", text: text("certificate.external_cert", optional: true))
                            TextField("私钥文件路径", text: text("certificate.external_key", optional: true))
                        } else {
                            TextField("证书联系邮箱", text: text("certificate.email"))
                            TextField("ACME 服务地址", text: text("certificate.directory"))
                            TextField("EAB 标识 · 可选", text: text("certificate.eab_kid", optional: true))
                        }
                        if draft.get(key("certificate.challenge")).text == "dns_cloudflare" {
                            TextField("DNS 验证服务器 · 可选 IP:端口，逗号分隔", text: Binding(get: { draft.get(key("certificate.dns_resolvers")).array.map(\.text).joined(separator: ", ") }, set: { draft.set(key("certificate.dns_resolvers"), .array($0.split(separator: ",").map { .string($0.trimmingCharacters(in: .whitespaces)) })) }))
                            TextField("DNS 传播等待秒数 · 0 自动检查", value: Binding(get: { draft.get(key("certificate.dns_wait_seconds")).number }, set: { draft.set(key("certificate.dns_wait_seconds"), .number($0)) }), format: .number)
                            Text("系统 DNS 受限时可指定验证服务器或等待时间；CA 仍验证域名，HTTPS 验证保持开启。").font(.caption).foregroundStyle(.secondary)
                        }
                        Text("业务端口可自定义；公网验证端口由证书协议规定，无法开放时选择 DNS 验证。").font(.caption).foregroundStyle(.secondary)
                    }.textFieldStyle(.roundedBorder).padding(.vertical, 10)
                }
                DisclosureGroup("Cloudflare 动态域名 · 可选") {
                    VStack(alignment: .leading, spacing: 12) {
                        Toggle("自动维护所选域名的 A／AAAA", isOn: flag("ddns.enabled"))
                        if draft.get(key("ddns.enabled")).boolean {
                            TextField("Zone ID", text: text("ddns.zone_id"))
                            ipv6Fields
                            Toggle("已确认 Core 的 IPv4 可以入站", isOn: flag("ddns.ipv4_inbound_confirmed"))
                            Toggle("探测并更新公网 IPv4", isOn: flag("ddns.auto_ipv4"))
                            TextField("固定 IPv4 · 可选", text: text("ddns.ipv4", optional: true))
                            TextField("指定网卡 · 可选", text: text("ddns.interface", optional: true))
                            Toggle("允许接管同名现有记录", isOn: flag("ddns.adopt_existing"))
                        }
                    }.textFieldStyle(.roundedBorder).padding(.vertical, 10)
                }
            }
            DisclosureGroup("HTTPS 与证书端口") {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(ports, id: \.0) { port in
                        HStack { Text(port.1); Spacer(); TextField(port.1, value: Binding(get: { draft.get(key("ports.\(port.0)")).number }, set: { draft.set(key("ports.\(port.0)"), .number($0)) }), format: .number.grouping(.never)).frame(width: 110) }
                    }
                    Text("监听与对外端口分别设置，范围 1–65535；不会自动改动路由器或停止其他服务。").font(.caption).foregroundStyle(.secondary)
                }.textFieldStyle(.roundedBorder).padding(.vertical, 10)
            }
        }
    }
    private var ports: [(String, String)] { [("https_listen", "HTTPS 监听"), ("https_public", "HTTPS 对外端口"), ("acme_http_listen", "ACME HTTP 本机验证端口"), ("acme_tls_listen", "ACME TLS 验证端口 · 0 自动")] }
}

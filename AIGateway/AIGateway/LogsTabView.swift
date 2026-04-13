import SwiftUI

struct LogsTabView: View {
    @ObservedObject var viewModel: GatewayViewModel
    @Environment(\.colorScheme) private var colorScheme
    @State private var showingClearConfirmation = false

    var body: some View {
        NavigationStack {
            HStack(spacing: 18) {
                logListPanel
                logDetailPanel
            }
            .padding(24)
            .background(background)
            .navigationTitle("请求日志")
            .confirmationDialog(
                "清空所有日志？",
                isPresented: $showingClearConfirmation,
                titleVisibility: .visible
            ) {
                Button("清空全部", role: .destructive) {
                    Task {
                        await viewModel.clearLogs()
                    }
                }
                Button("取消", role: .cancel) {}
            } message: {
                Text("这会删除 `log.db` 里的所有日志记录，当前列表也会立即清空。")
            }
        }
        .task {
            if viewModel.logs.isEmpty {
                await viewModel.refreshLogs()
            }
        }
    }

    private var logListPanel: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .center) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("日志列表")
                        .font(.system(size: 28, weight: .bold, design: .rounded))
                    Text("查看每个 `request_id` 的完整入口、出口和错误链路。")
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(.secondary)
                }

                Spacer()

                HStack(spacing: 12) {
                    Toggle(
                        isOn: Binding(
                            get: { viewModel.isLoggingEnabled },
                            set: { enabled in
                                Task {
                                    await viewModel.setLoggingEnabled(enabled)
                                }
                            }
                        )
                    ) {
                        Text("记录日志")
                            .font(.system(size: 13, weight: .semibold))
                    }
                    .toggleStyle(.switch)

                    Button {
                        Task {
                            await viewModel.refreshLogs()
                        }
                    } label: {
                        Label("刷新", systemImage: "arrow.clockwise")
                    }
                    .buttonStyle(.bordered)

                    Button(role: .destructive) {
                        showingClearConfirmation = true
                    } label: {
                        Label("清空", systemImage: "trash")
                    }
                    .buttonStyle(.bordered)
                    .disabled(viewModel.logs.isEmpty)
                }
            }

            HStack(spacing: 10) {
                statusBadge(
                    title: viewModel.isLoggingEnabled ? "记录中" : "已暂停",
                    tint: viewModel.isLoggingEnabled ? activeTint : pausedTint
                )
                statusBadge(
                    title: "\(viewModel.logs.count) 条请求",
                    tint: secondaryTint
                )
            }

            Group {
                if viewModel.logs.isEmpty && !viewModel.isLogsLoading {
                    ContentUnavailableView(
                        "还没有日志",
                        systemImage: "text.page.slash",
                        description: Text(
                            viewModel.isLoggingEnabled
                                ? "等网关收到请求后，这里会显示每个 request_id 的摘要。"
                                : "日志记录当前已关闭，打开右上角开关后才会继续写入。"
                        )
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    List(selection: selectedLogBinding) {
                        ForEach(viewModel.logs) { log in
                            logRow(log)
                                .tag(log.requestID)
                                .listRowSeparator(.hidden)
                                .listRowBackground(Color.clear)
                        }
                    }
                    .listStyle(.plain)
                    .scrollContentBackground(.hidden)
                    .background(Color.clear)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .padding(22)
        .frame(minWidth: 360, maxWidth: 420, maxHeight: .infinity, alignment: .topLeading)
        .background(panelBackground)
        .overlay(panelBorder)
        .shadow(color: panelShadow, radius: 18, x: 0, y: 10)
    }

    private var logDetailPanel: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .center) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("日志详情")
                        .font(.system(size: 28, weight: .bold, design: .rounded))
                    Text("按时间顺序查看每一步事件、状态码、URL 和请求/响应体。")
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(.secondary)
                }

                Spacer()

                if viewModel.isLogDetailLoading {
                    ProgressView()
                        .controlSize(.small)
                } else if viewModel.selectedLogRequestID != nil {
                    Button {
                        Task {
                            await viewModel.refreshSelectedLogDetail()
                        }
                    } label: {
                        Label("刷新详情", systemImage: "arrow.clockwise")
                    }
                    .buttonStyle(.bordered)
                }
            }

            if let detail = viewModel.selectedLogDetail {
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        detailHeader(detail)

                        ForEach(detail.events) { event in
                            eventCard(event)
                        }
                    }
                    .padding(.bottom, 8)
                }
                .scrollContentBackground(.hidden)
            } else if viewModel.isLogsLoading || viewModel.isLogDetailLoading {
                VStack {
                    Spacer()
                    ProgressView("正在读取日志详情…")
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ContentUnavailableView(
                    "请选择一条日志",
                    systemImage: "sidebar.left",
                    description: Text("左侧选中一个 request_id 后，这里会显示完整事件链路。")
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .padding(22)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(panelBackground)
        .overlay(panelBorder)
        .shadow(color: panelShadow, radius: 18, x: 0, y: 10)
    }

    private func logRow(_ log: GatewayLogSummary) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(log.providerName ?? "未知供应商")
                        .font(.system(size: 16, weight: .bold, design: .rounded))
                        .lineLimit(1)

                    Text(log.requestID)
                        .font(.system(size: 11, weight: .medium, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .lineLimit(1)
                }

                Spacer()

                statusPill(log)
            }

            HStack(spacing: 8) {
                compactPill(log.stream ? "SSE" : "JSON", tint: secondaryTint)
                compactPill(log.model ?? "未知模型", tint: modelTint)
                compactPill("\(log.eventCount) 事件", tint: secondaryTint)
            }

            if let errorMessage = log.errorMessage, !errorMessage.isEmpty {
                Text(errorMessage)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.red)
                    .lineLimit(2)
            } else if let accountEmail = log.accountEmail {
                Text(accountEmail)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Text(log.updatedDate.formatted(.dateTime.month().day().hour().minute().second()))
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(rowBackground(for: log))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(rowBorder(for: log), lineWidth: 1)
        )
        .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .onTapGesture {
            Task {
                await viewModel.selectLog(requestID: log.requestID)
            }
        }
    }

    private func detailHeader(_ detail: GatewayLogDetail) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(detail.requestID)
                .font(.system(size: 14, weight: .semibold, design: .monospaced))
                .textSelection(.enabled)

            if let summary = viewModel.logs.first(where: { $0.requestID == detail.requestID }) {
                HStack(spacing: 10) {
                    compactPill(summary.providerName ?? "未知供应商", tint: secondaryTint)
                    compactPill(summary.model ?? "未知模型", tint: modelTint)
                    compactPill(summary.stream ? "流式" : "非流式", tint: secondaryTint)
                    if let statusCode = summary.statusCode {
                        compactPill("HTTP \(statusCode)", tint: summary.hasError ? .red : activeTint)
                    }
                }
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .fill(sectionBackground)
        )
    }

    private func eventCard(_ event: GatewayLogEvent) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(stageTitle(event.stage))
                        .font(.system(size: 16, weight: .bold, design: .rounded))

                    Text(event.createdDate.formatted(.dateTime.month().day().hour().minute().second()))
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.secondary)
                }

                Spacer()

                HStack(spacing: 8) {
                    if let statusCode = event.statusCode {
                        compactPill("HTTP \(statusCode)", tint: colorForStatus(statusCode))
                    }

                    if let elapsedMS = event.elapsedMS {
                        compactPill("\(elapsedMS) ms", tint: secondaryTint)
                    }
                }
            }

            LazyVGrid(columns: [GridItem(.adaptive(minimum: 180), spacing: 10)], alignment: .leading, spacing: 10) {
                if let providerName = event.providerName {
                    detailMeta(title: "供应商", value: providerName)
                }
                if let accountEmail = event.accountEmail {
                    detailMeta(title: "账号", value: accountEmail)
                }
                if let model = event.model {
                    detailMeta(title: "模型", value: model)
                }
                if let method = event.method {
                    detailMeta(title: "方法", value: method)
                }
                if let path = event.path {
                    detailMeta(title: "路径", value: path)
                }
                if let url = event.url {
                    detailMeta(title: "URL", value: url)
                }
                if let ingressProtocol = event.ingressProtocol {
                    detailMeta(title: "入口协议", value: ingressProtocol)
                }
                if let egressProtocol = event.egressProtocol {
                    detailMeta(title: "出口协议", value: egressProtocol)
                }
            }

            if let errorMessage = event.errorMessage, !errorMessage.isEmpty {
                detailBlock(
                    title: event.errorTruncated ? "错误信息（已截断）" : "错误信息",
                    text: errorMessage,
                    tint: .red
                )
            }

            if let body = event.body, !body.isEmpty {
                detailBlock(
                    title: event.bodyTruncated ? "Body（已截断）" : "Body",
                    text: body,
                    tint: modelTint
                )
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .fill(sectionBackground)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .stroke(sectionBorder, lineWidth: 1)
        )
    }

    private func detailMeta(title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(title)
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(.secondary)
            Text(value)
                .font(.system(size: 12, weight: .medium, design: .monospaced))
                .textSelection(.enabled)
                .lineLimit(3)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func detailBlock(title: String, text: String, tint: Color) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(tint)

            if let jsonValue = JSONTreeCache.shared.value(for: text) {
                JSONTreeView(root: jsonValue, tint: tint)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                    .background(
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .fill(codeBlockBackground(tint: tint))
                    )
            } else {
                ScrollView(.horizontal) {
                    Text(text)
                        .font(.system(size: 12, weight: .medium, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                .padding(12)
                .background(
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .fill(codeBlockBackground(tint: tint))
                )
            }
        }
    }

    private func statusBadge(title: String, tint: Color) -> some View {
        Text(title)
            .font(.system(size: 11, weight: .bold))
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(tint.opacity(0.16))
            .foregroundStyle(tint)
            .clipShape(Capsule())
    }

    private func compactPill(_ title: String, tint: Color) -> some View {
        Text(title)
            .font(.system(size: 11, weight: .semibold))
            .padding(.horizontal, 9)
            .padding(.vertical, 5)
            .background(tint.opacity(0.12))
            .foregroundStyle(tint)
            .clipShape(Capsule())
    }

    private func statusPill(_ log: GatewayLogSummary) -> some View {
        let tint: Color = log.hasError ? .red : activeTint
        let title: String
        if let statusCode = log.statusCode {
            title = "HTTP \(statusCode)"
        } else {
            title = log.hasError ? "错误" : "完成"
        }
        return compactPill(title, tint: tint)
    }

    private func stageTitle(_ stage: String) -> String {
        switch stage {
        case "ingress_request":
            return "入口请求"
        case "egress_request":
            return "出口请求"
        case "ingress_response":
            return "入口响应"
        case "egress_response":
            return "出口响应"
        case "error":
            return "错误"
        default:
            return stage
        }
    }

    private func colorForStatus(_ statusCode: Int) -> Color {
        switch statusCode {
        case 200 ..< 300:
            return activeTint
        case 400 ..< 500:
            return Color.orange
        default:
            return .red
        }
    }

    private var selectedLogBinding: Binding<String?> {
        Binding(
            get: { viewModel.selectedLogRequestID },
            set: { requestID in
                guard let requestID else {
                    viewModel.selectedLogRequestID = nil
                    viewModel.selectedLogDetail = nil
                    return
                }

                Task {
                    await viewModel.selectLog(requestID: requestID)
                }
            }
        )
    }

    private func rowBackground(for log: GatewayLogSummary) -> Color {
        if log.requestID == viewModel.selectedLogRequestID {
            return colorScheme == .dark ? activeTint.opacity(0.24) : activeTint.opacity(0.14)
        }
        return colorScheme == .dark
            ? Color.white.opacity(0.045)
            : Color.primary.opacity(0.035)
    }

    private func rowBorder(for log: GatewayLogSummary) -> Color {
        if log.requestID == viewModel.selectedLogRequestID {
            return colorScheme == .dark ? activeTint.opacity(0.72) : activeTint.opacity(0.55)
        }
        return colorScheme == .dark
            ? Color.white.opacity(0.08)
            : Color.white.opacity(0.05)
    }

    private var background: some View {
        LinearGradient(
            colors: [
                colorScheme == .dark
                    ? Color(red: 0.08, green: 0.10, blue: 0.13)
                    : Color(red: 0.95, green: 0.97, blue: 0.99),
                colorScheme == .dark
                    ? Color(red: 0.06, green: 0.13, blue: 0.11)
                    : Color(red: 0.91, green: 0.95, blue: 0.94)
            ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
        .ignoresSafeArea()
    }

    private var panelBackground: some View {
        RoundedRectangle(cornerRadius: 28, style: .continuous)
            .fill(
                colorScheme == .dark
                    ? Color(red: 0.11, green: 0.14, blue: 0.17).opacity(0.94)
                    : Color.white.opacity(0.84)
            )
    }

    private var panelBorder: some View {
        RoundedRectangle(cornerRadius: 28, style: .continuous)
            .stroke(
                colorScheme == .dark
                    ? Color.white.opacity(0.10)
                    : Color.white.opacity(0.7),
                lineWidth: 1
            )
    }

    private var panelShadow: Color {
        colorScheme == .dark ? .black.opacity(0.28) : .black.opacity(0.08)
    }

    private var sectionBackground: Color {
        colorScheme == .dark
            ? Color.white.opacity(0.05)
            : Color.primary.opacity(0.045)
    }

    private var sectionBorder: Color {
        colorScheme == .dark
            ? Color.white.opacity(0.08)
            : Color.white.opacity(0.06)
    }

    private func codeBlockBackground(tint: Color) -> Color {
        colorScheme == .dark ? tint.opacity(0.14) : tint.opacity(0.08)
    }

    private var activeTint: Color {
        colorScheme == .dark
            ? Color(red: 0.33, green: 0.78, blue: 0.52)
            : Color(red: 0.15, green: 0.63, blue: 0.38)
    }

    private var pausedTint: Color {
        colorScheme == .dark
            ? Color(red: 0.95, green: 0.68, blue: 0.28)
            : Color(red: 0.86, green: 0.48, blue: 0.16)
    }

    private var secondaryTint: Color {
        colorScheme == .dark
            ? Color(red: 0.67, green: 0.75, blue: 0.86)
            : Color(red: 0.33, green: 0.42, blue: 0.55)
    }

    private var modelTint: Color {
        colorScheme == .dark
            ? Color(red: 0.45, green: 0.66, blue: 1.00)
            : Color(red: 0.18, green: 0.45, blue: 0.88)
    }
}

private struct JSONTreeView: View {
    let root: JSONTreeValue
    let tint: Color

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            JSONTreeNodeView(label: nil, value: root, tint: tint, level: 0)
        }
    }
}

private struct JSONTreeNodeView: View {
    let label: String?
    let value: JSONTreeValue
    let tint: Color
    let level: Int

    @State private var isExpanded: Bool

    init(label: String?, value: JSONTreeValue, tint: Color, level: Int) {
        self.label = label
        self.value = value
        self.tint = tint
        self.level = level
        _isExpanded = State(initialValue: value.shouldAutoExpand(at: level))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if value.isContainer {
                DisclosureGroup(isExpanded: $isExpanded) {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(value.children) { child in
                            JSONTreeNodeView(
                                label: child.label,
                                value: child.value,
                                tint: tint,
                                level: level + 1
                            )
                        }
                    }
                    .padding(.top, 6)
                } label: {
                    headerRow
                }
                .tint(tint)
            } else {
                leafRow
            }
        }
        .padding(.leading, CGFloat(level) * 14)
    }

    private var headerRow: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            if let label {
                Text(label)
                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .foregroundStyle(.primary)
                    .textSelection(.enabled)
            }

            Text(value.containerSummary)
                .font(.system(size: 11, weight: .medium, design: .monospaced))
                .foregroundStyle(.secondary)
        }
    }

    private var leafRow: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            if let label {
                Text(label)
                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .foregroundStyle(.primary)
                    .textSelection(.enabled)
            }

            Text(value.renderedValue)
                .font(.system(size: 12, weight: .medium, design: .monospaced))
                .foregroundStyle(value.foregroundStyle)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct JSONTreeChild: Identifiable {
    let id: String
    let label: String
    let value: JSONTreeValue
}

private enum JSONTreeValue {
    case object([(String, JSONTreeValue)])
    case array([JSONTreeValue])
    case string(String)
    case number(String)
    case bool(Bool)
    case null

    static func parse(from text: String) -> JSONTreeValue? {
        guard let data = text.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) else {
            return nil
        }
        return make(from: object)
    }

    private static func make(from raw: Any) -> JSONTreeValue? {
        switch raw {
        case let dictionary as [String: Any]:
            let entries = dictionary.keys.sorted().map { key in
                (key, make(from: dictionary[key] ?? NSNull()) ?? .null)
            }
            return .object(entries)
        case let array as [Any]:
            return .array(array.map { make(from: $0) ?? .null })
        case let string as String:
            return .string(string)
        case let bool as Bool:
            return .bool(bool)
        case let number as NSNumber:
            if CFGetTypeID(number) == CFBooleanGetTypeID() {
                return .bool(number.boolValue)
            }
            return .number(number.stringValue)
        case _ as NSNull:
            return .null
        default:
            return nil
        }
    }

    var isContainer: Bool {
        switch self {
        case .object, .array:
            return true
        case .string, .number, .bool, .null:
            return false
        }
    }

    var children: [JSONTreeChild] {
        switch self {
        case .object(let entries):
            return entries.map { key, value in
                JSONTreeChild(id: key, label: key, value: value)
            }
        case .array(let values):
            return values.enumerated().map { index, value in
                JSONTreeChild(id: "[\(index)]", label: "[\(index)]", value: value)
            }
        case .string, .number, .bool, .null:
            return []
        }
    }

    var containerSummary: String {
        switch self {
        case .object(let entries):
            return "{\(entries.count) fields}"
        case .array(let values):
            return "[\(values.count) items]"
        case .string, .number, .bool, .null:
            return renderedValue
        }
    }

    var renderedValue: String {
        switch self {
        case .string(let value):
            return "\"\(value)\""
        case .number(let value):
            return value
        case .bool(let value):
            return value ? "true" : "false"
        case .null:
            return "null"
        case .object:
            return "{}"
        case .array:
            return "[]"
        }
    }

    var foregroundStyle: Color {
        switch self {
        case .string:
            return .primary
        case .number:
            return .blue
        case .bool:
            return .orange
        case .null:
            return .secondary
        case .object, .array:
            return .secondary
        }
    }

    var childCount: Int {
        switch self {
        case .object(let entries):
            return entries.count
        case .array(let values):
            return values.count
        case .string, .number, .bool, .null:
            return 0
        }
    }

    func shouldAutoExpand(at level: Int) -> Bool {
        guard isContainer else { return false }
        if level == 0 { return true }
        if level == 1 { return childCount <= 12 }
        return false
    }
}

private final class JSONTreeCache {
    static let shared = JSONTreeCache()

    private let cache = NSCache<NSString, Box>()

    private init() {
        cache.countLimit = 96
    }

    func value(for text: String) -> JSONTreeValue? {
        let key = text as NSString
        if let cached = cache.object(forKey: key) {
            return cached.value
        }

        let parsed = JSONTreeValue.parse(from: text)
        cache.setObject(Box(value: parsed), forKey: key)
        return parsed
    }

    private final class Box: NSObject {
        let value: JSONTreeValue?

        init(value: JSONTreeValue?) {
            self.value = value
        }
    }
}

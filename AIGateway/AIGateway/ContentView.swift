//
//  ContentView.swift
//  AIGateway
//
//  Created by 刘子豪 on 2026/4/11.
//

import AppKit
import SwiftUI

struct ContentView: View {
    @Environment(\.colorScheme) private var colorScheme
    @StateObject private var viewModel = GatewayViewModel()
    @StateObject private var serviceSupervisor = GatewayServiceSupervisor()
    @State private var showingAddProvider = false
    @State private var showingCodexConfigSheet = false
    @State private var modelRefreshRotation: Double = 0
    private let gridColumns = [
        GridItem(.adaptive(minimum: 280, maximum: 360), spacing: 18)
    ]

    var body: some View {
        NavigationStack {
            VStack(spacing: 18) {
                header
                topControlBar
                providerGrid
                footer
            }
            .padding(24)
            .background(background)
            .navigationTitle("AI Gateway")
        }
        .sheet(isPresented: $showingAddProvider) {
            AddProviderSheet(viewModel: viewModel)
        }
        .sheet(isPresented: $showingCodexConfigSheet) {
            CodexConfigSheet(viewModel: viewModel)
        }
        .task {
            await initialLoad()
        }
        .alert("Request Failed", isPresented: errorPresented) {
            Button("OK") {
                viewModel.dismissError()
            }
        } message: {
            Text(viewModel.errorMessage ?? "Unknown error")
        }
        .frame(minWidth: 980, minHeight: 680)
    }

    private var header: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 10) {
                Text("供应商")
                    .font(.system(size: 32, weight: .bold, design: .rounded))
                Text("管理 API 供应商、发起 Google / OpenAI 账号登录，并切换当前选中的供应商。")
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(.secondary)

                if let selected = viewModel.selectedProviderName {
                    Label("当前选中的供应商：\(selected)", systemImage: "checkmark.circle.fill")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(selectionAccent)
                } else {
                    Label("当前未选择供应商", systemImage: "circle.dashed")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }

            Spacer()

            HStack(spacing: 10) {
                Button {
                    Task {
                        await refreshAll()
                    }
                } label: {
                    Label("刷新", systemImage: "arrow.clockwise")
                }
                .buttonStyle(.bordered)

                Button {
                    showingAddProvider = true
                } label: {
                    Label("添加供应商", systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)

                Button {
                    showingCodexConfigSheet = true
                } label: {
                    Label("CodeX 配置", systemImage: "doc.badge.gearshape")
                }
                .buttonStyle(.bordered)
            }
        }
    }

    private var topControlBar: some View {
        HStack(alignment: .center, spacing: 18) {
            servicePanel

            Divider()
                .frame(height: 24)
                .overlay(cardBorder.opacity(colorScheme == .dark ? 0.9 : 0.6))

            modelSelector
        }
        .controlSize(.small)
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(
                    LinearGradient(
                        colors: [
                            cardTop.opacity(colorScheme == .dark ? 0.9 : 0.94),
                            cardBottom.opacity(colorScheme == .dark ? 0.88 : 0.9),
                        ],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                )
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .strokeBorder(cardBorder.opacity(colorScheme == .dark ? 1 : 0.8), lineWidth: 1)
        )
    }

    private var servicePanel: some View {
        HStack(alignment: .center, spacing: 14) {
            Text("网关")
                .font(.system(size: 11, weight: .semibold, design: .rounded))
                .foregroundStyle(.secondary)

            HStack(spacing: 8) {
                Circle()
                    .fill(serviceStatusColor)
                    .frame(width: 7, height: 7)

                Text(serviceSupervisor.statusTitle)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(serviceStatusColor)
            }
            .padding(.horizontal, 11)
            .padding(.vertical, 7)
            .background(serviceStatusColor.opacity(colorScheme == .dark ? 0.18 : 0.1))
            .clipShape(Capsule())

            if serviceSupervisor.isBusy {
                ProgressView()
                    .controlSize(.small)
            }

            Spacer(minLength: 12)

            HStack(spacing: 10) {
                Button {
                    Task {
                        if serviceSupervisor.canStop {
                            await stopManagedService()
                        } else {
                            await startService()
                        }
                    }
                } label: {
                    Image(systemName: serviceSupervisor.isReachable ? "stop.fill" : "play.fill")
                        .font(.system(size: 11, weight: .semibold))
                        .frame(width: 16, height: 16)
                }
                .buttonStyle(.bordered)
                .help(serviceSupervisor.isReachable ? "停止服务" : "启动服务")
                .accessibilityLabel(serviceSupervisor.isReachable ? "停止服务" : "启动服务")
                .disabled(!serviceSupervisor.canStart && !serviceSupervisor.canStop)

                Button {
                    viewModel.openDebugDashboard()
                } label: {
                    Image(systemName: "ladybug")
                        .font(.system(size: 11, weight: .semibold))
                        .frame(width: 16, height: 16)
                }
                .buttonStyle(.bordered)
                .help("打开调试页")
                .accessibilityLabel("打开调试页")
                .disabled(!serviceSupervisor.isReachable)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var providerTable: some View {
        EmptyView()
    }

    private var modelSelector: some View {
        HStack(spacing: 14) {
            Text("模型")
                .font(.system(size: 11, weight: .semibold, design: .rounded))
                .foregroundStyle(.secondary)

            Picker("模型", selection: modelSelectionBinding) {
                Text("跟随请求模型").tag("")
                ForEach(viewModel.availableModels) { model in
                    Text(model.id).tag(model.id)
                }
            }
            .pickerStyle(.menu)
            .labelsHidden()
            .frame(minWidth: 180, idealWidth: 220, maxWidth: 240)
            .disabled(viewModel.availableModels.isEmpty || viewModel.isLoadingModels)

            Button {
                Task {
                    await viewModel.refreshModels(forceRefresh: true, userInitiated: true)
                }
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 11, weight: .semibold))
                    .frame(width: 16, height: 16)
                    .rotationEffect(.degrees(modelRefreshRotation))
            }
            .help("刷新模型列表")
            .buttonStyle(.bordered)
            .disabled(viewModel.selectedProviderID == nil || viewModel.isLoadingModels)
        }
        .fixedSize(horizontal: true, vertical: false)
        .onChange(of: viewModel.showsModelRefreshActivity) { isActive in
            if isActive {
                modelRefreshRotation = 0
                withAnimation(.linear(duration: 0.9).repeatForever(autoreverses: false)) {
                    modelRefreshRotation = 360
                }
            } else {
                withAnimation(.easeOut(duration: 0.2)) {
                    modelRefreshRotation = 0
                }
            }
        }
    }

    private var modelSelectionBinding: Binding<String> {
        Binding(
            get: { viewModel.selectedModelID ?? "" },
            set: { modelID in
                guard modelID != (viewModel.selectedModelID ?? "") else { return }
                Task {
                    if modelID.isEmpty {
                        await viewModel.clearSelectedModel()
                    } else {
                        await viewModel.selectModel(id: modelID)
                    }
                }
            }
        )
    }

    private var providerGrid: some View {
        ScrollView {
            if !serviceSupervisor.isReachable && !viewModel.isLoading {
                ContentUnavailableView(
                    "服务未连接",
                    systemImage: "bolt.horizontal.circle",
                    description: Text("先启动网关服务，再加载供应商和模型信息。")
                )
                .frame(maxWidth: .infinity, minHeight: 420)
            } else if viewModel.providers.isEmpty && !viewModel.isLoading {
                ContentUnavailableView(
                    "还没有供应商",
                    systemImage: "tray",
                    description: Text("添加一个 API 供应商，或者用账号登录自动生成供应商。")
                )
                .frame(maxWidth: .infinity, minHeight: 420)
            } else {
                LazyVGrid(columns: gridColumns, alignment: .leading, spacing: 18) {
                    ForEach(viewModel.providers) { provider in
                        providerCard(provider)
                    }
                }
                .padding(.vertical, 6)
            }
        }
        .scrollContentBackground(.hidden)
    }

    private var footer: some View {
        HStack {
            if serviceSupervisor.isBusy {
                ProgressView()
                    .controlSize(.small)
                Text("Waiting for gateway service...")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.secondary)
            } else if viewModel.isLoading {
                ProgressView()
                    .controlSize(.small)
                Text("Syncing with gateway...")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.secondary)
            } else {
                Text("Gateway Base URL: \(viewModel.baseURL.absoluteString)")
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(.secondary)
            }

            Spacer()
        }
    }

    private func authBadge(for provider: GatewayProvider) -> some View {
        Text(provider.authMode == .apiKey ? "API Key" : "账户")
            .font(.system(size: 11, weight: .bold))
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(provider.authMode == .apiKey ? apiBadgeBackground : accountBadgeBackground)
            .foregroundStyle(provider.authMode == .apiKey ? apiAccent : accountAccent)
            .clipShape(Capsule())
    }

    @ViewBuilder
    private func providerCard(_ provider: GatewayProvider) -> some View {
        let isSelected = provider.id == viewModel.selectedProviderID

        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(provider.name)
                        .font(.system(size: 20, weight: .bold, design: .rounded))
                        .foregroundStyle(.primary)
                        .lineLimit(2)

                    HStack(spacing: 8) {
                        authBadge(for: provider)

                        Text(provider.billingModeLabel)
                            .font(.system(size: 11, weight: .semibold))
                            .padding(.horizontal, 9)
                            .padding(.vertical, 6)
                            .background(billingBadgeBackground(for: provider))
                            .foregroundStyle(billingBadgeForeground(for: provider))
                            .clipShape(Capsule())
                    }
                }

                Spacer()

                if isSelected {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 22, weight: .bold))
                        .foregroundStyle(selectionAccent)
                }
            }

            if provider.authMode == .account {
                providerMetaRow(
                    title: "邮箱",
                    value: provider.accountEmail ?? "等待登录完成",
                    emphasized: true
                )
            }

            providerQuotaSection(for: provider)
        }
        .padding(20)
        .frame(maxWidth: .infinity, minHeight: 245, alignment: .topLeading)
        .background(cardBackground(isSelected: isSelected))
        .overlay(
            RoundedRectangle(cornerRadius: 24, style: .continuous)
                .strokeBorder(
                    isSelected
                        ? selectionAccent.opacity(colorScheme == .dark ? 0.95 : 0.78)
                        : cardBorder,
                    lineWidth: isSelected ? 2 : 1
                )
        )
        .shadow(color: shadowColor.opacity(isSelected ? 0.34 : 0.18), radius: isSelected ? 22 : 12, x: 0, y: 12)
        .scaleEffect(isSelected ? 1.01 : 1.0)
        .animation(.spring(response: 0.26, dampingFraction: 0.85), value: isSelected)
        .contentShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
        .onTapGesture {
            guard !isSelected, !viewModel.isLoading else { return }
            Task {
                await viewModel.selectProvider(id: provider.id)
            }
        }
    }

    private func providerMetaRow(title: String, value: String, emphasized: Bool) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(title.uppercased())
                .font(.system(size: 10, weight: .bold))
                .tracking(0.8)
                .foregroundStyle(.secondary)

            Text(value)
                .font(emphasized ? .system(size: 12, weight: .semibold, design: .monospaced) : .system(size: 12))
                .foregroundStyle(emphasized ? .primary : .secondary)
                .lineLimit(2)
                .textSelection(.enabled)
        }
    }

    private func providerQuotaSection(for provider: GatewayProvider) -> some View {
        Group {
            if !provider.supportsQuotaDisplay {
                EmptyView()
            } else if viewModel.isLoadingQuota(for: provider.id) {
                quotaPanel(
                    title: "额度窗口",
                    headline: "同步中",
                    headlineTint: .secondary
                ) {
                    Text("正在读取当前供应商的额度快照")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            } else if let error = viewModel.quotaErrorMessage(for: provider.id) {
                quotaPanel(
                    title: "额度窗口",
                    headline: "读取失败",
                    headlineTint: .red
                ) {
                    Text(error)
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            } else if let summary = viewModel.quotaSummary(for: provider.id) {
                if summary.status == .unsupported {
                    quotaPanel(
                        title: "额度窗口",
                        headline: "暂不支持",
                        headlineTint: .secondary
                    ) {
                        Text(summary.message ?? "当前供应商暂不支持额度快照")
                            .font(.system(size: 11, weight: .medium))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                } else {
                    supportedQuotaPanel(summary: summary)
                }
            } else {
                quotaPanel(
                    title: "额度窗口",
                    headline: "暂无数据",
                    headlineTint: .secondary
                ) {
                    Text("还没有拿到额度信息")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
        }
    }

    @ViewBuilder
    private func supportedQuotaPanel(summary: ProviderQuotaSummary) -> some View {
        let primary = summary.snapshot?.primary
        let secondary = summary.snapshot?.secondary

        if primary == nil && secondary == nil {
            quotaPanel(
                title: "额度窗口",
                headline: "可用",
                headlineTint: selectionAccent
            ) {
                Text("额度接口已接通，但当前没有可视化窗口数据")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        } else {
            let headlineTint = quotaTint(forRemainingPercent: (primary ?? secondary)!.remainingPercent)
            quotaPanel(
                title: "额度窗口",
                headline: quotaHeadline(primary: primary, secondary: secondary),
                headlineTint: headlineTint
            ) {
                VStack(alignment: .leading, spacing: 10) {
                    if let primary {
                        quotaWindowRow(
                            title: quotaWindowTitle(minutes: primary.windowMinutes, fallback: "五小时窗口"),
                            window: primary
                        )
                    }

                    if let secondary {
                        quotaWindowRow(
                            title: quotaWindowTitle(minutes: secondary.windowMinutes, fallback: "周窗口"),
                            window: secondary
                        )
                    }

                    if let footnote = quotaCreditsFootnoteText(summary: summary) {
                        Text(footnote)
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                }
            }
        }
    }

    private func quotaHeadline(primary: ProviderQuotaWindow?, secondary: ProviderQuotaWindow?) -> String {
        var parts: [String] = []
        if let primary {
            parts.append("5h \(Int(primary.remainingPercent.rounded()))%")
        }
        if let secondary {
            parts.append("周 \(Int(secondary.remainingPercent.rounded()))%")
        }
        return parts.isEmpty ? "可用" : parts.joined(separator: " · ")
    }

    private func quotaCreditsFootnoteText(summary: ProviderQuotaSummary) -> String? {
        var footnotes: [String] = []

        if summary.hasUnlimitedCredits {
            footnotes.append("账户余额无限")
        } else if let balance = summary.creditBalance {
            footnotes.append("余额 \(balance)")
        }

        if let plan = summary.snapshot?.planType, !plan.isEmpty {
            footnotes.append("Plan \(plan)")
        }

        return footnotes.isEmpty ? nil : footnotes.joined(separator: " · ")
    }

    private func quotaWindowTitle(minutes: Int?, fallback: String) -> String {
        guard let minutes else { return fallback }
        if minutes == 300 { return "五小时窗口" }
        if minutes >= 7 * 24 * 60 { return "周窗口" }
        return quotaWindowDescription(minutes: minutes)
    }

    private func quotaWindowRow(title: String, window: ProviderQuotaWindow) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(title)
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(.secondary)

                Spacer()

                Text("\(Int(window.remainingPercent.rounded()))%")
                    .font(.system(size: 12, weight: .bold, design: .rounded))
                    .foregroundStyle(quotaTint(forRemainingPercent: window.remainingPercent))
            }

            ProgressView(value: max(window.remainingPercent / 100, 0.02), total: 1)
                .progressViewStyle(.linear)
                .tint(quotaTint(forRemainingPercent: window.remainingPercent))
                .scaleEffect(x: 1, y: 1.15, anchor: .center)
                .animation(.easeInOut(duration: 0.2), value: window.remainingPercent)

            if let resetDate = window.resetDate {
                Text("\(resetDate.formatted(.dateTime.month().day().hour().minute())) 重置")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    private func quotaPanel<Content: View>(
        title: String,
        headline: String,
        headlineTint: Color,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Text(title)
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(.secondary)

                Spacer()

                Text(headline)
                    .font(.system(size: 13, weight: .bold, design: .rounded))
                    .foregroundStyle(headlineTint)
            }

            content()
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(quotaPanelBackground)
        )
    }

    private func cardBackground(isSelected: Bool) -> some View {
        RoundedRectangle(cornerRadius: 24, style: .continuous)
            .fill(
                LinearGradient(
                    colors: isSelected
                        ? [
                            selectedCardTop,
                            selectedCardBottom
                        ]
                        : [
                            cardTop,
                            cardBottom
                        ],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
            )
    }

    private var background: some View {
        LinearGradient(
            colors: [
                backgroundTop,
                backgroundBottom
            ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
        .ignoresSafeArea()
    }

    private var quotaPanelBackground: Color {
        colorScheme == .dark
            ? Color.white.opacity(0.06)
            : Color.black.opacity(0.04)
    }

    private var serviceStatusColor: Color {
        switch serviceSupervisor.status {
        case .checking, .installing, .starting:
            return Color(red: 0.94, green: 0.59, blue: 0.18)
        case .runningLaunchAgent:
            return selectionAccent
        case .runningExternal:
            return apiAccent
        case .installedStopped, .notInstalled, .failed:
            return Color(red: 0.86, green: 0.24, blue: 0.24)
        }
    }

    private var errorPresented: Binding<Bool> {
        Binding(
            get: { viewModel.errorMessage != nil },
            set: { newValue in
                if !newValue {
                    viewModel.dismissError()
                }
            }
        )
    }

    private var backgroundTop: Color {
        colorScheme == .dark
            ? Color(red: 0.09, green: 0.11, blue: 0.14)
            : Color(red: 0.95, green: 0.97, blue: 0.99)
    }

    private var backgroundBottom: Color {
        colorScheme == .dark
            ? Color(red: 0.07, green: 0.09, blue: 0.11)
            : Color(red: 0.92, green: 0.95, blue: 0.93)
    }

    private var cardTop: Color {
        colorScheme == .dark
            ? Color(red: 0.15, green: 0.17, blue: 0.20)
            : Color.white.opacity(0.96)
    }

    private var cardBottom: Color {
        colorScheme == .dark
            ? Color(red: 0.11, green: 0.13, blue: 0.16)
            : Color(red: 0.95, green: 0.96, blue: 0.98)
    }

    private var selectedCardTop: Color {
        colorScheme == .dark
            ? Color(red: 0.11, green: 0.19, blue: 0.15)
            : Color.white.opacity(0.96)
    }

    private var selectedCardBottom: Color {
        colorScheme == .dark
            ? Color(red: 0.10, green: 0.24, blue: 0.18)
            : Color(red: 0.89, green: 0.97, blue: 0.92)
    }

    private var cardBorder: Color {
        colorScheme == .dark
            ? Color.white.opacity(0.08)
            : Color.white.opacity(0.6)
    }

    private var apiAccent: Color {
        Color(red: 0.22, green: 0.52, blue: 0.96)
    }

    private var accountAccent: Color {
        Color(red: 0.94, green: 0.59, blue: 0.18)
    }

    private var selectionAccent: Color {
        Color(red: 0.19, green: 0.74, blue: 0.46)
    }

    private var apiBadgeBackground: Color {
        colorScheme == .dark ? apiAccent.opacity(0.22) : apiAccent.opacity(0.14)
    }

    private var accountBadgeBackground: Color {
        colorScheme == .dark ? accountAccent.opacity(0.22) : accountAccent.opacity(0.16)
    }

    private var shadowColor: Color {
        .black
    }

    private func quotaPrimaryDetail(for window: ProviderQuotaWindow) -> String {
        var parts: [String] = [quotaWindowDescription(minutes: window.windowMinutes)]

        if let resetDate = window.resetDate {
            parts.append("\(resetDate.formatted(.dateTime.month().day().hour().minute())) 重置")
        }

        return parts.joined(separator: " · ")
    }

    private func quotaWindowDescription(minutes: Int?) -> String {
        guard let minutes else {
            return "当前窗口"
        }

        if minutes % (60 * 24) == 0 {
            return "\(minutes / (60 * 24)) 天窗口"
        }

        if minutes % 60 == 0 {
            return "\(minutes / 60) 小时窗口"
        }

        return "\(minutes) 分钟窗口"
    }

    private func quotaTint(forRemainingPercent remainingPercent: Double) -> Color {
        switch remainingPercent {
        case 60...:
            return selectionAccent
        case 30..<60:
            return Color(red: 0.94, green: 0.59, blue: 0.18)
        default:
            return Color(red: 0.86, green: 0.24, blue: 0.24)
        }
    }

    private func billingBadgeBackground(for provider: GatewayProvider) -> Color {
        switch provider.billingMode {
        case .metered:
            return colorScheme == .dark
                ? Color(red: 0.55, green: 0.31, blue: 0.08).opacity(0.34)
                : Color(red: 0.98, green: 0.72, blue: 0.33).opacity(0.28)
        case .subscription:
            return colorScheme == .dark
                ? apiAccent.opacity(0.18)
                : apiAccent.opacity(0.12)
        }
    }

    private func billingBadgeForeground(for provider: GatewayProvider) -> Color {
        switch provider.billingMode {
        case .metered:
            return colorScheme == .dark
                ? Color(red: 1.00, green: 0.82, blue: 0.48)
                : Color(red: 0.66, green: 0.38, blue: 0.05)
        case .subscription:
            return colorScheme == .dark
                ? Color(red: 0.61, green: 0.78, blue: 1.00)
                : apiAccent
        }
    }

    private func initialLoad() async {
        do {
            try await serviceSupervisor.ensureServiceRunning()
            await viewModel.refresh()
        } catch {
            viewModel.clearData()
            viewModel.errorMessage = error.localizedDescription
        }
    }

    private func refreshAll() async {
        await serviceSupervisor.refreshStatus()
        if serviceSupervisor.isReachable {
            await viewModel.refresh()
        } else {
            viewModel.clearData()
        }
    }

    private func startService() async {
        do {
            try await serviceSupervisor.startService()
            await viewModel.refresh()
        } catch {
            viewModel.clearData()
            viewModel.errorMessage = error.localizedDescription
        }
    }

    private func stopManagedService() async {
        await serviceSupervisor.stopService()
        if serviceSupervisor.isReachable {
            await viewModel.refresh()
        } else {
            viewModel.clearData()
        }
    }
}

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
    }
}

struct CodexConfigSheet: View {
    @ObservedObject var viewModel: GatewayViewModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("CodeX 配置")
                .font(.system(size: 26, weight: .bold, design: .rounded))

            Text("可以把 CodeX 的 `~/.codex/config.toml` 一键切换成内置的 AI Gateway 配置，也可以随时恢复到原来的版本。")
                .font(.system(size: 14))
                .foregroundStyle(.secondary)

            statusCard

            HStack {
                Spacer()

                Button("关闭") {
                    dismiss()
                }
                .buttonStyle(.bordered)

                Button {
                    Task {
                        _ = await viewModel.applyCodexConfig()
                    }
                } label: {
                    Label("应用内置配置", systemImage: "square.and.arrow.down")
                }
                .buttonStyle(.borderedProminent)

                Button {
                    Task {
                        await viewModel.restoreCodexConfig()
                    }
                } label: {
                    Label("恢复原配置", systemImage: "arrow.uturn.backward")
                }
                .buttonStyle(.bordered)
                .disabled(!(viewModel.codexConfigStatus?.restoreAvailable ?? false))
            }
        }
        .padding(24)
        .frame(minWidth: 560, minHeight: 320)
    }

    private var statusCard: some View {
        VStack(alignment: .leading, spacing: 14) {
            statusRow(title: "配置文件", value: viewModel.codexConfigStatus?.targetPath ?? "读取中…")
            statusRow(title: "认证文件", value: viewModel.codexConfigStatus?.authPath ?? "读取中…")
            statusRow(
                title: "当前状态",
                value: statusText
            )
            statusRow(title: "备份内容", value: backupText)
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .fill(Color.primary.opacity(0.05))
        )
    }

    private var statusText: String {
        guard let status = viewModel.codexConfigStatus else {
            return "读取中…"
        }

        if status.restoreAvailable {
            return "已应用 AI Gateway 内置配置，可恢复原来的配置和认证"
        }

        if status.targetExists || status.authExists {
            return "当前是本地原配置"
        }

        return "当前没有本地配置"
    }

    private var backupText: String {
        guard let status = viewModel.codexConfigStatus else {
            return "读取中…"
        }

        if !status.restoreAvailable {
            return "当前没有备份"
        }

        var items: [String] = []
        if status.configBackupExists {
            items.append("config.toml")
        }
        if status.authBackupExists {
            items.append("auth.json")
        }
        return items.joined(separator: "、")
    }

    private func statusRow(title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
            Text(value)
                .font(.system(size: 13, design: .monospaced))
                .textSelection(.enabled)
        }
    }
}

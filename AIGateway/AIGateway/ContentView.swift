//
//  ContentView.swift
//  AIGateway
//
//  Created by 刘子豪 on 2026/4/11.
//

import SwiftUI

struct ContentView: View {
    @Environment(\.colorScheme) private var colorScheme
    @StateObject private var viewModel = GatewayViewModel()
    @State private var showingAddProvider = false
    @State private var showingCodexConfigSheet = false
    private let gridColumns = [
        GridItem(.adaptive(minimum: 280, maximum: 360), spacing: 18)
    ]

    var body: some View {
        TabView {
            NavigationStack {
                VStack(spacing: 18) {
                    header
                    providerGrid
                    footer
                }
                .padding(24)
                .background(background)
                .navigationTitle("AI Gateway")
            }
            .tabItem {
                Label("供应商", systemImage: "square.grid.2x2")
            }

            LogsTabView(viewModel: viewModel)
                .tabItem {
                    Label("日志", systemImage: "text.document")
                }
        }
        .sheet(isPresented: $showingAddProvider) {
            AddProviderSheet(viewModel: viewModel)
        }
        .sheet(isPresented: $showingCodexConfigSheet) {
            CodexConfigSheet(viewModel: viewModel)
        }
        .task {
            await viewModel.refresh()
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
                        await viewModel.refresh()
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

    private var providerTable: some View {
        EmptyView()
    }

    private var providerGrid: some View {
        ScrollView {
            if viewModel.providers.isEmpty && !viewModel.isLoading {
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
            if viewModel.isLoading {
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
        let display = quotaDisplay(for: provider)

        return VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Text("剩余额度")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(.secondary)

                Spacer()

                Text(display.headline)
                    .font(.system(size: 13, weight: .bold, design: .rounded))
                    .foregroundStyle(display.tint)
            }

            ProgressView(value: display.fraction, total: 1)
                .progressViewStyle(.linear)
                .tint(display.tint)
                .scaleEffect(x: 1, y: 1.25, anchor: .center)
                .animation(.easeInOut(duration: 0.2), value: display.fraction)

            Text(display.detail)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(2)

            if let footnote = display.footnote {
                Text(footnote)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
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

    private func quotaDisplay(for provider: GatewayProvider) -> ProviderQuotaDisplay {
        if viewModel.isLoadingQuota(for: provider.id) {
            return ProviderQuotaDisplay(
                fraction: 1,
                headline: "同步中",
                detail: "正在读取当前供应商的额度快照",
                footnote: nil,
                tint: .secondary
            )
        }

        if let error = viewModel.quotaErrorMessage(for: provider.id) {
            return ProviderQuotaDisplay(
                fraction: 1,
                headline: "读取失败",
                detail: error,
                footnote: nil,
                tint: .red
            )
        }

        guard let summary = viewModel.quotaSummary(for: provider.id) else {
            return ProviderQuotaDisplay(
                fraction: 1,
                headline: "暂无数据",
                detail: "还没有拿到额度信息",
                footnote: nil,
                tint: .secondary
            )
        }

        if summary.status == .unsupported {
            return ProviderQuotaDisplay(
                fraction: 1,
                headline: "暂不支持",
                detail: summary.message ?? "当前供应商暂不支持额度快照",
                footnote: nil,
                tint: .secondary
            )
        }

        if let window = summary.primaryWindow {
            let remaining = Int(window.remainingPercent.rounded())
            var footnotes: [String] = []

            if let secondaryWindow = summary.secondaryWindow, secondaryWindow != window {
                footnotes.append("长窗口剩余 \(Int(secondaryWindow.remainingPercent.rounded()))%")
            }

            if summary.hasUnlimitedCredits {
                footnotes.append("账户余额无限")
            } else if let balance = summary.creditBalance {
                footnotes.append("余额 \(balance)")
            }

            return ProviderQuotaDisplay(
                fraction: max(window.remainingPercent / 100, 0.02),
                headline: "\(remaining)%",
                detail: quotaPrimaryDetail(for: window),
                footnote: footnotes.isEmpty ? nil : footnotes.joined(separator: " · "),
                tint: quotaTint(forRemainingPercent: window.remainingPercent)
            )
        }

        if summary.hasUnlimitedCredits {
            return ProviderQuotaDisplay(
                fraction: 1,
                headline: "无限",
                detail: "当前账户没有 credits 上限",
                footnote: nil,
                tint: selectionAccent
            )
        }

        if let balance = summary.creditBalance {
            return ProviderQuotaDisplay(
                fraction: 1,
                headline: balance,
                detail: "当前账户剩余余额",
                footnote: nil,
                tint: apiAccent
            )
        }

        return ProviderQuotaDisplay(
            fraction: 1,
            headline: "可用",
            detail: "额度接口已接通，但当前没有可视化窗口数据",
            footnote: nil,
            tint: selectionAccent
        )
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
}

private struct ProviderQuotaDisplay {
    let fraction: Double
    let headline: String
    let detail: String
    let footnote: String?
    let tint: Color
}

#Preview {
    ContentView()
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

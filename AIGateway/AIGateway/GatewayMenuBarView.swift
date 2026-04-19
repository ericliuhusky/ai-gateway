import AppKit
import SwiftUI

struct GatewayMenuBarView: View {
    @Environment(\.openWindow) private var openWindow
    @ObservedObject var viewModel: GatewayViewModel
    @ObservedObject var serviceSupervisor: GatewayServiceSupervisor
    @State private var isRefreshing = false
    @State private var isStarting = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header

            Divider()

            providerSection

            Divider()

            actions
        }
        .padding(16)
        .frame(width: 320, alignment: .topLeading)
        .task {
            await refreshIfReachable()
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Image(systemName: "bolt.horizontal.circle.fill")
                .font(.system(size: 22, weight: .semibold))
                .foregroundStyle(serviceTint)

            VStack(alignment: .leading, spacing: 2) {
                Text("AI Gateway")
                    .font(.system(size: 14, weight: .bold))

                Text(serviceSupervisor.statusTitle)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(serviceTint)
            }

            Spacer()

            if serviceSupervisor.isBusy || isRefreshing || isStarting {
                ProgressView()
                    .controlSize(.small)
            }
        }
    }

    @ViewBuilder
    private var providerSection: some View {
        if !serviceSupervisor.isReachable {
            statusMessage(
                title: "服务未连接",
                detail: "启动网关服务后会显示当前供应商和额度。"
            )
        } else if viewModel.isLoading && viewModel.providers.isEmpty {
            statusMessage(title: "正在同步", detail: "正在读取供应商信息。")
        } else if let provider = viewModel.selectedProvider {
            selectedProviderPanel(provider)
        } else {
            statusMessage(title: "未选择供应商", detail: "在主窗口选择供应商后，这里会显示它的额度。")
        }
    }

    private func selectedProviderPanel(_ provider: GatewayProvider) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 6) {
                Text("当前供应商")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.secondary)

                Text(provider.name)
                    .font(.system(size: 18, weight: .bold, design: .rounded))
                    .lineLimit(2)
                    .textSelection(.enabled)

                if let email = provider.accountEmail?.trimmingCharacters(in: .whitespacesAndNewlines),
                   !email.isEmpty
                {
                    Label(email, systemImage: "envelope.fill")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .textSelection(.enabled)
                }
            }

            if provider.supportsQuotaDisplay {
                quotaSection(for: provider)
            }
        }
    }

    @ViewBuilder
    private func quotaSection(for provider: GatewayProvider) -> some View {
        if let summary = viewModel.quotaSummary(for: provider.id) {
            quotaWindow(summary: summary)
        } else if viewModel.isLoadingQuota(for: provider.id) {
            statusMessage(title: "额度窗口", detail: "正在读取当前供应商额度。")
        } else if let error = viewModel.quotaErrorMessage(for: provider.id) {
            statusMessage(title: "额度窗口", detail: error)
        } else {
            statusMessage(title: "额度窗口", detail: "还没有拿到额度信息。")
        }
    }

    @ViewBuilder
    private func quotaWindow(summary: ProviderQuotaSummary) -> some View {
        if summary.status == .unsupported {
            statusMessage(title: "额度窗口", detail: summary.message ?? "当前供应商暂不支持额度快照。")
        } else {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline) {
                    Text("额度窗口")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(.secondary)

                    Spacer()

                    Text(quotaHeadline(summary: summary))
                        .font(.system(size: 13, weight: .bold, design: .rounded))
                        .foregroundStyle(quotaHeadlineTint(summary: summary))
                }

                if let primary = summary.snapshot?.primary {
                    quotaWindowRow(title: quotaWindowTitle(minutes: primary.windowMinutes, fallback: "五小时窗口"), window: primary)
                }

                if let secondary = summary.snapshot?.secondary {
                    quotaWindowRow(title: quotaWindowTitle(minutes: secondary.windowMinutes, fallback: "周窗口"), window: secondary)
                }

                if summary.snapshot?.primary == nil && summary.snapshot?.secondary == nil {
                    Text("额度接口已接通，但当前没有可视化窗口数据。")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(.secondary)
                }

                if let footnote = quotaCreditsFootnoteText(summary: summary) {
                    Text(footnote)
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
            .padding(12)
            .background(.quaternary.opacity(0.65))
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
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

            if let resetDate = window.resetDate {
                Text(quotaResetText(for: resetDate, windowMinutes: window.windowMinutes))
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    private func statusMessage(title: String, detail: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.system(size: 13, weight: .bold))

            Text(detail)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var actions: some View {
        HStack(spacing: 10) {
            Button {
                openWindow(id: "main")
                NSApp.activate(ignoringOtherApps: true)
            } label: {
                Label("主窗口", systemImage: "macwindow")
            }

            Spacer()

            if serviceSupervisor.canStart {
                Button {
                    Task {
                        await startFromMenu()
                    }
                } label: {
                    Label("启动", systemImage: "play.fill")
                }
                .disabled(isStarting)
            }
        }
        .controlSize(.small)
    }

    private var serviceTint: Color {
        switch serviceSupervisor.status {
        case .checking, .installing, .starting:
            return Color(red: 0.94, green: 0.59, blue: 0.18)
        case .runningLaunchAgent:
            return Color(red: 0.19, green: 0.74, blue: 0.46)
        case .runningExternal:
            return Color(red: 0.22, green: 0.52, blue: 0.96)
        case .installedStopped, .notInstalled, .failed:
            return Color(red: 0.86, green: 0.24, blue: 0.24)
        }
    }

    private func refreshIfReachable() async {
        await serviceSupervisor.refreshStatus()
        guard serviceSupervisor.isReachable else { return }

        if viewModel.providers.isEmpty {
            await refreshFromMenu()
            return
        }

        if let providerID = viewModel.selectedProviderID,
           viewModel.quotaSummary(for: providerID) == nil,
           viewModel.quotaErrorMessage(for: providerID) == nil
        {
            await refreshSelectedQuota(providerID: providerID)
        }
    }

    private func refreshFromMenu() async {
        guard !isRefreshing else { return }

        isRefreshing = true
        defer { isRefreshing = false }

        await viewModel.refresh()
    }

    private func refreshSelectedQuota(providerID: String) async {
        guard !isRefreshing else { return }

        isRefreshing = true
        defer { isRefreshing = false }

        await viewModel.refreshQuota(for: providerID)
    }

    private func startFromMenu() async {
        guard !isStarting else { return }

        isStarting = true
        defer { isStarting = false }

        do {
            try await serviceSupervisor.startService()
            await viewModel.refresh()
        } catch {
            viewModel.clearData()
            viewModel.errorMessage = error.localizedDescription
        }
    }

    private func quotaHeadline(summary: ProviderQuotaSummary) -> String {
        var parts: [String] = []
        if let primary = summary.snapshot?.primary {
            parts.append("5h \(Int(primary.remainingPercent.rounded()))%")
        }
        if let secondary = summary.snapshot?.secondary {
            parts.append("周 \(Int(secondary.remainingPercent.rounded()))%")
        }
        return parts.isEmpty ? "可用" : parts.joined(separator: " · ")
    }

    private func quotaHeadlineTint(summary: ProviderQuotaSummary) -> Color {
        let remainingPercent = [summary.snapshot?.primary?.remainingPercent, summary.snapshot?.secondary?.remainingPercent]
            .compactMap { $0 }
            .min() ?? 100
        return quotaTint(forRemainingPercent: remainingPercent)
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

    private func quotaResetText(for date: Date, windowMinutes: Int?) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "zh_CN")
        formatter.dateFormat = windowMinutes == 300 ? "HH:mm" : "M月d日 EEE HH:mm"
        return "\(formatter.string(from: date)) 重置"
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
            return Color(red: 0.19, green: 0.74, blue: 0.46)
        case 30..<60:
            return Color(red: 0.94, green: 0.59, blue: 0.18)
        default:
            return Color(red: 0.86, green: 0.24, blue: 0.24)
        }
    }
}

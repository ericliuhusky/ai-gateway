import SwiftUI

struct AddProviderSheet: View {
    @ObservedObject var viewModel: GatewayViewModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme

    private let mode: ProviderCreationMode
    @State private var loginProvider: AccountLoginProvider = .google
    @State private var name = ""
    @State private var baseURL = ""
    @State private var apiKey = ""
    @State private var billingMode: GatewayBillingMode = .metered
    @State private var usesChatCompletions = false

    init(viewModel: GatewayViewModel, initialMode: ProviderCreationMode = .apiKey) {
        self.viewModel = viewModel
        self.mode = initialMode
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            header

            contentCard

            actions
        }
        .padding(24)
        .frame(width: 540)
        .background(sheetBackground)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(mode == .apiKey ? "API Key" : "账户登录")
                .font(.system(size: 22, weight: .bold, design: .rounded))

            Text(mode == .apiKey ? "接入 OpenAI 兼容接口。" : "登录后会自动生成供应商。")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
        }
    }

    private var contentCard: some View {
        VStack(alignment: .leading, spacing: 18) {
            switch mode {
            case .apiKey:
                apiForm
            case .account:
                accountForm
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .fill(cardBackground)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .strokeBorder(cardBorder, lineWidth: 1)
        )
    }

    private var apiForm: some View {
        VStack(alignment: .leading, spacing: 14) {
            field("名称", text: $name, prompt: "openai-proxy")
            field("Base URL", text: $baseURL, prompt: "https://api.example.com/v1")

            secureField("API Key", text: $apiKey, prompt: "sk-...")

            HStack(alignment: .top, spacing: 14) {
                choiceGroup("计费", selection: $billingMode) {
                    ForEach(GatewayBillingMode.allCases) { mode in
                        Text(mode.title).tag(mode)
                    }
                }

                Toggle("Chat Completions", isOn: $usesChatCompletions)
                    .toggleStyle(.checkbox)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .padding(.top, 23)
                    .help("默认走 Responses 接口；旧供应商需要时再开启。")
            }
        }
    }

    private var accountForm: some View {
        VStack(alignment: .leading, spacing: 16) {
            choiceGroup("账号", selection: $loginProvider) {
                ForEach(AccountLoginProvider.allCases) { provider in
                    Text(provider.title).tag(provider)
                }
            }

            if loginProvider == .openai {
                HStack(spacing: 10) {
                    Image(systemName: "terminal")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(openAIAccent)

                    Text("已登录 Codex 时，可直接导入本机账号。")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)

                    Spacer()

                    Button {
                        Task {
                            _ = await viewModel.importOpenAIFromLocalCodexAuth()
                        }
                    } label: {
                        Text("导入")
                            .frame(minWidth: 44)
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                    .disabled(viewModel.isLoading)
                }
                .padding(12)
                .background(
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .fill(openAIAccent.opacity(colorScheme == .dark ? 0.16 : 0.08))
                )
            }
        }
    }

    private var actions: some View {
        HStack(spacing: 10) {
            Button("取消") {
                dismiss()
            }
            .keyboardShortcut(.cancelAction)

            Spacer()

            if mode == .account {
                Button {
                    Task {
                        await viewModel.refresh()
                    }
                } label: {
                    Label("已完成", systemImage: "arrow.clockwise")
                }
                .buttonStyle(.bordered)
                .disabled(viewModel.isLoading)

                Button {
                    viewModel.openLogin(provider: loginProvider)
                } label: {
                    Label("打开登录", systemImage: "link")
                        .frame(minWidth: 94)
                }
                .buttonStyle(.borderedProminent)
            } else {
                Button {
                    Task {
                        let didCreate = await viewModel.createProvider(
                            name: trimmedName,
                            baseURL: trimmedBaseURL,
                            apiKey: trimmedAPIKey,
                            billingMode: billingMode,
                            usesChatCompletions: usesChatCompletions
                        )
                        if didCreate {
                            dismiss()
                        }
                    }
                } label: {
                    Text(viewModel.isLoading ? "创建中" : "创建")
                        .frame(minWidth: 94)
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(!canCreateAPIProvider || viewModel.isLoading)
            }
        }
    }

    private func field(_ title: String, text: Binding<String>, prompt: String) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            label(title)

            TextField(prompt, text: text)
                .textFieldStyle(.plain)
                .font(.system(size: 13, weight: .medium))
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(fieldBackground)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        }
    }

    private func secureField(_ title: String, text: Binding<String>, prompt: String) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            label(title)

            SecureField(prompt, text: text)
                .textFieldStyle(.plain)
                .font(.system(size: 13, weight: .medium))
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(fieldBackground)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        }
    }

    private func choiceGroup<Selection: Hashable, Content: View>(
        _ title: String,
        selection: Binding<Selection>,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            label(title)

            Picker(title, selection: selection) {
                content()
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(width: 190)
        }
    }

    private func label(_ title: String) -> some View {
        Text(title)
            .font(.system(size: 11, weight: .semibold, design: .rounded))
            .foregroundStyle(.secondary)
    }

    private var trimmedName: String {
        name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var trimmedBaseURL: String {
        baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var trimmedAPIKey: String {
        apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var canCreateAPIProvider: Bool {
        !trimmedName.isEmpty && !trimmedBaseURL.isEmpty && !trimmedAPIKey.isEmpty
    }

    private var sheetBackground: some View {
        LinearGradient(
            colors: colorScheme == .dark
                ? [
                    Color(red: 0.09, green: 0.11, blue: 0.14),
                    Color(red: 0.07, green: 0.09, blue: 0.11)
                ]
                : [
                    Color(red: 0.95, green: 0.97, blue: 0.99),
                    Color(red: 0.92, green: 0.95, blue: 0.93)
                ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
    }

    private var cardBackground: Color {
        colorScheme == .dark
            ? Color.white.opacity(0.06)
            : Color.white.opacity(0.76)
    }

    private var fieldBackground: Color {
        colorScheme == .dark
            ? Color.white.opacity(0.08)
            : Color.white.opacity(0.82)
    }

    private var cardBorder: Color {
        colorScheme == .dark
            ? Color.white.opacity(0.08)
            : Color.white.opacity(0.70)
    }

    private var openAIAccent: Color {
        Color(red: 0.19, green: 0.74, blue: 0.46)
    }
}

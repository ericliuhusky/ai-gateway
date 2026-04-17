import SwiftUI

struct AddProviderSheet: View {
    @ObservedObject var viewModel: GatewayViewModel
    @Environment(\.dismiss) private var dismiss

    @State private var mode: ProviderCreationMode = .apiKey
    @State private var loginProvider: AccountLoginProvider = .google
    @State private var name = ""
    @State private var baseURL = ""
    @State private var apiKey = ""
    @State private var billingMode: GatewayBillingMode = .metered
    @State private var usesChatCompletions = false

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("添加供应商")
                .font(.system(size: 26, weight: .bold, design: .rounded))

            Picker("模式", selection: $mode) {
                Text("API").tag(ProviderCreationMode.apiKey)
                Text("账户").tag(ProviderCreationMode.account)
            }
            .pickerStyle(.segmented)

            Group {
                switch mode {
                case .apiKey:
                    apiForm
                case .account:
                    accountForm
                }
            }

            Spacer()
        }
        .padding(24)
        .frame(minWidth: 520, minHeight: 420)
    }

    private var apiForm: some View {
        VStack(alignment: .leading, spacing: 16) {
            labeledField("供应商名字", text: $name, prompt: "openai-compatible")
            labeledField("Base URL", text: $baseURL, prompt: "https://api.example.com/v1")

            VStack(alignment: .leading, spacing: 8) {
                Text("API Key")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                SecureField("sk-...", text: $apiKey)
                    .textFieldStyle(.roundedBorder)
            }

            VStack(alignment: .leading, spacing: 8) {
                Text("付费模式")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                Picker("付费模式", selection: $billingMode) {
                    ForEach(GatewayBillingMode.allCases) { item in
                        Text(item.title).tag(item)
                    }
                }
                .pickerStyle(.segmented)
            }

            Toggle("兼容 Chat Completions 接口", isOn: $usesChatCompletions)
                .toggleStyle(.checkbox)

            HStack {
                Spacer()

                Button("取消") {
                    dismiss()
                }
                .buttonStyle(.bordered)

                Button {
                    Task {
                        let didCreate = await viewModel.createProvider(
                            name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                            baseURL: baseURL.trimmingCharacters(in: .whitespacesAndNewlines),
                            apiKey: apiKey.trimmingCharacters(in: .whitespacesAndNewlines),
                            billingMode: billingMode,
                            usesChatCompletions: usesChatCompletions
                        )
                        if didCreate {
                            dismiss()
                        }
                    }
                } label: {
                    Text("创建供应商")
                        .frame(minWidth: 120)
                }
                .buttonStyle(.borderedProminent)
                .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    || baseURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    || apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }

    private var accountForm: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("账户型供应商不通过表单直接创建，而是通过登录自动生成。登录成功后，网关会自动创建或更新对应供应商。")
                .font(.system(size: 14))
                .foregroundStyle(.secondary)

            Picker("登录供应商", selection: $loginProvider) {
                ForEach(AccountLoginProvider.allCases) { provider in
                    Text(provider.title).tag(provider)
                }
            }
            .pickerStyle(.segmented)

            HStack {
                Spacer()

                Button("关闭") {
                    dismiss()
                }
                .buttonStyle(.bordered)

                Button {
                    viewModel.openLogin(provider: loginProvider)
                } label: {
                    Label("打开登录链接", systemImage: "link")
                        .frame(minWidth: 120)
                }
                .buttonStyle(.borderedProminent)

                Button {
                    Task {
                        await viewModel.refresh()
                    }
                } label: {
                    Label("完成登录", systemImage: "arrow.clockwise")
                }
                .buttonStyle(.bordered)
            }
        }
    }

    private func labeledField(_ title: String, text: Binding<String>, prompt: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
            TextField(prompt, text: text)
                .textFieldStyle(.roundedBorder)
        }
    }
}

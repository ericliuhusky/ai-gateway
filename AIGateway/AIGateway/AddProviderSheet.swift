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

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("Add Provider")
                .font(.system(size: 26, weight: .bold, design: .rounded))

            Picker("Mode", selection: $mode) {
                Text("API").tag(ProviderCreationMode.apiKey)
                Text("Account").tag(ProviderCreationMode.account)
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
            labeledField("Provider Name", text: $name, prompt: "bytedance")
            labeledField("Base URL", text: $baseURL, prompt: "https://api.example.com/v1")

            VStack(alignment: .leading, spacing: 8) {
                Text("API Key")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                SecureField("sk-...", text: $apiKey)
                    .textFieldStyle(.roundedBorder)
            }

            VStack(alignment: .leading, spacing: 8) {
                Text("Billing Mode")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                Picker("Billing Mode", selection: $billingMode) {
                    ForEach(GatewayBillingMode.allCases) { item in
                        Text(item.title).tag(item)
                    }
                }
                .pickerStyle(.segmented)
            }

            HStack {
                Spacer()

                Button("Cancel") {
                    dismiss()
                }
                .buttonStyle(.bordered)

                Button {
                    Task {
                        let didCreate = await viewModel.createProvider(
                            name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                            baseURL: baseURL.trimmingCharacters(in: .whitespacesAndNewlines),
                            apiKey: apiKey.trimmingCharacters(in: .whitespacesAndNewlines),
                            billingMode: billingMode
                        )
                        if didCreate {
                            dismiss()
                        }
                    }
                } label: {
                    Text("Create Provider")
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
            Text("账号型 provider 不通过表单直接创建，而是通过登录自动生成。登录成功后，网关会自动创建或更新对应 provider。")
                .font(.system(size: 14))
                .foregroundStyle(.secondary)

            Picker("Login Provider", selection: $loginProvider) {
                ForEach(AccountLoginProvider.allCases) { provider in
                    Text(provider.title).tag(provider)
                }
            }
            .pickerStyle(.segmented)

            VStack(alignment: .leading, spacing: 10) {
                Text("对应接口")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                Text(loginProvider == .google ? "/auth/google/start" : "/auth/openai/start")
                    .font(.system(size: 13, design: .monospaced))
                    .textSelection(.enabled)
            }

            HStack {
                Spacer()

                Button("Close") {
                    dismiss()
                }
                .buttonStyle(.bordered)

                Button {
                    viewModel.openLogin(provider: loginProvider)
                } label: {
                    Label("Open Login", systemImage: "link")
                        .frame(minWidth: 120)
                }
                .buttonStyle(.borderedProminent)

                Button {
                    Task {
                        await viewModel.refresh()
                    }
                } label: {
                    Label("I Finished Login", systemImage: "arrow.clockwise")
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

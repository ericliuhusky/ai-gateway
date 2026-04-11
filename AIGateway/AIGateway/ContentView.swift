//
//  ContentView.swift
//  AIGateway
//
//  Created by 刘子豪 on 2026/4/11.
//

import SwiftUI

struct ContentView: View {
    @StateObject private var viewModel = GatewayViewModel()
    @State private var showingAddProvider = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 18) {
                header
                providerTable
                footer
            }
            .padding(24)
            .background(background)
            .navigationTitle("AI Gateway")
            .sheet(isPresented: $showingAddProvider) {
                AddProviderSheet(viewModel: viewModel)
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
        }
        .frame(minWidth: 980, minHeight: 680)
    }

    private var header: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 10) {
                Text("Providers")
                    .font(.system(size: 32, weight: .bold, design: .rounded))
                Text("管理 API provider、发起 Google / OpenAI 账号登录，并切换当前选中的 provider。")
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(.secondary)

                if let selected = viewModel.selectedProviderName {
                    Label("Current: \(selected)", systemImage: "checkmark.circle.fill")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(.green)
                } else {
                    Label("No provider selected", systemImage: "circle.dashed")
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
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
                .buttonStyle(.bordered)

                Button {
                    showingAddProvider = true
                } label: {
                    Label("Add Provider", systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)
            }
        }
    }

    private var providerTable: some View {
        Table(viewModel.providers, selection: $viewModel.selectedTableID) {
            TableColumn("Selected") { provider in
                HStack {
                    if provider.id == viewModel.selectedProviderID {
                        Label("Active", systemImage: "checkmark.circle.fill")
                            .labelStyle(.titleAndIcon)
                            .foregroundStyle(.green)
                    } else {
                        Text(" ")
                    }
                }
            }
            .width(min: 92, ideal: 96, max: 110)

            TableColumn("Name") { provider in
                VStack(alignment: .leading, spacing: 4) {
                    Text(provider.name)
                        .font(.system(size: 13, weight: .semibold))
                    Text(provider.id)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
            }
            .width(min: 210, ideal: 260)

            TableColumn("Auth") { provider in
                authBadge(for: provider)
            }
            .width(min: 120, ideal: 130)

            TableColumn("Base URL") { provider in
                Text(provider.baseURL.isEmpty ? "OAuth managed" : provider.baseURL)
                    .font(.system(size: 12))
                    .foregroundStyle(provider.baseURL.isEmpty ? .secondary : .primary)
                    .textSelection(.enabled)
                    .lineLimit(2)
            }
            .width(min: 220, ideal: 320)

            TableColumn("Billing") { provider in
                Text(provider.billingModeLabel)
                    .font(.system(size: 12, weight: .medium))
            }
            .width(min: 100, ideal: 120)

            TableColumn("Credential") { provider in
                Text(provider.authMode == .account ? (provider.accountID ?? "Waiting for account") : provider.apiKeyPreview)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .textSelection(.enabled)
            }
            .width(min: 180, ideal: 250)

            TableColumn("Action") { provider in
                Button {
                    Task {
                        await viewModel.selectProvider(id: provider.id)
                    }
                } label: {
                    Text(provider.id == viewModel.selectedProviderID ? "Selected" : "Use")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .disabled(provider.id == viewModel.selectedProviderID || viewModel.isLoading)
            }
            .width(min: 90, ideal: 110)
        }
        .tableStyle(.inset(alternatesRowBackgrounds: true))
        .overlay {
            if viewModel.providers.isEmpty && !viewModel.isLoading {
                ContentUnavailableView(
                    "No Providers Yet",
                    systemImage: "tray",
                    description: Text("添加一个 API provider，或者用账号登录自动生成 provider。")
                )
            }
        }
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
        Text(provider.authMode == .apiKey ? "API Key" : "Account")
            .font(.system(size: 11, weight: .bold))
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(provider.authMode == .apiKey ? Color.blue.opacity(0.14) : Color.orange.opacity(0.16))
            .foregroundStyle(provider.authMode == .apiKey ? .blue : .orange)
            .clipShape(Capsule())
    }

    private var background: some View {
        LinearGradient(
            colors: [
                Color(red: 0.95, green: 0.97, blue: 0.99),
                Color(red: 0.92, green: 0.95, blue: 0.93)
            ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
        .ignoresSafeArea()
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
}

#Preview {
    ContentView()
}

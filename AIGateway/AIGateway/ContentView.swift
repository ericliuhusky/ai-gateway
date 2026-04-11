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
    private let gridColumns = [
        GridItem(.adaptive(minimum: 280, maximum: 360), spacing: 18)
    ]

    var body: some View {
        NavigationStack {
            VStack(spacing: 18) {
                header
                providerGrid
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
                        .foregroundStyle(selectionAccent)
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
        EmptyView()
    }

    private var providerGrid: some View {
        ScrollView {
            if viewModel.providers.isEmpty && !viewModel.isLoading {
                ContentUnavailableView(
                    "No Providers Yet",
                    systemImage: "tray",
                    description: Text("添加一个 API provider，或者用账号登录自动生成 provider。")
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
        Text(provider.authMode == .apiKey ? "API Key" : "Account")
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
                            .background(chipBackground)
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

            VStack(alignment: .leading, spacing: 10) {
                providerMetaRow(
                    title: "Base URL",
                    value: provider.baseURL.isEmpty ? "OAuth managed by gateway" : provider.baseURL,
                    emphasized: false
                )

                providerMetaRow(
                    title: provider.authMode == .account ? "Account" : "Credential",
                    value: provider.authMode == .account
                        ? (provider.accountID ?? "Waiting for login")
                        : provider.apiKeyPreview,
                    emphasized: provider.authMode == .account
                )
            }

            HStack {
                if isSelected {
                    Text("Selected")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(selectionAccent)
                } else {
                    Text("Ready to use")
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(.secondary)
                }

                Spacer()

                Button {
                    Task {
                        await viewModel.selectProvider(id: provider.id)
                    }
                } label: {
                    Text(isSelected ? "Current" : "Select")
                        .frame(minWidth: 88)
                }
                .buttonStyle(.borderedProminent)
                .tint(isSelected ? selectionAccent : apiAccent)
                .disabled(isSelected || viewModel.isLoading)
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, minHeight: 220, alignment: .topLeading)
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

    private var chipBackground: Color {
        colorScheme == .dark
            ? Color.white.opacity(0.08)
            : Color.primary.opacity(0.07)
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
}

#Preview {
    ContentView()
}

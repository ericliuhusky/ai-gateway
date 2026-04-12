import AppKit
import Combine
import Foundation

@MainActor
final class GatewayViewModel: ObservableObject {
    @Published var providers: [GatewayProvider] = []
    @Published var providerQuotas: [String: ProviderQuotaSummary] = [:]
    @Published var quotaErrors: [String: String] = [:]
    @Published var quotaLoadingProviderIDs: Set<String> = []
    @Published var selectedProviderID: String?
    @Published var codexConfigStatus: CodexConfigStatus?
    @Published var logs: [GatewayLogSummary] = []
    @Published var selectedLogRequestID: String?
    @Published var selectedLogDetail: GatewayLogDetail?
    @Published var isLogsLoading = false
    @Published var isLogDetailLoading = false
    @Published var isLoggingEnabled = true
    @Published var isLoading = false
    @Published var errorMessage: String?

    let baseURL: URL
    private let client: GatewayAPIClient

    init(baseURL: URL = URL(string: "http://127.0.0.1:10100")!) {
        self.baseURL = baseURL
        self.client = GatewayAPIClient(baseURL: baseURL)
    }

    var selectedProviderName: String? {
        providers.first(where: { $0.id == selectedProviderID })?.name
    }

    func quotaSummary(for providerID: String) -> ProviderQuotaSummary? {
        providerQuotas[providerID]
    }

    func quotaErrorMessage(for providerID: String) -> String? {
        quotaErrors[providerID]
    }

    func isLoadingQuota(for providerID: String) -> Bool {
        quotaLoadingProviderIDs.contains(providerID)
    }

    func refresh() async {
        isLoading = true
        defer { isLoading = false }

        do {
            async let providersTask = client.fetchProviders()
            async let selectedTask = client.fetchSelectedProvider()
            async let codexConfigTask = client.fetchCodexConfigStatus()

            let (providers, selected, codexConfig) = try await (
                providersTask,
                selectedTask,
                codexConfigTask
            )
            let sortedProviders = providers.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            self.providers = sortedProviders
            self.selectedProviderID = selected.providerID
            self.codexConfigStatus = codexConfig
            await refreshProviderQuotas(for: sortedProviders)
            await refreshLogs()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func refreshLogs() async {
        isLogsLoading = true
        defer { isLogsLoading = false }

        do {
            async let logsTask = client.fetchLogs()
            async let settingsTask = client.fetchLoggingSettings()
            let (logs, settings) = try await (logsTask, settingsTask)
            self.logs = logs
            self.isLoggingEnabled = settings.enabled
            await syncSelectedLogDetail(with: logs, preserveSelection: true)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func createProvider(
        name: String,
        baseURL: String,
        apiKey: String,
        billingMode: GatewayBillingMode
    ) async -> Bool {
        isLoading = true
        defer { isLoading = false }

        do {
            try await client.createProvider(
                CreateAPIProviderRequest(
                    name: name,
                    baseURL: baseURL,
                    apiKey: apiKey,
                    billingMode: billingMode
                )
            )
            await refresh()
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }

    func selectProvider(id: String) async {
        isLoading = true
        defer { isLoading = false }

        do {
            try await client.selectProvider(id: id)
            selectedProviderID = id
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func openLogin(provider: AccountLoginProvider) {
        NSWorkspace.shared.open(client.loginURL(for: provider))
    }

    func applyCodexConfig() async -> Bool {
        isLoading = true
        defer { isLoading = false }

        do {
            codexConfigStatus = try await client.applyCodexConfig()
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }

    func restoreCodexConfig() async {
        isLoading = true
        defer { isLoading = false }

        do {
            codexConfigStatus = try await client.restoreCodexConfig()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func dismissError() {
        errorMessage = nil
    }

    func selectLog(requestID: String) async {
        selectedLogRequestID = requestID
        await refreshSelectedLogDetail()
    }

    func refreshSelectedLogDetail() async {
        guard let requestID = selectedLogRequestID else {
            selectedLogDetail = nil
            return
        }

        isLogDetailLoading = true
        defer { isLogDetailLoading = false }

        do {
            selectedLogDetail = try await client.fetchLogDetail(requestID: requestID)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func setLoggingEnabled(_ enabled: Bool) async {
        let previous = isLoggingEnabled
        isLoggingEnabled = enabled

        do {
            let settings = try await client.setLoggingEnabled(enabled)
            isLoggingEnabled = settings.enabled
        } catch {
            isLoggingEnabled = previous
            errorMessage = error.localizedDescription
        }
    }

    func clearLogs() async {
        isLogsLoading = true
        defer { isLogsLoading = false }

        do {
            try await client.clearLogs()
            logs = []
            selectedLogRequestID = nil
            selectedLogDetail = nil
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func refreshProviderQuotas(for providers: [GatewayProvider]) async {
        let providerIDs = Set(providers.map(\.id))
        providerQuotas = providerQuotas.filter { providerIDs.contains($0.key) }
        quotaErrors = [:]
        quotaLoadingProviderIDs = providerIDs

        guard !providers.isEmpty else {
            quotaLoadingProviderIDs = []
            return
        }

        let client = self.client
        var nextQuotas: [String: ProviderQuotaSummary] = [:]
        var nextErrors: [String: String] = [:]

        await withTaskGroup(of: (String, Result<ProviderQuotaSummary, Error>).self) { group in
            for provider in providers {
                let providerID = provider.id
                group.addTask {
                    do {
                        return (providerID, .success(try await client.fetchProviderQuota(providerID: providerID)))
                    } catch {
                        return (providerID, .failure(error))
                    }
                }
            }

            for await (providerID, result) in group {
                switch result {
                case .success(let quota):
                    nextQuotas[providerID] = quota
                case .failure(let error):
                    nextErrors[providerID] = error.localizedDescription
                }
            }
        }

        providerQuotas = nextQuotas
        quotaErrors = nextErrors
        quotaLoadingProviderIDs = []
    }

    private func syncSelectedLogDetail(
        with logs: [GatewayLogSummary],
        preserveSelection: Bool
    ) async {
        let nextSelection: String?
        if preserveSelection, let selectedLogRequestID,
           logs.contains(where: { $0.requestID == selectedLogRequestID }) {
            nextSelection = selectedLogRequestID
        } else {
            nextSelection = logs.first?.requestID
        }

        selectedLogRequestID = nextSelection

        guard nextSelection != nil else {
            selectedLogDetail = nil
            return
        }

        await refreshSelectedLogDetail()
    }
}

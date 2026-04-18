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
    @Published var selectedModelID: String?
    @Published var availableModels: [GatewayModel] = []
    @Published var isLoadingModels = false
    @Published var modelErrorMessage: String?
    @Published var codexConfigStatus: CodexConfigStatus?
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
            self.selectedModelID = selected.selectedModel
            self.codexConfigStatus = codexConfig
            self.errorMessage = nil
            await refreshModels()
            await refreshProviderQuotas(for: sortedProviders)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func createProvider(
        name: String,
        baseURL: String,
        apiKey: String,
        billingMode: GatewayBillingMode,
        usesChatCompletions: Bool
    ) async -> Bool {
        isLoading = true
        defer { isLoading = false }

        do {
            try await client.createProvider(
                CreateAPIProviderRequest(
                    name: name,
                    baseURL: baseURL,
                    apiKey: apiKey,
                    billingMode: billingMode,
                    usesChatCompletions: usesChatCompletions
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
            selectedModelID = nil
            availableModels = []
            await refreshModels()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func refreshModels(forceRefresh: Bool = false) async {
        guard selectedProviderID != nil else {
            availableModels = []
            selectedModelID = nil
            modelErrorMessage = nil
            return
        }

        isLoadingModels = true
        defer { isLoadingModels = false }

        do {
            let models = try await client.fetchModels(forceRefresh: forceRefresh)
            availableModels = models.sorted { $0.id.localizedStandardCompare($1.id) == .orderedAscending }
            modelErrorMessage = nil
        } catch {
            availableModels = []
            modelErrorMessage = error.localizedDescription
        }
    }

    func selectModel(id: String) async {
        isLoadingModels = true
        defer { isLoadingModels = false }

        do {
            let selected = try await client.selectModel(id: id)
            selectedModelID = selected.selectedModel
            selectedProviderID = selected.providerID
            modelErrorMessage = nil
        } catch {
            modelErrorMessage = error.localizedDescription
        }
    }

    func clearSelectedModel() async {
        isLoadingModels = true
        defer { isLoadingModels = false }

        do {
            let selected = try await client.clearSelectedModel()
            selectedModelID = selected.selectedModel
            selectedProviderID = selected.providerID
            modelErrorMessage = nil
        } catch {
            modelErrorMessage = error.localizedDescription
        }
    }

    func openLogin(provider: AccountLoginProvider) {
        NSWorkspace.shared.open(client.loginURL(for: provider))
    }

    func importOpenAIFromLocalCodexAuth() async -> Bool {
        isLoading = true
        defer { isLoading = false }

        do {
            _ = try await client.importOpenAIFromLocalCodexAuth()
            await refresh()
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
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

    func clearData() {
        providers = []
        providerQuotas = [:]
        quotaErrors = [:]
        quotaLoadingProviderIDs = []
        selectedProviderID = nil
        selectedModelID = nil
        availableModels = []
        isLoadingModels = false
        modelErrorMessage = nil
        codexConfigStatus = nil
    }

    func openDebugDashboard() {
        NSWorkspace.shared.open(baseURL.appending(path: "debug"))
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
}

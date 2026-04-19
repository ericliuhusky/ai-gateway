import AppKit
import Combine
import Foundation

@MainActor
final class GatewayViewModel: ObservableObject {
    private enum QuotaRefreshPolicy {
        static let selectedInterval: TimeInterval = 60
        static let unselectedInterval: TimeInterval = 300
        static let lowRemainingSelectedInterval: TimeInterval = 30
        static let lowRemainingUnselectedInterval: TimeInterval = 150
        static let lowRemainingThreshold: Double = 20
    }

    @Published var providers: [GatewayProvider] = []
    @Published var providerQuotas: [String: ProviderQuotaSummary] = [:]
    @Published var quotaErrors: [String: String] = [:]
    @Published var quotaLoadingProviderIDs: Set<String> = []
    @Published var selectedProviderID: String?
    @Published var selectedModelID: String?
    @Published var availableModels: [GatewayModel] = []
    @Published var isLoadingModels = false
    @Published var showsModelRefreshActivity = false
    @Published var modelErrorMessage: String?
    @Published var isLoading = false
    @Published var errorMessage: String?

    let baseURL: URL
    private let client: GatewayAPIClient
    private var modelActivityTask: Task<Void, Never>?
    private var quotaLastRefreshAt: [String: Date] = [:]
    private var refreshingQuotaProviderIDs: Set<String> = []

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

            let (providers, selected) = try await (
                providersTask,
                selectedTask
            )
            let sortedProviders = providers.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            self.providers = sortedProviders
            self.selectedProviderID = selected.providerID
            self.selectedModelID = selected.selectedModel
            self.errorMessage = nil
            trimQuotaState(to: Set(sortedProviders.map(\.id)))
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
            await refreshModels()
            await refreshQuota(for: id, showLoadingState: false)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func refreshModels(forceRefresh: Bool = false, userInitiated: Bool = false) async {
        guard selectedProviderID != nil else {
            modelActivityTask?.cancel()
            availableModels = []
            selectedModelID = nil
            showsModelRefreshActivity = false
            modelErrorMessage = nil
            return
        }

        modelActivityTask?.cancel()
        isLoadingModels = true

        let shouldShowActivityImmediately = userInitiated
        let shouldDelayActivity = !userInitiated && availableModels.isEmpty

        if shouldShowActivityImmediately {
            showsModelRefreshActivity = true
        } else {
            showsModelRefreshActivity = false
        }

        if shouldDelayActivity {
            modelActivityTask = Task { @MainActor [weak self] in
                try? await Task.sleep(for: .milliseconds(180))
                guard let self, !Task.isCancelled, self.isLoadingModels else { return }
                self.showsModelRefreshActivity = true
            }
        } else {
            modelActivityTask = nil
        }

        defer {
            modelActivityTask?.cancel()
            modelActivityTask = nil
            isLoadingModels = false
            showsModelRefreshActivity = false
        }

        do {
            let models = try await client.fetchModels(forceRefresh: forceRefresh)
            availableModels = models.sorted { $0.id.localizedStandardCompare($1.id) == .orderedAscending }
            modelErrorMessage = nil
        } catch {
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
        showsModelRefreshActivity = false
        modelErrorMessage = nil
        quotaLastRefreshAt = [:]
        refreshingQuotaProviderIDs = []
    }

    func openDebugDashboard() {
        NSWorkspace.shared.open(baseURL.appending(path: "debug"))
    }

    func refreshDueQuotas(now: Date = Date()) async {
        let dueProviderIDs = quotaProviderIDs.filter { shouldRefreshQuota(for: $0, now: now) }

        guard !dueProviderIDs.isEmpty else { return }
        await refreshQuotas(for: Array(dueProviderIDs), showLoadingState: false)
    }

    func refreshQuota(for providerID: String, showLoadingState: Bool = true) async {
        guard quotaProviderIDs.contains(providerID) else { return }
        await refreshQuotas(for: [providerID], showLoadingState: showLoadingState)
    }

    private func refreshProviderQuotas(for providers: [GatewayProvider]) async {
        let providerIDs = Set(providers.filter(\.supportsQuotaDisplay).map(\.id))
        trimQuotaState(to: providerIDs)
        quotaErrors = [:]
        quotaLoadingProviderIDs = providerIDs

        guard !providerIDs.isEmpty else {
            quotaLoadingProviderIDs = []
            return
        }

        await refreshQuotas(
            for: Array(providerIDs),
            showLoadingState: true,
            replaceExistingQuotas: true
        )
    }

    private func refreshQuotas(
        for providerIDs: [String],
        showLoadingState: Bool,
        replaceExistingQuotas: Bool = false
    ) async {
        let refreshableIDs = Array(Set(providerIDs)).filter { !refreshingQuotaProviderIDs.contains($0) }
        guard !refreshableIDs.isEmpty else { return }

        refreshingQuotaProviderIDs.formUnion(refreshableIDs)
        if showLoadingState {
            quotaLoadingProviderIDs.formUnion(refreshableIDs)
        }

        defer {
            refreshingQuotaProviderIDs.subtract(refreshableIDs)
            if showLoadingState {
                quotaLoadingProviderIDs.subtract(refreshableIDs)
            }
        }

        let client = self.client
        var fetchedQuotas: [String: ProviderQuotaSummary] = [:]
        var nextErrors: [String: String] = [:]

        await withTaskGroup(of: (String, Result<ProviderQuotaSummary, Error>).self) { group in
            for providerID in refreshableIDs {
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
                    fetchedQuotas[providerID] = quota
                case .failure(let error):
                    nextErrors[providerID] = error.localizedDescription
                }

                quotaLastRefreshAt[providerID] = Date()
            }
        }

        if replaceExistingQuotas {
            providerQuotas = fetchedQuotas
        } else {
            for (providerID, quota) in fetchedQuotas {
                providerQuotas[providerID] = quota
            }
        }

        for providerID in refreshableIDs {
            if let error = nextErrors[providerID] {
                quotaErrors[providerID] = error
            } else {
                quotaErrors.removeValue(forKey: providerID)
            }
        }
    }

    private func shouldRefreshQuota(for providerID: String, now: Date) -> Bool {
        guard !refreshingQuotaProviderIDs.contains(providerID) else {
            return false
        }

        guard let lastRefreshAt = quotaLastRefreshAt[providerID] else {
            return true
        }

        return now.timeIntervalSince(lastRefreshAt) >= quotaRefreshInterval(for: providerID)
    }

    private func quotaRefreshInterval(for providerID: String) -> TimeInterval {
        if quotaRemainingPercent(for: providerID) < QuotaRefreshPolicy.lowRemainingThreshold {
            return selectedProviderID == providerID
                ? QuotaRefreshPolicy.lowRemainingSelectedInterval
                : QuotaRefreshPolicy.lowRemainingUnselectedInterval
        }

        return selectedProviderID == providerID
            ? QuotaRefreshPolicy.selectedInterval
            : QuotaRefreshPolicy.unselectedInterval
    }

    private func quotaRemainingPercent(for providerID: String) -> Double {
        guard let summary = providerQuotas[providerID] else {
            return 100
        }

        let remainingPercents = summary.snapshots.flatMap { snapshot in
            [snapshot.primary?.remainingPercent, snapshot.secondary?.remainingPercent].compactMap { $0 }
        }

        return remainingPercents.min() ?? 100
    }

    private var quotaProviderIDs: Set<String> {
        Set(providers.filter(\.supportsQuotaDisplay).map(\.id))
    }

    private func trimQuotaState(to providerIDs: Set<String>) {
        providerQuotas = providerQuotas.filter { providerIDs.contains($0.key) }
        quotaErrors = quotaErrors.filter { providerIDs.contains($0.key) }
        quotaLastRefreshAt = quotaLastRefreshAt.filter { providerIDs.contains($0.key) }
    }
}

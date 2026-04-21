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
    @Published private(set) var deletingProviderIDs: Set<String> = []

    let baseURL: URL
    private let client: GatewayAPIClient
    private var modelActivityTask: Task<Void, Never>?
    private var selectedProviderRefreshTask: Task<Void, Never>?
    private var providerSelectionGeneration = 0
    private var modelRefreshGeneration = 0
    private var quotaLastRefreshAt: [String: Date] = [:]
    private var refreshingQuotaProviderIDs: Set<String> = []

    init(baseURL: URL = URL(string: "http://127.0.0.1:10100")!) {
        self.baseURL = baseURL
        self.client = GatewayAPIClient(baseURL: baseURL)
    }

    var selectedProviderName: String? {
        providers.first(where: { $0.id == selectedProviderID })?.name
    }

    var selectedProvider: GatewayProvider? {
        providers.first(where: { $0.id == selectedProviderID })
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

    func isDeletingProvider(id: String) -> Bool {
        deletingProviderIDs.contains(id)
    }

    func deleteProvider(id: String) async -> Bool {
        guard !deletingProviderIDs.contains(id) else { return false }
        deletingProviderIDs.insert(id)
        defer { deletingProviderIDs.remove(id) }

        do {
            try await client.deleteProvider(id: id)

            providers.removeAll { $0.id == id }
            providerQuotas.removeValue(forKey: id)
            quotaErrors.removeValue(forKey: id)
            quotaLoadingProviderIDs.remove(id)
            quotaLastRefreshAt.removeValue(forKey: id)
            refreshingQuotaProviderIDs.remove(id)

            if selectedProviderID == id {
                providerSelectionGeneration += 1
                selectedProviderRefreshTask?.cancel()
                selectedProviderRefreshTask = nil
                selectedProviderID = nil
                invalidateModelRefreshState()
            }

            errorMessage = nil
            return true
        } catch {
            errorMessage = error.localizedDescription
            return false
        }
    }

    func selectProvider(id: String) async {
        providerSelectionGeneration += 1
        let generation = providerSelectionGeneration
        selectedProviderRefreshTask?.cancel()
        invalidateModelRefreshState()

        selectedProviderID = id
        selectedModelID = nil
        availableModels = []
        modelErrorMessage = nil

        do {
            try await client.selectProvider(id: id)
            guard generation == providerSelectionGeneration else { return }
            selectedProviderRefreshTask = Task { @MainActor [weak self] in
                guard let self else { return }
                await self.refreshModels()
                guard !Task.isCancelled, generation == self.providerSelectionGeneration else { return }
                await self.refreshQuota(for: id, showLoadingState: true)
            }
        } catch {
            guard generation == providerSelectionGeneration else { return }
            errorMessage = error.localizedDescription
        }
    }

    func refreshModels(forceRefresh: Bool = false, userInitiated: Bool = false) async {
        guard let providerID = selectedProviderID else {
            invalidateModelRefreshState()
            return
        }

        modelActivityTask?.cancel()
        modelRefreshGeneration += 1
        let refreshGeneration = modelRefreshGeneration
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
                guard let self,
                      !Task.isCancelled,
                      self.isLoadingModels,
                      refreshGeneration == self.modelRefreshGeneration
                else { return }
                self.showsModelRefreshActivity = true
            }
        } else {
            modelActivityTask = nil
        }

        defer {
            if refreshGeneration == modelRefreshGeneration {
                modelActivityTask?.cancel()
                modelActivityTask = nil
                isLoadingModels = false
                showsModelRefreshActivity = false
            }
        }

        do {
            let models = try await client.fetchModels(forceRefresh: forceRefresh)
            guard refreshGeneration == modelRefreshGeneration,
                  selectedProviderID == providerID
            else { return }
            availableModels = models.sorted { $0.id.localizedStandardCompare($1.id) == .orderedAscending }
            modelErrorMessage = nil
        } catch {
            guard refreshGeneration == modelRefreshGeneration,
                  selectedProviderID == providerID
            else { return }
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
        deletingProviderIDs = []
    }

    func clearData() {
        providers = []
        providerQuotas = [:]
        quotaErrors = [:]
        quotaLoadingProviderIDs = []
        selectedProviderRefreshTask?.cancel()
        selectedProviderRefreshTask = nil
        providerSelectionGeneration += 1
        selectedProviderID = nil
        selectedModelID = nil
        invalidateModelRefreshState()
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
        }

        let client = self.client

        if replaceExistingQuotas {
            providerQuotas = providerQuotas.filter { !refreshableIDs.contains($0.key) }
        }

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
                    providerQuotas[providerID] = quota
                    quotaErrors.removeValue(forKey: providerID)
                case .failure(let error):
                    quotaErrors[providerID] = error.localizedDescription
                }

                quotaLastRefreshAt[providerID] = Date()
                if showLoadingState {
                    quotaLoadingProviderIDs.remove(providerID)
                }
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

    private func invalidateModelRefreshState() {
        modelRefreshGeneration += 1
        modelActivityTask?.cancel()
        modelActivityTask = nil
        availableModels = []
        selectedModelID = nil
        isLoadingModels = false
        showsModelRefreshActivity = false
        modelErrorMessage = nil
    }
}

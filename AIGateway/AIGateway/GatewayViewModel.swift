import AppKit
import Combine
import Foundation

@MainActor
final class GatewayViewModel: ObservableObject {
    @Published var providers: [GatewayProvider] = []
    @Published var selectedProviderID: String?
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

    func refresh() async {
        isLoading = true
        defer { isLoading = false }

        do {
            async let providersTask = client.fetchProviders()
            async let selectedTask = client.fetchSelectedProvider()

            let (providers, selected) = try await (providersTask, selectedTask)
            self.providers = providers.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            self.selectedProviderID = selected.providerID
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

    func dismissError() {
        errorMessage = nil
    }
}

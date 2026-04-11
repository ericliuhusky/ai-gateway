import Foundation

enum GatewayAuthMode: String, Codable {
    case apiKey = "api_key"
    case account
}

enum GatewayBillingMode: String, Codable, CaseIterable, Identifiable {
    case metered
    case subscription

    var id: String { rawValue }

    var title: String {
        switch self {
        case .metered:
            return "Metered"
        case .subscription:
            return "Subscription"
        }
    }
}

struct GatewayProvider: Codable, Identifiable, Hashable {
    let id: String
    let name: String
    let authMode: GatewayAuthMode
    let baseURL: String
    let accountID: String?
    let billingMode: GatewayBillingMode
    let apiKeyPreview: String

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case authMode = "auth_mode"
        case baseURL = "base_url"
        case accountID = "account_id"
        case billingMode = "billing_mode"
        case apiKeyPreview = "api_key_preview"
    }

    var billingModeLabel: String {
        billingMode.title
    }
}

struct ProvidersResponse: Codable {
    let providers: [GatewayProvider]
}

struct SelectedProviderPayload: Codable {
    let providerID: String?
    let updatedAt: Int64

    enum CodingKeys: String, CodingKey {
        case providerID = "provider_id"
        case updatedAt = "updated_at"
    }
}

struct SelectedProviderResponse: Codable {
    let selectedProvider: SelectedProviderPayload

    enum CodingKeys: String, CodingKey {
        case selectedProvider = "selected_provider"
    }
}

struct CreateAPIProviderRequest: Codable {
    let name: String
    let baseURL: String
    let apiKey: String
    let billingMode: GatewayBillingMode

    enum CodingKeys: String, CodingKey {
        case name
        case baseURL = "base_url"
        case apiKey = "api_key"
        case billingMode = "billing_mode"
    }
}

struct UpdateSelectedProviderRequest: Codable {
    let providerID: String

    enum CodingKeys: String, CodingKey {
        case providerID = "provider_id"
    }
}

enum ProviderCreationMode: String, CaseIterable, Identifiable {
    case apiKey
    case account

    var id: String { rawValue }
}

enum AccountLoginProvider: String, CaseIterable, Identifiable {
    case google
    case openai

    var id: String { rawValue }

    var title: String {
        switch self {
        case .google:
            return "Google Account"
        case .openai:
            return "OpenAI Account"
        }
    }
}

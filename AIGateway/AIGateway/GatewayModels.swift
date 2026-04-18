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
            return "按量"
        case .subscription:
            return "订阅"
        }
    }
}

struct GatewayProvider: Codable, Identifiable, Hashable {
    let id: String
    let name: String
    let authMode: GatewayAuthMode
    let baseURL: String
    let accountID: String?
    let accountEmail: String?
    let billingMode: GatewayBillingMode
    let usesChatCompletions: Bool
    let apiKeyPreview: String

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case authMode = "auth_mode"
        case baseURL = "base_url"
        case accountID = "account_id"
        case accountEmail = "account_email"
        case billingMode = "billing_mode"
        case usesChatCompletions = "uses_chat_completions"
        case apiKeyPreview = "api_key_preview"
    }

    var billingModeLabel: String {
        billingMode.title
    }

    var supportsQuotaDisplay: Bool {
        authMode == .account
    }
}

struct ProvidersResponse: Codable {
    let providers: [GatewayProvider]
}

enum QuotaSupportStatus: String, Codable {
    case supported
    case unsupported
}

struct ProviderQuotaWindow: Codable, Hashable {
    let usedPercent: Double
    let windowMinutes: Int?
    let resetsAt: Int64?

    enum CodingKeys: String, CodingKey {
        case usedPercent = "used_percent"
        case windowMinutes = "window_minutes"
        case resetsAt = "resets_at"
    }

    var remainingPercent: Double {
        min(max(100 - usedPercent, 0), 100)
    }

    var resetDate: Date? {
        guard let resetsAt else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(resetsAt))
    }
}

struct ProviderQuotaCredits: Codable, Hashable {
    let hasCredits: Bool
    let unlimited: Bool
    let balance: String?

    enum CodingKeys: String, CodingKey {
        case hasCredits = "has_credits"
        case unlimited
        case balance
    }
}

struct ProviderQuotaSnapshot: Codable, Hashable {
    let limitID: String?
    let limitName: String?
    let primary: ProviderQuotaWindow?
    let secondary: ProviderQuotaWindow?
    let credits: ProviderQuotaCredits?
    let planType: String?

    enum CodingKeys: String, CodingKey {
        case limitID = "limit_id"
        case limitName = "limit_name"
        case primary
        case secondary
        case credits
        case planType = "plan_type"
    }
}

struct ProviderQuotaSummary: Codable, Hashable {
    let status: QuotaSupportStatus
    let snapshot: ProviderQuotaSnapshot?
    let additionalSnapshots: [ProviderQuotaSnapshot]
    let message: String?

    enum CodingKeys: String, CodingKey {
        case status
        case snapshot
        case additionalSnapshots = "additional_snapshots"
        case message
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        status = try container.decode(QuotaSupportStatus.self, forKey: .status)
        snapshot = try container.decodeIfPresent(ProviderQuotaSnapshot.self, forKey: .snapshot)
        additionalSnapshots =
            try container.decodeIfPresent([ProviderQuotaSnapshot].self, forKey: .additionalSnapshots) ?? []
        message = try container.decodeIfPresent(String.self, forKey: .message)
    }

    var snapshots: [ProviderQuotaSnapshot] {
        let primarySnapshot = snapshot.map { [$0] } ?? []
        return primarySnapshot + additionalSnapshots
    }

    var primaryWindow: ProviderQuotaWindow? {
        snapshot?.primary
            ?? snapshot?.secondary
            ?? additionalSnapshots.lazy.compactMap(\.primary).first
            ?? additionalSnapshots.lazy.compactMap(\.secondary).first
    }

    var secondaryWindow: ProviderQuotaWindow? {
        snapshot?.secondary
    }

    var creditBalance: String? {
        snapshots.compactMap { $0.credits?.balance }.first
    }

    var hasUnlimitedCredits: Bool {
        snapshots.contains { $0.credits?.unlimited == true }
    }
}

struct ProviderQuotaResponse: Codable {
    let quota: ProviderQuotaSummary
}

struct SelectedProviderPayload: Codable {
    let providerID: String?
    let selectedModel: String?
    let updatedAt: Int64

    enum CodingKeys: String, CodingKey {
        case providerID = "provider_id"
        case selectedModel = "selected_model"
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
    let usesChatCompletions: Bool

    enum CodingKeys: String, CodingKey {
        case name
        case baseURL = "base_url"
        case apiKey = "api_key"
        case billingMode = "billing_mode"
        case usesChatCompletions = "uses_chat_completions"
    }
}

struct UpdateSelectedProviderRequest: Codable {
    let providerID: String

    enum CodingKeys: String, CodingKey {
        case providerID = "provider_id"
    }
}

struct GatewayModel: Codable, Identifiable, Hashable {
    let id: String
}

struct ModelListResponse: Codable {
    let data: [GatewayModel]
}

struct SelectedModelResponse: Codable {
    let selectedModel: SelectedProviderPayload

    enum CodingKeys: String, CodingKey {
        case selectedModel = "selected_model"
    }
}

struct UpdateSelectedModelRequest: Codable {
    let model: String
}

struct CodexConfigStatus: Codable {
    let targetPath: String
    let authPath: String
    let configBackupExists: Bool
    let authBackupExists: Bool
    let restoreAvailable: Bool
    let targetExists: Bool
    let authExists: Bool

    enum CodingKeys: String, CodingKey {
        case targetPath = "target_path"
        case authPath = "auth_path"
        case configBackupExists = "config_backup_exists"
        case authBackupExists = "auth_backup_exists"
        case restoreAvailable = "restore_available"
        case targetExists = "target_exists"
        case authExists = "auth_exists"
    }
}

struct CodexConfigStatusResponse: Codable {
    let codexConfig: CodexConfigStatus

    enum CodingKeys: String, CodingKey {
        case codexConfig = "codex_config"
    }
}

struct ImportOpenAiFromLocalResponse: Codable {
    let imported: Bool
    let email: String
    let accountID: String
    let hasResponsesWrite: Bool
    let sourcePath: String

    enum CodingKeys: String, CodingKey {
        case imported
        case email
        case accountID = "account_id"
        case hasResponsesWrite = "has_responses_write"
        case sourcePath = "source_path"
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
            return "Google"
        case .openai:
            return "OpenAI"
        }
    }
}

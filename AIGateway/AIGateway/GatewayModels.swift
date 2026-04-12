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
    let apiKeyPreview: String

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case authMode = "auth_mode"
        case baseURL = "base_url"
        case accountID = "account_id"
        case accountEmail = "account_email"
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

struct GatewayLogSummary: Codable, Identifiable, Hashable {
    let requestID: String
    let createdAt: Int64
    let updatedAt: Int64
    let providerName: String?
    let accountEmail: String?
    let model: String?
    let stream: Bool
    let statusCode: Int?
    let hasError: Bool
    let errorMessage: String?
    let ingressProtocol: String?
    let egressProtocol: String?
    let eventCount: Int

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case createdAt = "created_at"
        case updatedAt = "updated_at"
        case providerName = "provider_name"
        case accountEmail = "account_email"
        case model
        case stream
        case statusCode = "status_code"
        case hasError = "has_error"
        case errorMessage = "error_message"
        case ingressProtocol = "ingress_protocol"
        case egressProtocol = "egress_protocol"
        case eventCount = "event_count"
    }

    var id: String { requestID }

    var updatedDate: Date {
        Date(timeIntervalSince1970: TimeInterval(updatedAt))
    }
}

struct GatewayLogEvent: Codable, Identifiable, Hashable {
    let id: Int64
    let requestID: String
    let stage: String
    let statusCode: Int?
    let ingressProtocol: String?
    let egressProtocol: String?
    let providerName: String?
    let accountID: String?
    let accountEmail: String?
    let model: String?
    let stream: Bool
    let method: String?
    let path: String?
    let url: String?
    let body: String?
    let bodyTruncated: Bool
    let errorMessage: String?
    let errorTruncated: Bool
    let elapsedMS: Int64?
    let createdAt: Int64

    enum CodingKeys: String, CodingKey {
        case id
        case requestID = "request_id"
        case stage
        case statusCode = "status_code"
        case ingressProtocol = "ingress_protocol"
        case egressProtocol = "egress_protocol"
        case providerName = "provider_name"
        case accountID = "account_id"
        case accountEmail = "account_email"
        case model
        case stream
        case method
        case path
        case url
        case body
        case bodyTruncated = "body_truncated"
        case errorMessage = "error_message"
        case errorTruncated = "error_truncated"
        case elapsedMS = "elapsed_ms"
        case createdAt = "created_at"
    }

    var createdDate: Date {
        Date(timeIntervalSince1970: TimeInterval(createdAt))
    }
}

struct GatewayLogDetail: Codable, Hashable {
    let requestID: String
    let events: [GatewayLogEvent]

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case events
    }
}

struct GatewayLogListResponse: Codable {
    let logs: [GatewayLogSummary]
}

struct GatewayLogDetailResponse: Codable {
    let log: GatewayLogDetail
}

struct GatewayLoggingSettings: Codable, Hashable {
    let enabled: Bool
}

struct GatewayLoggingSettingsResponse: Codable {
    let logging: GatewayLoggingSettings
}

struct UpdateGatewayLoggingSettingsRequest: Codable {
    let enabled: Bool
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

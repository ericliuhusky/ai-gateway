import Foundation

enum GatewayEndpoints {
    static let agentBaseURL = URL(string: "http://127.0.0.1:10101")!

    static var serverBaseURL: URL {
        if let configured = UserDefaults.standard.string(forKey: "gatewayServerURL"),
           let url = normalizedURL(configured)
        {
            return url
        }
        if let configured = ProcessInfo.processInfo.environment["AI_GATEWAY_SERVER_URL"],
           let url = normalizedURL(configured)
        {
            return url
        }
        return URL(string: "http://127.0.0.1:10100")!
    }

    private static func normalizedURL(_ value: String) -> URL? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard let url = URL(string: trimmed),
              ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
              url.host != nil
        else {
            return nil
        }
        return url
    }
}

import Foundation

enum GatewayAPIError: LocalizedError {
    case invalidResponse
    case server(String)

    var errorDescription: String? {
        switch self {
        case .invalidResponse:
            return "The gateway returned an invalid response."
        case .server(let message):
            return message
        }
    }
}

struct GatewayAPIClient: Sendable {
    let baseURL: URL

    func fetchProviders() async throws -> [GatewayProvider] {
        let response: ProvidersResponse = try await request(path: "/providers")
        return response.providers
    }

    func fetchProviderQuota(providerID: String) async throws -> ProviderQuotaSummary {
        let response: ProviderQuotaResponse = try await request(path: "/providers/\(providerID)/quota")
        return response.quota
    }

    func fetchSelectedProvider() async throws -> SelectedProviderPayload {
        let response: SelectedProviderResponse = try await request(path: "/selected-provider")
        return response.selectedProvider
    }

    func fetchCodexConfigStatus() async throws -> CodexConfigStatus {
        let response: CodexConfigStatusResponse = try await request(path: "/codex-config")
        return response.codexConfig
    }

    func createProvider(_ payload: CreateAPIProviderRequest) async throws {
        _ = try await requestWithoutBody(
            path: "/providers",
            method: "POST",
            body: payload
        )
    }

    func selectProvider(id: String) async throws {
        _ = try await requestWithoutBody(
            path: "/selected-provider",
            method: "PUT",
            body: UpdateSelectedProviderRequest(providerID: id)
        )
    }

    func applyCodexConfig() async throws -> CodexConfigStatus {
        let response: CodexConfigStatusResponse = try await request(path: "/codex-config", method: "PUT")
        return response.codexConfig
    }

    func restoreCodexConfig() async throws -> CodexConfigStatus {
        let response: CodexConfigStatusResponse = try await request(path: "/codex-config", method: "DELETE")
        return response.codexConfig
    }

    func loginURL(for provider: AccountLoginProvider) -> URL {
        switch provider {
        case .google:
            return baseURL.appending(path: "auth/google/start")
        case .openai:
            return baseURL.appending(path: "auth/openai/start")
        }
    }

    private func request<T: Decodable>(
        path: String,
        method: String = "GET",
        queryItems: [URLQueryItem] = []
    ) async throws -> T {
        var request = URLRequest(url: try endpointURL(path: path, queryItems: queryItems))
        request.httpMethod = method
        let (data, response) = try await URLSession.shared.data(for: request)
        try validate(response: response, data: data)
        return try JSONDecoder().decode(T.self, from: data)
    }

    private func requestWithoutBody<T: Encodable>(
        path: String,
        method: String,
        body: T
    ) async throws -> Data {
        var request = URLRequest(url: try endpointURL(path: path))
        request.httpMethod = method
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)
        let (data, response) = try await URLSession.shared.data(for: request)
        try validate(response: response, data: data)
        return data
    }

    private func requestWithBody<T: Encodable, U: Decodable>(
        path: String,
        method: String,
        body: T
    ) async throws -> U {
        var request = URLRequest(url: try endpointURL(path: path))
        request.httpMethod = method
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)
        let (data, response) = try await URLSession.shared.data(for: request)
        try validate(response: response, data: data)
        return try JSONDecoder().decode(U.self, from: data)
    }

    private func endpointURL(
        path: String,
        queryItems: [URLQueryItem] = []
    ) throws -> URL {
        let baseWithPath = baseURL.appending(path: path)
        guard var components = URLComponents(url: baseWithPath, resolvingAgainstBaseURL: false) else {
            throw GatewayAPIError.invalidResponse
        }
        if !queryItems.isEmpty {
            components.queryItems = queryItems
        }
        guard let url = components.url else {
            throw GatewayAPIError.invalidResponse
        }
        return url
    }

    private func validate(response: URLResponse, data: Data) throws {
        guard let http = response as? HTTPURLResponse else {
            throw GatewayAPIError.invalidResponse
        }

        guard (200 ..< 300).contains(http.statusCode) else {
            if
                let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let error = object["error"] as? [String: Any],
                let message = error["message"] as? String
            {
                throw GatewayAPIError.server(message)
            }

            if
                let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let error = object["error"] as? String
            {
                throw GatewayAPIError.server(error)
            }

            let message = String(data: data, encoding: .utf8) ?? "HTTP \(http.statusCode)"
            throw GatewayAPIError.server(message)
        }
    }
}

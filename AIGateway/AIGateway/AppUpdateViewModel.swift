import AppKit
import Combine
import Foundation

@MainActor
final class AppUpdateViewModel: ObservableObject {
    private enum Constants {
        static let owner = "ericliuhusky"
        static let repo = "ai-gateway"
        static let releaseAssetName = "AIGateway.app.zip"
    }

    enum UpdateState: Equatable {
        case hidden
        case available(version: String)
        case downloading(version: String)
        case installing(version: String)
        case failed(String)
    }

    private struct GitHubRelease: Decodable {
        struct Asset: Decodable {
            let name: String
            let browserDownloadURL: URL

            enum CodingKeys: String, CodingKey {
                case name
                case browserDownloadURL = "browser_download_url"
            }
        }

        let tagName: String
        let assets: [Asset]

        enum CodingKeys: String, CodingKey {
            case tagName = "tag_name"
            case assets
        }
    }

    @Published private(set) var state: UpdateState = .hidden

    private let session: URLSession
    private let serviceSupervisor: GatewayServiceSupervisor
    private let fileManager = FileManager.default
    private var latestRelease: GitHubRelease?
    private var lastCheckedAt: Date?

    init(
        serviceSupervisor: GatewayServiceSupervisor,
        session: URLSession = .shared
    ) {
        self.serviceSupervisor = serviceSupervisor
        self.session = session
    }

    var shouldShowButton: Bool {
        switch state {
        case .available, .downloading, .installing:
            return true
        case .hidden, .failed:
            return false
        }
    }

    var buttonTitle: String {
        switch state {
        case .available:
            return "更新"
        case .downloading:
            return "下载中..."
        case .installing:
            return "安装中..."
        case .hidden, .failed:
            return "更新"
        }
    }

    var isBusy: Bool {
        switch state {
        case .downloading, .installing:
            return true
        case .hidden, .available, .failed:
            return false
        }
    }

    func checkForUpdatesOnLaunch() async {
        await checkForUpdates()
    }

    func checkForUpdates() async {
        guard !isBusy else { return }

        do {
            let release = try await fetchLatestRelease()
            lastCheckedAt = Date()

            let remoteVersion = normalizeVersion(release.tagName)
            let localVersion = localAppVersion()

            guard isRemoteVersionNewer(remoteVersion, than: localVersion) else {
                latestRelease = nil
                if case .failed = state {
                    state = .hidden
                } else if case .available = state {
                    state = .hidden
                }
                return
            }

            latestRelease = release
            if case .downloading = state { return }
            if case .installing = state { return }
            state = .available(version: remoteVersion)
        } catch {
            if case .downloading = state { return }
            if case .installing = state { return }
            state = .failed(error.localizedDescription)
        }
    }

    func updateApp() async {
        guard let release = latestRelease else {
            await checkForUpdates()
            guard latestRelease != nil else { return }
            await updateApp()
            return
        }

        let remoteVersion = normalizeVersion(release.tagName)

        do {
            guard let asset = release.assets.first(where: { $0.name == Constants.releaseAssetName }) else {
                throw AppUpdateError.missingReleaseAsset(Constants.releaseAssetName)
            }

            state = .downloading(version: remoteVersion)
            let zipURL = try await downloadAsset(from: asset.browserDownloadURL)

            state = .installing(version: remoteVersion)
            let installedAppURL = try prepareInstalledApp(from: zipURL)
            await serviceSupervisor.stopService()
            try launchInstaller(appURL: installedAppURL)
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    private func fetchLatestRelease() async throws -> GitHubRelease {
        let url = URL(string: "https://api.github.com/repos/\(Constants.owner)/\(Constants.repo)/releases/latest")!
        var request = URLRequest(url: url)
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        request.setValue("AIGateway", forHTTPHeaderField: "User-Agent")

        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw AppUpdateError.invalidServerResponse
        }
        guard (200 ..< 300).contains(http.statusCode) else {
            if http.statusCode == 404 {
                throw AppUpdateError.latestReleaseNotFound
            }
            throw AppUpdateError.httpStatus(http.statusCode)
        }

        return try JSONDecoder().decode(GitHubRelease.self, from: data)
    }

    private func downloadAsset(from remoteURL: URL) async throws -> URL {
        let (temporaryURL, response) = try await session.download(from: remoteURL)
        guard let http = response as? HTTPURLResponse,
              (200 ..< 300).contains(http.statusCode)
        else {
            throw AppUpdateError.downloadFailed
        }

        let destinationURL = fileManager.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
            .appendingPathComponent(Constants.releaseAssetName, isDirectory: false)

        try fileManager.createDirectory(
            at: destinationURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        if fileManager.fileExists(atPath: destinationURL.path) {
            try fileManager.removeItem(at: destinationURL)
        }
        try fileManager.moveItem(at: temporaryURL, to: destinationURL)
        return destinationURL
    }

    private func prepareInstalledApp(from zipURL: URL) throws -> URL {
        let extractionDirectory = zipURL.deletingLastPathComponent()
            .appendingPathComponent("extracted", isDirectory: true)

        if fileManager.fileExists(atPath: extractionDirectory.path) {
            try fileManager.removeItem(at: extractionDirectory)
        }
        try fileManager.createDirectory(at: extractionDirectory, withIntermediateDirectories: true)

        try runProcess(
            executablePath: "/usr/bin/ditto",
            arguments: ["-x", "-k", zipURL.path, extractionDirectory.path]
        )

        let appURL = extractionDirectory.appendingPathComponent("AIGateway.app", isDirectory: true)
        guard fileManager.fileExists(atPath: appURL.path) else {
            throw AppUpdateError.extractedAppMissing
        }

        return appURL
    }

    private func launchInstaller(appURL: URL) throws {
        let currentAppURL = Bundle.main.bundleURL
        let destinationAppURL = currentAppURL.deletingLastPathComponent()
            .appendingPathComponent("AIGateway.app", isDirectory: true)

        let script = """
        set -e
        while pgrep -x "AIGateway" >/dev/null; do
          sleep 0.2
        done
        rm -rf "\(destinationAppURL.path)"
        cp -R "\(appURL.path)" "\(destinationAppURL.path)"
        open "\(destinationAppURL.path)"
        """

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = ["-c", script]
        try process.run()

        NSApp.terminate(nil)
    }

    private func runProcess(executablePath: String, arguments: [String]) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executablePath)
        process.arguments = arguments

        let stderr = Pipe()
        process.standardError = stderr

        try process.run()
        process.waitUntilExit()

        guard process.terminationStatus == 0 else {
            let error = String(decoding: stderr.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            throw AppUpdateError.processFailed(executablePath, error.trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }

    private func localAppVersion() -> String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0"
    }

    private func normalizeVersion(_ value: String) -> String {
        value.hasPrefix("v") ? String(value.dropFirst()) : value
    }

    private func isRemoteVersionNewer(_ remote: String, than local: String) -> Bool {
        remote.compare(local, options: .numeric) == .orderedDescending
    }
}

private enum AppUpdateError: LocalizedError {
    case invalidServerResponse
    case httpStatus(Int)
    case latestReleaseNotFound
    case missingReleaseAsset(String)
    case downloadFailed
    case extractedAppMissing
    case processFailed(String, String)

    var errorDescription: String? {
        switch self {
        case .invalidServerResponse:
            return "更新接口返回了无效响应。"
        case .httpStatus(let statusCode):
            return "检查更新失败，GitHub 返回 HTTP \(statusCode)。"
        case .latestReleaseNotFound:
            return "未找到 latest release。仓库可能仍是私有的，或尚未发布正式 Release。"
        case .missingReleaseAsset(let name):
            return "最新 Release 里没有找到 \(name)。"
        case .downloadFailed:
            return "下载更新包失败。"
        case .extractedAppMissing:
            return "解压后没有找到 AIGateway.app。"
        case .processFailed(let executable, let message):
            if message.isEmpty {
                return "执行 \(executable) 失败。"
            }
            return "执行 \(executable) 失败：\(message)"
        }
    }
}

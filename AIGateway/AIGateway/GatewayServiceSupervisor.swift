import AppKit
import Combine
import Darwin
import Foundation

@MainActor
final class GatewayServiceSupervisor: ObservableObject {
    static let serviceLabel = "ericliu.husky.ai-gateway.server"
    static let installedBinaryName = "ai-gateway-server"

    enum Status: Equatable {
        case checking
        case installing
        case starting
        case runningLaunchAgent(pid: Int32?)
        case runningExternal
        case installedStopped
        case notInstalled
        case failed(String)
    }

    @Published private(set) var status: Status = .checking

    let baseURL: URL

    private var startupTask: Task<Void, Error>?
    private let fileManager = FileManager.default

    init(baseURL: URL = URL(string: "http://127.0.0.1:10100")!) {
        self.baseURL = baseURL
    }

    var isReachable: Bool {
        switch status {
        case .runningLaunchAgent, .runningExternal:
            return true
        case .checking, .installing, .starting, .installedStopped, .notInstalled, .failed:
            return false
        }
    }

    var isBusy: Bool {
        switch status {
        case .checking, .installing, .starting:
            return true
        case .runningLaunchAgent, .runningExternal, .installedStopped, .notInstalled, .failed:
            return false
        }
    }

    var canStart: Bool {
        !isBusy && !isReachable
    }

    var canStop: Bool {
        if case .runningLaunchAgent = status {
            return true
        }
        return false
    }

    var statusTitle: String {
        switch status {
        case .checking:
            return "检查中"
        case .installing:
            return "启动中"
        case .starting:
            return "启动中"
        case .runningLaunchAgent:
            return "运行中"
        case .runningExternal:
            return "运行中"
        case .installedStopped:
            return "未启动"
        case .notInstalled:
            return "未启动"
        case .failed:
            return "启动失败"
        }
    }

    var statusDetail: String {
        switch status {
        case .checking:
            return "正在检查服务状态。"
        case .installing:
            return "正在准备服务文件。"
        case .starting:
            return "正在启动服务，请稍候。"
        case .runningLaunchAgent(let pid):
            _ = pid
            return "服务正在运行。"
        case .runningExternal:
            return "服务正在运行。"
        case .installedStopped:
            return "服务当前未启动。"
        case .notInstalled:
            return "服务当前未安装。点击启动服务后会自动完成安装。"
        case .failed(let message):
            return message
        }
    }

    func ensureServiceRunning() async throws {
        if let startupTask {
            try await startupTask.value
            return
        }

        let task = Task<Void, Error> {
            let launchd = try await self.inspectLaunchAgent()
            if launchd.isLoaded, await self.isServerHealthy() {
                self.status = .runningLaunchAgent(pid: launchd.pid)
                return
            }

            if !launchd.isLoaded, await self.isServerHealthy() {
                self.status = .runningExternal
                return
            }

            try await self.startLaunchAgent()
        }

        startupTask = task
        defer { startupTask = nil }
        try await task.value
    }

    func refreshStatus() async {
        do {
            let launchd = try await inspectLaunchAgent()
            if launchd.isLoaded {
                if await isServerHealthy() {
                    status = .runningLaunchAgent(pid: launchd.pid)
                } else {
                    status = .installedStopped
                }
                return
            }

            if await isServerHealthy() {
                status = .runningExternal
            } else if fileManager.fileExists(atPath: installedBinaryURL.path) || fileManager.fileExists(atPath: launchAgentPlistURL.path) {
                status = .installedStopped
            } else {
                status = .notInstalled
            }
        } catch {
            status = .failed(error.localizedDescription)
        }
    }

    func startService() async throws {
        try await ensureServiceRunning()
    }

    func stopService() async {
        do {
            let launchd = try await inspectLaunchAgent()
            guard launchd.isLoaded else {
                await refreshStatus()
                return
            }

            _ = try await runLaunchctl(arguments: ["bootout", launchDomain, launchAgentPlistURL.path], allowFailure: false)
            await refreshStatus()
        } catch {
            status = .failed(error.localizedDescription)
        }
    }

    func openDebugDashboard() {
        NSWorkspace.shared.open(baseURL.appending(path: "debug"))
    }

    private func startLaunchAgent() async throws {
        status = .installing
        try installOrUpdateLaunchAgentFiles()

        let launchd = try await inspectLaunchAgent()
        status = .starting

        if !launchd.isLoaded {
            _ = try await runLaunchctl(
                arguments: ["bootstrap", launchDomain, launchAgentPlistURL.path],
                allowFailure: false
            )
        }

        _ = try await runLaunchctl(
            arguments: ["kickstart", "-k", launchTarget],
            allowFailure: false
        )

        for _ in 0 ..< 40 {
            let refreshed = try await inspectLaunchAgent()
            if refreshed.isLoaded, await isServerHealthy() {
                status = .runningLaunchAgent(pid: refreshed.pid)
                return
            }
            try? await Task.sleep(for: .milliseconds(250))
        }

        status = .failed("服务启动后健康检查超时，未能连接到 /healthz。")
        throw GatewayServiceError.startupTimedOut
    }

    private func installOrUpdateLaunchAgentFiles() throws {
        try fileManager.createDirectory(at: aiGatewayDirectoryURL, withIntermediateDirectories: true)
        try fileManager.createDirectory(at: installedBinaryURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        try fileManager.createDirectory(at: launchAgentPlistURL.deletingLastPathComponent(), withIntermediateDirectories: true)

        let bundledURL = try bundledServerExecutableURL()
        let shouldCopyBinary = !fileManager.fileExists(atPath: installedBinaryURL.path)
            || !fileManager.contentsEqual(atPath: bundledURL.path, andPath: installedBinaryURL.path)

        if shouldCopyBinary {
            if fileManager.fileExists(atPath: installedBinaryURL.path) {
                try fileManager.removeItem(at: installedBinaryURL)
            }
            try fileManager.copyItem(at: bundledURL, to: installedBinaryURL)
        }

        try fileManager.setAttributes([.posixPermissions: 0o755], ofItemAtPath: installedBinaryURL.path)

        let plistData = try launchAgentPlistData()
        let shouldWritePlist = !fileManager.fileExists(atPath: launchAgentPlistURL.path)
            || (try? Data(contentsOf: launchAgentPlistURL)) != plistData

        if shouldWritePlist {
            try plistData.write(to: launchAgentPlistURL, options: .atomic)
        }
    }

    private func launchAgentPlistData() throws -> Data {
        let plist: [String: Any] = [
            "Label": Self.serviceLabel,
            "ProgramArguments": [installedBinaryURL.path],
            "WorkingDirectory": aiGatewayDirectoryURL.path,
            "RunAtLoad": false,
            "KeepAlive": false,
        ]

        return try PropertyListSerialization.data(
            fromPropertyList: plist,
            format: .xml,
            options: 0
        )
    }

    private func inspectLaunchAgent() async throws -> LaunchAgentState {
        do {
            let output = try await runLaunchctl(arguments: ["print", launchTarget], allowFailure: false)
            return LaunchAgentState(
                isLoaded: true,
                pid: Self.extractPID(from: output)
            )
        } catch let error as GatewayServiceError {
            if case .launchctlFailed(_, _, _) = error {
                return LaunchAgentState(isLoaded: false, pid: nil)
            }
            throw error
        }
    }

    private func bundledServerExecutableURL() throws -> URL {
        guard let resourceURL = Bundle.main.resourceURL else {
            throw GatewayServiceError.missingBundleResources
        }

        let executableURL = resourceURL
            .appendingPathComponent("bin", isDirectory: true)
            .appendingPathComponent(Self.installedBinaryName, isDirectory: false)

        guard fileManager.isExecutableFile(atPath: executableURL.path) else {
            throw GatewayServiceError.missingBundledServer(executableURL.path)
        }

        return executableURL
    }

    private func isServerHealthy() async -> Bool {
        var request = URLRequest(url: baseURL.appending(path: "healthz"))
        request.httpMethod = "GET"
        request.timeoutInterval = 1.5

        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse,
                  (200 ..< 300).contains(http.statusCode)
            else {
                return false
            }
            return String(data: data, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) == "ok"
        } catch {
            return false
        }
    }

    private func runLaunchctl(arguments: [String], allowFailure: Bool) async throws -> String {
        try await Task.detached(priority: .userInitiated) {
            try Self.runLaunchctlSync(arguments: arguments, allowFailure: allowFailure)
        }.value
    }

    nonisolated private static func runLaunchctlSync(arguments: [String], allowFailure: Bool) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        process.arguments = arguments

        let stdout = Pipe()
        let stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr

        try process.run()
        process.waitUntilExit()

        let outputData = stdout.fileHandleForReading.readDataToEndOfFile()
        let errorData = stderr.fileHandleForReading.readDataToEndOfFile()
        let output = String(decoding: outputData, as: UTF8.self)
        let error = String(decoding: errorData, as: UTF8.self)
        let combined = [output, error]
            .filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .joined(separator: "\n")

        if !allowFailure && process.terminationStatus != 0 {
            throw GatewayServiceError.launchctlFailed(
                arguments.joined(separator: " "),
                Int(process.terminationStatus),
                combined
            )
        }

        return combined
    }

    private static func extractPID(from output: String) -> Int32? {
        guard let regex = try? NSRegularExpression(pattern: #"pid = (\d+)"#) else {
            return nil
        }

        let range = NSRange(output.startIndex..., in: output)
        guard let match = regex.firstMatch(in: output, range: range),
              let pidRange = Range(match.range(at: 1), in: output)
        else {
            return nil
        }

        return Int32(String(output[pidRange]))
    }

    private var homeDirectoryURL: URL {
        if let home = Self.posixHomeDirectory() {
            return home
        }
        return fileManager.homeDirectoryForCurrentUser
    }

    private var aiGatewayDirectoryURL: URL {
        homeDirectoryURL.appendingPathComponent(".ai-gateway", isDirectory: true)
    }

    private var installedBinaryURL: URL {
        aiGatewayDirectoryURL
            .appendingPathComponent("bin", isDirectory: true)
            .appendingPathComponent(Self.installedBinaryName, isDirectory: false)
    }

    private var launchAgentPlistURL: URL {
        launchAgentPlistURL(for: Self.serviceLabel)
    }

    private func launchAgentPlistURL(for label: String) -> URL {
        homeDirectoryURL
            .appendingPathComponent("Library", isDirectory: true)
            .appendingPathComponent("LaunchAgents", isDirectory: true)
            .appendingPathComponent("\(label).plist", isDirectory: false)
    }

    private var launchDomain: String {
        "gui/\(getuid())"
    }

    private var launchTarget: String {
        "\(launchDomain)/\(Self.serviceLabel)"
    }

    private static func posixHomeDirectory() -> URL? {
        guard let passwd = getpwuid(getuid()), let directory = passwd.pointee.pw_dir else {
            return nil
        }
        return URL(fileURLWithPath: String(cString: directory), isDirectory: true)
    }
}

private struct LaunchAgentState {
    let isLoaded: Bool
    let pid: Int32?
}

enum GatewayServiceError: LocalizedError {
    case missingBundleResources
    case missingBundledServer(String)
    case startupTimedOut
    case launchctlFailed(String, Int, String)

    var errorDescription: String? {
        switch self {
        case .missingBundleResources:
            return "无法读取 app bundle 资源目录。"
        case .missingBundledServer(let path):
            return "没有在 app bundle 里找到内置 server：\(path)"
        case .startupTimedOut:
            return "服务启动超时，未能通过健康检查。"
        case .launchctlFailed(let command, let code, let output):
            let suffix = output.isEmpty ? "" : "\n\(output)"
            return "launchctl 执行失败：\(command) (exit \(code))\(suffix)"
        }
    }
}

//
//  AIGatewayApp.swift
//  AIGateway
//
//  Created by 刘子豪 on 2026/4/11.
//

import SwiftUI

@main
struct AIGatewayApp: App {
    @StateObject private var viewModel: GatewayViewModel
    @StateObject private var serviceSupervisor: GatewayServiceSupervisor
    @StateObject private var updater: AppUpdateViewModel

    init() {
        let serviceSupervisor = GatewayServiceSupervisor()
        _viewModel = StateObject(wrappedValue: GatewayViewModel())
        _serviceSupervisor = StateObject(wrappedValue: serviceSupervisor)
        _updater = StateObject(wrappedValue: AppUpdateViewModel(serviceSupervisor: serviceSupervisor))
    }

    var body: some Scene {
        Window("AI Gateway", id: "main") {
            ContentView(
                viewModel: viewModel,
                serviceSupervisor: serviceSupervisor,
                updater: updater
            )
            .task {
                await updater.checkForUpdatesOnLaunch()
            }
        }

        MenuBarExtra {
            GatewayMenuBarView(
                viewModel: viewModel,
                serviceSupervisor: serviceSupervisor,
                updater: updater
            )
        } label: {
            Label("AI Gateway", systemImage: "bolt.horizontal.circle.fill")
        }
        .menuBarExtraStyle(.window)
    }
}

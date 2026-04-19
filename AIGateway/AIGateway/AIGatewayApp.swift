//
//  AIGatewayApp.swift
//  AIGateway
//
//  Created by 刘子豪 on 2026/4/11.
//

import SwiftUI

@main
struct AIGatewayApp: App {
    @StateObject private var viewModel = GatewayViewModel()
    @StateObject private var serviceSupervisor = GatewayServiceSupervisor()

    var body: some Scene {
        Window("AI Gateway", id: "main") {
            ContentView(viewModel: viewModel, serviceSupervisor: serviceSupervisor)
        }

        MenuBarExtra {
            GatewayMenuBarView(viewModel: viewModel, serviceSupervisor: serviceSupervisor)
        } label: {
            Label("AI Gateway", systemImage: "bolt.horizontal.circle.fill")
        }
        .menuBarExtraStyle(.window)
    }
}

import { invokeTauri } from "./lib/connection";
import type {
  CodexAuthPayload,
  GatewayCompatibilityProfile,
  GatewayIssue,
  GatewayModel,
  GatewayProvider,
  ProviderQuotaSummary,
  SelectedProvider,
  ReasoningEffort,
  OpenAiDeviceLoginStart,
  OpenAiDeviceLoginStatus,
  DefaultCodexStatus,
  CodexConfigurationResult,
} from "./types";

function gatewayRequest<T>(
  method: "GET" | "POST" | "PUT" | "DELETE",
  path: string,
  body?: unknown,
): Promise<T> {
  return invokeTauri<T>("gateway_request", {
    method,
    path,
    ...(body === undefined ? {} : { body }),
  });
}

export const gatewayApi = {
  async gatewayIssues(limit = 200) {
    const payload = await gatewayRequest<{ issues: GatewayIssue[] }>(
      "GET",
      `/gateway/issues?limit=${limit}`,
    );
    return payload.issues;
  },
  gatewayIssueRepairPrompt(issueId: string) {
    return gatewayRequest<{ prompt: string }>(
      "GET",
      `/gateway/issues/${encodeURIComponent(issueId)}/repair-prompt`,
    );
  },
  clearGatewayIssues() {
    return gatewayRequest<{ deleted: number }>("DELETE", "/gateway/issues");
  },
  async providers() { const payload = await gatewayRequest<{ providers: GatewayProvider[] }>("GET", "/providers"); return payload.providers; },
  async selectedProvider() { const payload = await gatewayRequest<{ selected_provider: SelectedProvider }>("GET", "/selected-provider"); return payload.selected_provider; },
  async selectProvider(providerId: string) { const payload = await gatewayRequest<{ selected_provider: SelectedProvider }>("PUT", "/selected-provider", { provider_id: providerId }); return payload.selected_provider; },
  async models(providerId?: string, force = false) { const query = new URLSearchParams(); if (providerId) query.set("provider_id", providerId); if (force) query.set("force", "true"); const payload = await gatewayRequest<{ data: GatewayModel[] }>("GET", `/openai/v1/models${query.size ? `?${query.toString()}` : ""}`); return payload.data; },
  async selectModel(model: string) { const payload = await gatewayRequest<{ selected_model: SelectedProvider }>("PUT", "/selected-model", { model }); return payload.selected_model; },
  async clearSelectedModel() { const payload = await gatewayRequest<{ selected_model: SelectedProvider }>("DELETE", "/selected-model"); return payload.selected_model; },
  async selectReasoningEffort(effort: ReasoningEffort) { const payload = await gatewayRequest<{ selected_reasoning_effort: SelectedProvider }>("PUT", "/selected-reasoning-effort", { effort }); return payload.selected_reasoning_effort; },
  async clearSelectedReasoningEffort() { const payload = await gatewayRequest<{ selected_reasoning_effort: SelectedProvider }>("DELETE", "/selected-reasoning-effort"); return payload.selected_reasoning_effort; },
  createProvider(input: { name: string; base_url: string; api_key: string; compatibility_profile: GatewayCompatibilityProfile }) { return gatewayRequest("POST", "/providers", input); },
  deleteProvider(providerId: string) { return gatewayRequest("DELETE", `/providers/${encodeURIComponent(providerId)}`); },
  async quota(providerId: string) { const payload = await gatewayRequest<{ quota: ProviderQuotaSummary }>("GET", `/providers/${encodeURIComponent(providerId)}/quota`); return payload.quota; },
  importAccount(payload: CodexAuthPayload) { return gatewayRequest("POST", "/accounts/openai/import-token", payload); },
  startOpenAiDeviceLogin() { return gatewayRequest<OpenAiDeviceLoginStart>("POST", "/accounts/openai/login/device"); },
  pollOpenAiDeviceLogin(loginId: string) { return gatewayRequest<OpenAiDeviceLoginStatus>("GET", `/accounts/openai/login/device/${encodeURIComponent(loginId)}`); },
  cancelOpenAiDeviceLogin(loginId: string) { return gatewayRequest<{ cancelled: boolean }>("DELETE", `/accounts/openai/login/device/${encodeURIComponent(loginId)}`); },
  codexGatewayStatus: () => invokeTauri<DefaultCodexStatus>("get_codex_gateway_status"),
  startCodexGateway: () => invokeTauri<CodexConfigurationResult>("start_codex_gateway"),
  stopCodexGateway: () => invokeTauri<CodexConfigurationResult>("stop_codex_gateway"),
};

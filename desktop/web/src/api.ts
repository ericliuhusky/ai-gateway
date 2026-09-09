import { invokeTauri } from "./lib/connection";
import type {
  CodexAuthPayload,
  GatewayIssue,
  GatewayModel,
  GatewayProvider,
  CodexUsageResponse,
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
      `/management/gateway/issues?limit=${limit}`,
    );
    return payload.issues;
  },
  gatewayIssueRepairPrompt(issueId: string) {
    return gatewayRequest<{ prompt: string }>(
      "GET",
      `/management/gateway/issues/${encodeURIComponent(issueId)}/repair-prompt`,
    );
  },
  clearGatewayIssues() {
    return gatewayRequest<{ deleted: number }>("DELETE", "/management/gateway/issues");
  },
  async providers() { const payload = await gatewayRequest<{ providers: GatewayProvider[] }>("GET", "/management/providers"); return payload.providers; },
  async selectedProvider() { const payload = await gatewayRequest<{ selected_provider: SelectedProvider }>("GET", "/management/selected-provider"); return payload.selected_provider; },
  async selectProvider(providerId: string) { const payload = await gatewayRequest<{ selected_provider: SelectedProvider }>("PUT", "/management/selected-provider", { provider_id: providerId }); return payload.selected_provider; },
  async models(providerId?: string, force = false) { const query = new URLSearchParams(); if (providerId) query.set("provider_id", providerId); if (force) query.set("force", "true"); const payload = await gatewayRequest<{ data: GatewayModel[] }>("GET", `/v1/models${query.size ? `?${query.toString()}` : ""}`); return payload.data; },
  async selectModel(model: string) { const payload = await gatewayRequest<{ selected_model: SelectedProvider }>("PUT", "/management/selected-model", { model }); return payload.selected_model; },
  async clearSelectedModel() { const payload = await gatewayRequest<{ selected_model: SelectedProvider }>("DELETE", "/management/selected-model"); return payload.selected_model; },
  async selectReasoningEffort(effort: ReasoningEffort) { const payload = await gatewayRequest<{ selected_reasoning_effort: SelectedProvider }>("PUT", "/management/selected-reasoning-effort", { effort }); return payload.selected_reasoning_effort; },
  async clearSelectedReasoningEffort() { const payload = await gatewayRequest<{ selected_reasoning_effort: SelectedProvider }>("DELETE", "/management/selected-reasoning-effort"); return payload.selected_reasoning_effort; },
  createProvider(input: { name: string; base_url: string; api_key: string }) { return gatewayRequest("POST", "/management/providers", input); },
  deleteProvider(providerId: string) { return gatewayRequest("DELETE", `/management/providers/${encodeURIComponent(providerId)}`); },
  async quota(providerId: string) { return gatewayRequest<CodexUsageResponse>("GET", `/management/providers/${encodeURIComponent(providerId)}/quota`); },
  importProvider(payload: CodexAuthPayload) { return gatewayRequest("POST", "/management/providers/openai/import-token", payload); },
  refreshProvider(providerId: string) { return gatewayRequest<{ provider_id: string; email: string; expiry_timestamp: number }>("POST", `/management/providers/${encodeURIComponent(providerId)}/refresh`); },
  startOpenAiDeviceLogin() { return gatewayRequest<OpenAiDeviceLoginStart>("POST", "/management/providers/openai/login/device"); },
  pollOpenAiDeviceLogin(loginId: string) { return gatewayRequest<OpenAiDeviceLoginStatus>("GET", `/management/providers/openai/login/device/${encodeURIComponent(loginId)}`); },
  cancelOpenAiDeviceLogin(loginId: string) { return gatewayRequest<{ cancelled: boolean }>("DELETE", `/management/providers/openai/login/device/${encodeURIComponent(loginId)}`); },
  codexGatewayStatus: () => invokeTauri<DefaultCodexStatus>("get_codex_gateway_status"),
  startCodexGateway: () => invokeTauri<CodexConfigurationResult>("start_codex_gateway"),
  stopCodexGateway: () => invokeTauri<CodexConfigurationResult>("stop_codex_gateway"),
  restartChatGpt: () => invokeTauri<void>("restart_chatgpt_app"),
};

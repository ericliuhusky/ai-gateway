import { invokeTauri } from "./lib/connection";
import type {
  CodexAuthPayload,
  AutoRoutingSettings,
  CodexClientVersionSetting,
  GatewayCompatibilityProfile,
  GatewayModel,
  GatewayIssue,
  GatewayProvider,
  ModelBenchmarkResult,
  InstanceRoutingConfig,
  ProviderQuotaSummary,
  ReasoningEffort,
  SelectedProvider,
  TurnRouteLog,
  UsageSummary,
  DailyUsageSummary,
  OpenAiDeviceLoginStart,
  OpenAiDeviceLoginStatus,
  CenterGroup,
  CenterGroupDetail,
  CenterUser,
  LocalGatewayStatus,
  DefaultCodexStatus,
  CodexConfigurationResult,
  ManagedCenterUser,
  SharedSyncStatus,
} from "./types";

/**
 * All management operations cross the WebView/Rust boundary via Tauri invoke.
 * They are dispatched in-process and are intentionally not HTTP endpoints.
 */
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
  codexClientVersion: () =>
    gatewayRequest<CodexClientVersionSetting>("GET", "/settings/codex-client-version"),

  setCodexClientVersion(version: string) {
    return gatewayRequest<CodexClientVersionSetting>("PUT", "/settings/codex-client-version", { version });
  },

  clearCodexClientVersion() {
    return gatewayRequest<CodexClientVersionSetting>("DELETE", "/settings/codex-client-version");
  },

  automaticRouting() {
    return gatewayRequest<AutoRoutingSettings>("GET", "/settings/automatic-routing");
  },

  setAutomaticRouting(settings: AutoRoutingSettings) {
    return gatewayRequest<AutoRoutingSettings>("PUT", "/settings/automatic-routing", settings);
  },

  async instances() {
    const payload = await gatewayRequest<{ instances: InstanceRoutingConfig[] }>("GET", "/instances");
    return payload.instances;
  },

  instanceConfig(instanceId: string) {
    return gatewayRequest<InstanceRoutingConfig>("GET", `/instances/${encodeURIComponent(instanceId)}/config`);
  },

  setInstanceConfig(
    instanceId: string,
    config: Omit<InstanceRoutingConfig, "instance_id" | "updated_at">,
  ) {
    return gatewayRequest<InstanceRoutingConfig>("PUT", `/instances/${encodeURIComponent(instanceId)}/config`, config);
  },

  async deleteInstance(instanceId: string) {
    await invokeTauri<boolean>("delete_codex_instance", { instanceId });
    return gatewayRequest<{ deleted_instance: string }>("DELETE", `/instances/${encodeURIComponent(instanceId)}`);
  },

  async routingTurns(limit = 50) {
    const payload = await gatewayRequest<{ turns: TurnRouteLog[] }>("GET", `/routing/turns?limit=${limit}`);
    return payload.turns;
  },

  async gatewayIssues(limit = 200) {
    const payload = await gatewayRequest<{ issues: GatewayIssue[] }>("GET", `/gateway/issues?limit=${limit}`);
    return payload.issues;
  },

  gatewayIssueRepairPrompt(issueId: string) {
    return gatewayRequest<{ prompt: string }>("GET", `/gateway/issues/${encodeURIComponent(issueId)}/repair-prompt`);
  },

  clearGatewayIssues() {
    return gatewayRequest<{ deleted: number }>("DELETE", "/gateway/issues");
  },

  usageSummary(period: "total" | "today" | "week", providerId?: string) {
    const query = new URLSearchParams({ period });
    if (providerId) query.set("provider_id", providerId);
    return gatewayRequest<UsageSummary[]>("GET", `/usage/summary?${query.toString()}`);
  },

  usageDaily(days = 30) {
    return gatewayRequest<DailyUsageSummary[]>("GET", `/usage/daily?days=${days}`);
  },

  async providers() {
    const payload = await gatewayRequest<{ providers: GatewayProvider[] }>("GET", "/providers");
    return payload.providers;
  },

  async selectedProvider() {
    const payload = await gatewayRequest<{ selected_provider: SelectedProvider }>("GET", "/selected-provider");
    return payload.selected_provider;
  },

  async selectProvider(providerId: string) {
    const payload = await gatewayRequest<{ selected_provider: SelectedProvider }>("PUT", "/selected-provider", { provider_id: providerId });
    return payload.selected_provider;
  },

  async models(providerId?: string, force = false) {
    const query = new URLSearchParams();
    if (providerId) query.set("provider_id", providerId);
    if (force) query.set("force", "true");
    const payload = await gatewayRequest<{ data: GatewayModel[] }>("GET", `/openai/v1/models${query.size ? `?${query.toString()}` : ""}`);
    return payload.data;
  },

  async selectModel(model: string) {
    const payload = await gatewayRequest<{ selected_model: SelectedProvider }>("PUT", "/selected-model", { model });
    return payload.selected_model;
  },

  async clearSelectedModel() {
    const payload = await gatewayRequest<{ selected_model: SelectedProvider }>("DELETE", "/selected-model");
    return payload.selected_model;
  },

  async selectReasoningEffort(effort: ReasoningEffort) {
    const payload = await gatewayRequest<{ selected_reasoning_effort: SelectedProvider }>("PUT", "/selected-reasoning-effort", { effort });
    return payload.selected_reasoning_effort;
  },

  async clearSelectedReasoningEffort() {
    const payload = await gatewayRequest<{ selected_reasoning_effort: SelectedProvider }>("DELETE", "/selected-reasoning-effort");
    return payload.selected_reasoning_effort;
  },

  createProvider(input: {
    name: string;
    base_url: string;
    api_key: string;
    compatibility_profile: GatewayCompatibilityProfile;
  }) {
    return gatewayRequest("POST", "/providers", input);
  },

  deleteProvider(providerId: string) {
    return gatewayRequest("DELETE", `/providers/${encodeURIComponent(providerId)}`);
  },

  async quota(providerId: string) {
    const payload = await gatewayRequest<{ quota: ProviderQuotaSummary }>("GET", `/providers/${encodeURIComponent(providerId)}/quota`);
    return payload.quota;
  },

  benchmarkModel(input: {
    provider_id: string;
    model: string;
    runs?: number;
    account_usage_confirmed?: boolean;
  }) {
    return gatewayRequest<ModelBenchmarkResult>("POST", "/benchmarks/models", input);
  },

  importAccount(payload: CodexAuthPayload) {
    return gatewayRequest("POST", "/accounts/openai/import-token", payload);
  },

  startOpenAiDeviceLogin() {
    return gatewayRequest<OpenAiDeviceLoginStart>("POST", "/accounts/openai/login/device");
  },

  pollOpenAiDeviceLogin(loginId: string) {
    return gatewayRequest<OpenAiDeviceLoginStatus>("GET", `/accounts/openai/login/device/${encodeURIComponent(loginId)}`);
  },

  cancelOpenAiDeviceLogin(loginId: string) {
    return gatewayRequest<{ cancelled: boolean }>("DELETE", `/accounts/openai/login/device/${encodeURIComponent(loginId)}`);
  },
};

function centerRequest<T>(
  method: "GET" | "POST" | "PUT" | "DELETE",
  path: string,
  body?: unknown,
): Promise<T> {
  return gatewayRequest<T>("POST", "/control/control-plane/request", {
    method,
    path,
    ...(body === undefined ? {} : { body }),
  });
}

export const centerApi = {
  gatewayStatus: () => gatewayRequest<LocalGatewayStatus>("GET", "/control/status"),
  codexGatewayStatus: () => invokeTauri<DefaultCodexStatus>("get_codex_gateway_status"),
  startCodexGateway: () => invokeTauri<CodexConfigurationResult>("start_codex_gateway"),
  stopCodexGateway: () => invokeTauri<CodexConfigurationResult>("stop_codex_gateway"),
  startCodexInstance: (instanceId: string) => invokeTauri<string>("start_codex_instance", { instanceId }),
  login: (input: { url: string; email: string; password: string }) =>
    gatewayRequest<{ user: CenterUser; control_plane_url: string }>("POST", "/control/control-plane/login", input),
  disconnect: () => gatewayRequest<{ disconnected: boolean }>("DELETE", "/control/control-plane"),
  sync: () => gatewayRequest<SharedSyncStatus>("POST", "/control/control-plane/sync"),
  async me() {
    return (await centerRequest<{ user: CenterUser }>("GET", "/client/v1/me")).user;
  },
  async groups() {
    return (await centerRequest<{ groups: CenterGroup[] }>("GET", "/groups")).groups;
  },
  group: (groupId: number) => centerRequest<CenterGroupDetail>("GET", `/groups/${groupId}`),
  createGroup: (name: string) => centerRequest<CenterGroup>("POST", "/groups", { name }),
  searchUsers: (query: string) => centerRequest<{ users: CenterUser[] }>("GET", `/users/search?q=${encodeURIComponent(query)}`),
  addGroupMember: (groupId: number, userId: number) => centerRequest<{ ok: true }>("POST", `/groups/${groupId}/members`, { user_id: userId }),
  removeGroupMember: (groupId: number, userId: number) => centerRequest<{ ok: true }>("DELETE", `/groups/${groupId}/members/${userId}`),
  async shareLocalProvider(groupId: number, localProviderId: string) {
    const payload = await gatewayRequest<{ provider_id: string }>("POST", "/control/control-plane/share-provider", { provider_id: localProviderId });
    await centerRequest<{ ok: true }>("POST", `/groups/${groupId}/providers`, { provider_id: payload.provider_id });
    return payload.provider_id;
  },
  unshareProvider: (groupId: number, providerId: string) => centerRequest<{ ok: true }>("DELETE", `/groups/${groupId}/providers/${encodeURIComponent(providerId)}`),
  async users() {
    return (await centerRequest<{ users: ManagedCenterUser[] }>("GET", "/users")).users;
  },
};

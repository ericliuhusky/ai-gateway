import type {
  CodexAuthPayload,
  AutoRoutingSettings,
  CodexClientVersionSetting,
  GatewayCompatibilityProfile,
  GatewayModel,
  GatewayIssue,
  GatewayProvider,
  ModelBenchmarkResult,
  FeishuAppSecretResponse,
  InstanceRoutingConfig,
  ProviderQuotaSummary,
  ReasoningEffort,
  SecuritySettings,
  SelectedProvider,
  TurnRouteLog,
  UsageSummary,
  DailyUsageSummary,
  OpenAiDeviceLoginStart,
  OpenAiDeviceLoginStatus,
  GatewayGroup,
  GatewayGroupDetail,
  GatewayUser,
} from "./types";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    ...init,
    headers: {
      Accept: "application/json",
      ...(init?.body ? { "Content-Type": "application/json" } : {}),
      ...init?.headers,
    },
  });

  if (!response.ok) {
    const text = await response.text();
    let message = text || `HTTP ${response.status}`;
    try {
      const payload = JSON.parse(text) as { error?: string | { message?: string } };
      message =
        typeof payload.error === "string"
          ? payload.error
          : payload.error?.message ?? message;
    } catch {
      // Keep the text response.
    }
    throw new Error(message);
  }

  const contentType = response.headers.get("content-type") ?? "";
  if (!contentType.includes("application/json")) {
    return (await response.text()) as T;
  }
  return (await response.json()) as T;
}

export const gatewayApi = {
  health: () => request<string>("/healthz"),

  codexClientVersion: () =>
    request<CodexClientVersionSetting>("/settings/codex-client-version"),

  setCodexClientVersion(version: string) {
    return request<CodexClientVersionSetting>("/settings/codex-client-version", {
      method: "PUT",
      body: JSON.stringify({ version }),
    });
  },

  clearCodexClientVersion() {
    return request<CodexClientVersionSetting>("/settings/codex-client-version", {
      method: "DELETE",
    });
  },

  securitySettings: () => request<SecuritySettings>("/settings/security"),

  feishuAppSecret: () =>
    request<FeishuAppSecretResponse>("/settings/security/feishu-app-secret"),

  setSecuritySettings(settings: {
    feishu_app_id: string;
    feishu_app_secret?: string;
    auth_required: boolean;
  }) {
    return request<SecuritySettings>("/settings/security", {
      method: "PUT",
      body: JSON.stringify(settings),
    });
  },

  regenerateDatabaseEncryptionKey() {
    return request<SecuritySettings>("/settings/security/encryption-key/regenerate", {
      method: "POST",
    });
  },

  automaticRouting() {
    return request<AutoRoutingSettings>("/settings/automatic-routing");
  },

  setAutomaticRouting(settings: AutoRoutingSettings) {
    return request<AutoRoutingSettings>("/settings/automatic-routing", {
      method: "PUT",
      body: JSON.stringify(settings),
    });
  },

  async instances() {
    const payload = await request<{ instances: InstanceRoutingConfig[] }>("/instances");
    return payload.instances;
  },

  instanceConfig(instanceId: string) {
    return request<InstanceRoutingConfig>(`/instances/${encodeURIComponent(instanceId)}/config`);
  },

  setInstanceConfig(
    instanceId: string,
    config: Omit<InstanceRoutingConfig, "instance_id" | "updated_at">,
  ) {
    return request<InstanceRoutingConfig>(`/instances/${encodeURIComponent(instanceId)}/config`, {
      method: "PUT",
      body: JSON.stringify(config),
    });
  },

  deleteInstance(instanceId: string) {
    return request<{ deleted_instance: string }>(`/instances/${encodeURIComponent(instanceId)}`, {
      method: "DELETE",
    });
  },

  async routingTurns(limit = 50) {
    const payload = await request<{ turns: TurnRouteLog[] }>(`/routing/turns?limit=${limit}`);
    return payload.turns;
  },

  async gatewayIssues(limit = 200) {
    const payload = await request<{ issues: GatewayIssue[] }>(`/gateway/issues?limit=${limit}`);
    return payload.issues;
  },

  gatewayIssueRepairPrompt(issueId: string) {
    return request<{ prompt: string }>(
      `/gateway/issues/${encodeURIComponent(issueId)}/repair-prompt`,
    );
  },

  clearGatewayIssues() {
    return request<{ deleted: number }>("/gateway/issues", { method: "DELETE" });
  },

  usageSummary(period: "total" | "today" | "week", providerId?: string) {
    const query = new URLSearchParams({ period });
    if (providerId) query.set("provider_id", providerId);
    return request<UsageSummary[]>(`/usage/summary?${query.toString()}`);
  },

  usageDaily(days = 30) {
    return request<DailyUsageSummary[]>(`/usage/daily?days=${days}`);
  },

  async providers() {
    const payload = await request<{ providers: GatewayProvider[] }>("/providers");
    return payload.providers;
  },

  async groups() {
    const payload = await request<{ groups: GatewayGroup[] }>("/groups");
    return payload.groups;
  },

  group(groupId: number) {
    return request<GatewayGroupDetail>(`/groups/${groupId}`);
  },

  createGroup(name: string) {
    return request<GatewayGroup>("/groups", {
      method: "POST",
      body: JSON.stringify({ name }),
    });
  },

  searchUsers(query: string) {
    return request<{ users: GatewayUser[] }>(`/users/search?q=${encodeURIComponent(query)}`);
  },

  addGroupMember(groupId: number, userId: number) {
    return request<{ ok: true }>(`/groups/${groupId}/members`, {
      method: "POST",
      body: JSON.stringify({ user_id: userId }),
    });
  },

  removeGroupMember(groupId: number, userId: number) {
    return request<{ ok: true }>(`/groups/${groupId}/members/${userId}`, { method: "DELETE" });
  },

  shareGroupProvider(groupId: number, providerId: string) {
    return request<{ ok: true }>(`/groups/${groupId}/providers`, {
      method: "POST",
      body: JSON.stringify({ provider_id: providerId }),
    });
  },

  unshareGroupProvider(groupId: number, providerId: string) {
    return request<{ ok: true }>(`/groups/${groupId}/providers/${encodeURIComponent(providerId)}`, {
      method: "DELETE",
    });
  },

  async selectedProvider() {
    const payload = await request<{ selected_provider: SelectedProvider }>("/selected-provider");
    return payload.selected_provider;
  },

  async selectProvider(providerId: string) {
    const payload = await request<{ selected_provider: SelectedProvider }>("/selected-provider", {
      method: "PUT",
      body: JSON.stringify({ provider_id: providerId }),
    });
    return payload.selected_provider;
  },

  async models(providerId?: string, force = false) {
    const query = new URLSearchParams();
    if (providerId) query.set("provider_id", providerId);
    if (force) query.set("force", "true");
    const payload = await request<{ data: GatewayModel[] }>(
      `/openai/v1/models${query.size ? `?${query.toString()}` : ""}`,
    );
    return payload.data;
  },

  async selectModel(model: string) {
    const payload = await request<{ selected_model: SelectedProvider }>("/selected-model", {
      method: "PUT",
      body: JSON.stringify({ model }),
    });
    return payload.selected_model;
  },

  async clearSelectedModel() {
    const payload = await request<{ selected_model: SelectedProvider }>("/selected-model", {
      method: "DELETE",
    });
    return payload.selected_model;
  },

  async selectReasoningEffort(effort: ReasoningEffort) {
    const payload = await request<{ selected_reasoning_effort: SelectedProvider }>(
      "/selected-reasoning-effort",
      {
        method: "PUT",
        body: JSON.stringify({ effort }),
      },
    );
    return payload.selected_reasoning_effort;
  },

  async clearSelectedReasoningEffort() {
    const payload = await request<{ selected_reasoning_effort: SelectedProvider }>(
      "/selected-reasoning-effort",
      { method: "DELETE" },
    );
    return payload.selected_reasoning_effort;
  },

  createProvider(input: {
    name: string;
    base_url: string;
    api_key: string;
    compatibility_profile: GatewayCompatibilityProfile;
  }) {
    return request("/providers", { method: "POST", body: JSON.stringify(input) });
  },

  deleteProvider(providerId: string) {
    return request(`/providers/${encodeURIComponent(providerId)}`, { method: "DELETE" });
  },

  async quota(providerId: string) {
    const payload = await request<{ quota: ProviderQuotaSummary }>(
      `/providers/${encodeURIComponent(providerId)}/quota`,
    );
    return payload.quota;
  },

  benchmarkModel(input: {
    provider_id: string;
    model: string;
    runs?: number;
    account_usage_confirmed?: boolean;
  }) {
    return request<ModelBenchmarkResult>("/benchmarks/models", {
      method: "POST",
      body: JSON.stringify(input),
    });
  },

  importAccount(payload: CodexAuthPayload) {
    return request("/accounts/openai/import-token", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  startOpenAiDeviceLogin() {
    return request<OpenAiDeviceLoginStart>("/accounts/openai/login/device", {
      method: "POST",
    });
  },

  pollOpenAiDeviceLogin(loginId: string) {
    return request<OpenAiDeviceLoginStatus>(
      `/accounts/openai/login/device/${encodeURIComponent(loginId)}`,
    );
  },

  cancelOpenAiDeviceLogin(loginId: string) {
    return request<{ cancelled: boolean }>(
      `/accounts/openai/login/device/${encodeURIComponent(loginId)}`,
      { method: "DELETE" },
    );
  },
};


export const authApi = {
  status: () => request<{ mode: "disabled" | "required"; feishu_login_configured: boolean }>("/auth/status"),
  me: () => request<{ ok: true; user: GatewayUser }>("/auth/me"),
  logout: () => request<{ ok: true }>("/auth/logout", { method: "POST" }),
  gatewayAccessToken: () => request<{ access_token: string }>("/auth/access-tokens"),
  regenerateAccessToken: () => request<{ access_token: string }>("/auth/access-tokens", { method: "POST" }),
};

export type { GatewayUser } from "./types";

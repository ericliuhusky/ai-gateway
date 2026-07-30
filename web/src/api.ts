import type {
  CodexAuthPayload,
  AutoRoutingSettings,
  CodexClientVersionSetting,
  GatewayCompatibilityProfile,
  GatewayModel,
  GatewayProvider,
  InstanceRoutingConfig,
  ProviderQuotaSummary,
  ReasoningEffort,
  SelectedProvider,
  TurnRouteLog,
} from "./types";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
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
    const payload = await request<{ instances: string[] }>("/instances");
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

  async routingTurns(limit = 50) {
    const payload = await request<{ turns: TurnRouteLog[] }>(`/routing/turns?limit=${limit}`);
    return payload.turns;
  },

  async providers() {
    const payload = await request<{ providers: GatewayProvider[] }>("/providers");
    return payload.providers;
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

  importAccount(payload: CodexAuthPayload) {
    return request("/accounts/openai/import-token", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },
};

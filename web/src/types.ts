export type GatewayAuthMode = "api_key" | "account";
export type GatewayUpstreamProtocol = "openai_responses";
export type GatewayCompatibilityProfile =
  | "official_openai"
  | "generic_openai"
  | "openai_codex";

export interface GatewayProvider {
  id: string;
  name: string;
  auth_mode: GatewayAuthMode;
  base_url: string;
  account_id?: string;
  account_email?: string;
  upstream_protocol: GatewayUpstreamProtocol;
  compatibility_profile: GatewayCompatibilityProfile;
}

export interface SelectedProvider {
  provider_id?: string;
  selected_model?: string;
  selected_reasoning_effort?: ReasoningEffort;
  updated_at: number;
}

export type ReasoningEffort = "low" | "medium" | "high" | "xhigh";

export interface GatewayModel {
  id: string;
}

export interface ModelBenchmarkSample {
  ttft_ms: number;
  total_ms: number;
  output_text: string;
  output_tokens?: number;
  generation_tokens_per_second?: number;
}

export interface ModelBenchmarkResult {
  provider_id: string;
  model: string;
  prompt: string;
  samples: ModelBenchmarkSample[];
  median_ttft_ms: number;
  median_total_ms: number;
  median_generation_tokens_per_second?: number;
}

export interface RoutingModelTarget {
  provider_id: string;
  model: string;
  reasoning_effort?: ReasoningEffort;
}

export interface CodexClientVersionSetting {
  default_version: string;
  override_version?: string;
  effective_version: string;
  is_overridden: boolean;
}

export interface SecuritySettings {
  encryption_key_configured: boolean;
  feishu_app_id: string;
  feishu_app_secret_configured: boolean;
  auth_required: boolean;
}

export interface FeishuAppSecretResponse {
  feishu_app_secret: string;
}

export interface InstanceRoutingConfig {
  instance_id: string;
  provider_id?: string;
  selected_model?: string;
  selected_reasoning_effort?: ReasoningEffort;
  updated_at: number;
  automatic_routing: AutoRoutingSettings;
}

export interface AutoRoutingSettings {
  enabled: boolean;
  light?: RoutingModelTarget;
  standard?: RoutingModelTarget;
  pro?: RoutingModelTarget;
  max?: RoutingModelTarget;
}

export interface TurnRouteLog {
  turn_id: string;
  provider_id: string;
  model: string;
  routing_mode: string;
  routing_reason: string;
  routing_detail?: string;
  routing_tier?: "low" | "medium" | "high" | "xhigh";
  classifier_confidence?: number;
  classifier_output?: string;
  classifier_raw_input?: string;
  classifier_raw_output?: string;
  reasoning_effort?: string;
  user_input_preview?: string;
  started_at: number;
  updated_at: number;
  request_count: number;
  tool_round_count: number;
}

export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  cached_input_tokens: number;
  reasoning_tokens: number;
  total_tokens: number;
}

export interface UsageSummary {
  provider_id: string;
  model?: string;
  request_count: number;
  input_tokens: number;
  output_tokens: number;
  cached_input_tokens: number;
  reasoning_tokens: number;
  total_tokens: number;
}

export interface DailyUsageSummary {
  date: string;
  provider_id: string;
  model: string;
  request_count: number;
  total_tokens: number;
}

export interface ProviderQuotaWindow {
  used_percent: number;
  window_minutes?: number;
  resets_at?: number;
}

export interface ProviderQuotaCredits {
  has_credits: boolean;
  unlimited: boolean;
  balance?: string;
}

export interface ProviderQuotaSnapshot {
  limit_id?: string;
  limit_name?: string;
  primary?: ProviderQuotaWindow;
  secondary?: ProviderQuotaWindow;
  credits?: ProviderQuotaCredits;
  plan_type?: string;
}

export interface ProviderQuotaSummary {
  status: "supported" | "unsupported";
  snapshot?: ProviderQuotaSnapshot;
  additional_snapshots?: ProviderQuotaSnapshot[];
  message?: string;
}

export interface OfficialCodexAuthPayload {
  tokens: {
    id_token?: string;
    access_token: string;
    refresh_token: string;
    account_id?: string;
  };
}

export interface CockpitToolsCodexToken {
  id_token?: string;
  access_token: string;
  refresh_token: string;
  account_id?: string;
  type?: string;
  [key: string]: unknown;
}

export type CodexAuthPayload =
  | OfficialCodexAuthPayload
  | CockpitToolsCodexToken
  | CockpitToolsCodexToken[];

export interface OpenAiDeviceLoginStart {
  login_id: string;
  user_code: string;
  verification_uri: string;
  interval_seconds: number;
  expires_in: number;
}

export interface OpenAiDeviceLoginStatus {
  status: "pending" | "finalizing" | "completed" | "failed";
  login_id?: string;
  user_code?: string;
  verification_uri?: string;
  interval_seconds?: number;
  expires_in?: number;
  email?: string;
  account_id?: string;
  has_responses_write?: boolean;
  error?: string;
}

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

export interface RoutingModelTarget {
  provider_id: string;
  model: string;
}

export interface CodexClientVersionSetting {
  default_version: string;
  override_version?: string;
  effective_version: string;
  is_overridden: boolean;
}

export interface AutoRoutingSettings {
  enabled: boolean;
  classifier?: RoutingModelTarget;
  light?: RoutingModelTarget;
  standard?: RoutingModelTarget;
  pro?: RoutingModelTarget;
  max?: RoutingModelTarget;
  low_confidence_threshold: number;
}

export interface TurnRouteLog {
  turn_id: string;
  provider_id: string;
  model: string;
  routing_mode: string;
  routing_reason: string;
  routing_detail?: string;
  routing_tier?: "light" | "standard" | "pro" | "max";
  classifier_confidence?: number;
  classifier_output?: string;
  reasoning_effort?: string;
  user_input_preview?: string;
  started_at: number;
  updated_at: number;
  request_count: number;
  tool_round_count: number;
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

export interface CodexAuthPayload {
  tokens: {
    id_token?: string;
    access_token: string;
    refresh_token: string;
    account_id?: string;
  };
}

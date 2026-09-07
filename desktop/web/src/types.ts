export type GatewayAuthMode = "api_key" | "account";

export interface GatewayProvider {
  id: string;
  name: string;
  auth_mode: GatewayAuthMode;
  base_url: string;
  account_id?: string;
  account_email?: string;
  account_expires_at?: number;
}

export interface DefaultCodexStatus {
  started: boolean;
}

export interface CodexConfigurationResult {
  changed: boolean;
  warnings: string[];
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

export interface GatewayIssue {
  id: string;
  provider_id: string;
  provider_name: string;
  model: string;
  upstream_url: string;
  failure_kind: string;
  status_code?: number;
  error_message: string;
  upstream_response: string;
  upstream_response_truncated: boolean;
  created_at: number;
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
    access_token: string;
    refresh_token: string;
    account_id?: string;
  };
}

export interface CockpitToolsCodexToken {
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
  error?: string;
}

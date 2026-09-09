export type GatewayAuthMode = "api_key" | "account";

export type GatewayProvider = {
  id: string;
  name: string;
} & (
  | {
      auth_mode: "api_key";
      base_url: string;
    }
  | {
      auth_mode: "account";
      account_email: string;
      account_expires_at: number;
    }
);

export interface DefaultCodexStatus {
  started: boolean;
}

export interface CodexConfigurationResult {
  changed: boolean;
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

export interface CodexUsageRateLimitWindow {
  used_percent: number;
  limit_window_seconds: number;
  reset_after_seconds: number;
  reset_at: number;
}

export interface CodexUsageRateLimit {
  allowed: boolean;
  limit_reached: boolean;
  primary_window?: CodexUsageRateLimitWindow;
  secondary_window?: CodexUsageRateLimitWindow;
}

export interface CodexUsageCredits {
  has_credits: boolean;
  unlimited: boolean;
  balance?: string | null;
}

export interface CodexUsageAdditionalRateLimit {
  limit_name: string;
  metered_feature: string;
  rate_limit?: CodexUsageRateLimit;
}

export interface CodexUsageResponse {
  plan_type: string;
  rate_limit?: CodexUsageRateLimit;
  credits?: CodexUsageCredits;
  additional_rate_limits?: CodexUsageAdditionalRateLimit[];
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
  status: "pending" | "finalizing" | "conflict" | "completed" | "failed";
  login_id?: string;
  user_code?: string;
  verification_uri?: string;
  interval_seconds?: number;
  expires_in?: number;
  email?: string;
  provider_id?: string;
  error?: string;
}

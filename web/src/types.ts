export type GatewayAuthMode = "api_key" | "account";
export type GatewayBillingMode = "metered" | "subscription";

export interface GatewayProvider {
  id: string;
  name: string;
  auth_mode: GatewayAuthMode;
  base_url: string;
  account_id?: string;
  account_email?: string;
  billing_mode: GatewayBillingMode;
  uses_chat_completions: boolean;
}

export interface SelectedProvider {
  provider_id?: string;
  selected_model?: string;
  updated_at: number;
}

export interface GatewayModel {
  id: string;
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

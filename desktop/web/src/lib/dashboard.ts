import type {
  CodexAuthPayload,
  CodexUsageRateLimitWindow,
  CodexUsageResponse,
  GatewayModel,
} from "../types";

export const GATEWAY_ERROR_PREFIX = "AI网关错误：";
export const UPSTREAM_ERROR_PREFIX = "上游服务错误：";
export const MODEL_CACHE_STORAGE_KEY = "ai-gateway:model-cache:v1";
export const QUOTA_CACHE_STORAGE_KEY = "ai-gateway:quota-cache:v1";
export const MODEL_CACHE_MAX_AGE_MS = 24 * 60 * 60 * 1000;

export type Dialog = "provider" | "delete-provider" | null;
export type QuotaMap = Record<string, CodexUsageResponse | undefined>;
export type ErrorMap = Record<string, string | undefined>;
export type ModelCache = Record<string, { models: GatewayModel[]; fetchedAt: number }>;
export type QuotaCache = Record<string, { quota: CodexUsageResponse; fetchedAt: number }>;

export const NINEBOT_PRIVATE_DEPLOYMENT_PRESET = {
  name: "九号私有部署",
  baseUrl: "https://ai-service.segway-ninebot.com/v1",
} as const;

export function readLocalCache<T>(key: string, fallback: T): T {
  try {
    const raw = window.localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

export function writeLocalCache<T>(key: string, value: T): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Local storage is an optional cache; a full or unavailable store should
    // never prevent the dashboard from working.
  }
}

export function quotasFromCache(cache: QuotaCache): QuotaMap {
  return Object.fromEntries(Object.entries(cache).map(([id, entry]) => [id, entry.quota]));
}

export function errorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.startsWith(GATEWAY_ERROR_PREFIX) || message.startsWith(UPSTREAM_ERROR_PREFIX)
    ? message
    : `${GATEWAY_ERROR_PREFIX}${message}`;
}

export function duplicateAccountEmail(error: unknown): string | null {
  const message = error instanceof Error ? error.message : String(error);
  return message.match(/OpenAI 账号已经存在[:：]\s*(.+)$/)?.[1]?.trim() ?? null;
}

function hasTokenPair(value: unknown): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const token = value as Record<string, unknown>;
  return (
    typeof token.access_token === "string" &&
    token.access_token.trim().length > 0 &&
    typeof token.refresh_token === "string" &&
    token.refresh_token.trim().length > 0
  );
}

export function parseCodexAuthPayload(value: unknown): CodexAuthPayload | null {
  const entries = Array.isArray(value) ? value : [value];
  if (!entries.length) return null;
  const supported = entries.every((entry) => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) return false;
    const record = entry as Record<string, unknown>;
    return hasTokenPair(record.tokens) || hasTokenPair(record);
  });
  return supported ? (value as CodexAuthPayload) : null;
}

export function remaining(window: CodexUsageRateLimitWindow): number {
  return Math.min(100, Math.max(0, 100 - window.used_percent));
}

export function quotaTone(value: number): "danger" | "warning" | "good" {
  return value <= 15 ? "danger" : value <= 35 ? "warning" : "good";
}

export function resetLabel(window: CodexUsageRateLimitWindow): string | null {
  if (!window.reset_at) return null;
  const date = new Date(window.reset_at * 1000);
  const time = `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
  if (window.limit_window_seconds === 5 * 60 * 60) return `${time} 重置`;
  const weekdays = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
  return `${date.getMonth() + 1}月${date.getDate()}日 ${weekdays[date.getDay()]} ${time} 重置`;
}

export function authExpiryLabel(timestamp?: number): string {
  if (!timestamp) return "未知";
  const date = new Date(timestamp * 1000);
  const expired = timestamp * 1000 <= Date.now();
  const weekdays = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
  const time = `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
  return `${date.getMonth() + 1}月${date.getDate()}日 ${weekdays[date.getDay()]} ${time} ${expired ? "已过期" : "到期"}`;
}

export function copyText(text: string): Promise<void> {
  return navigator.clipboard.writeText(text);
}

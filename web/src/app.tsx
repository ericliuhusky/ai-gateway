import * as React from "react";
import {
  Activity,
  Check,
  CheckCircle2,
  ChevronDown,
  CircleAlert,
  Cloud,
  Code2,
  Copy,
  Gauge,
  KeyRound,
  LoaderCircle,
  Plus,
  RefreshCw,
  Server,
  Trash2,
  UserRound,
  X,
} from "lucide-react";

import { gatewayApi } from "./api";
import { Button } from "./components/ui/button";
import { cn } from "./lib/utils";
import type {
  CodexAuthPayload,
  GatewayBillingMode,
  GatewayModel,
  GatewayProvider,
  ProviderQuotaSummary,
  ProviderQuotaWindow,
  SelectedProvider,
  GatewayCompatibilityProfile,
  GatewayUpstreamProtocol,
} from "./types";

type Dialog = "api" | "account" | "setup" | null;
type QuotaMap = Record<string, ProviderQuotaSummary | undefined>;
type ErrorMap = Record<string, string | undefined>;

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function suggestedCompatibilityProfile(
  baseUrl: string,
): GatewayCompatibilityProfile {
  try {
    return new URL(baseUrl).hostname.toLowerCase() === "api.openai.com"
      ? "official_openai"
      : "generic_openai";
  } catch {
    return "generic_openai";
  }
}

function remaining(window: ProviderQuotaWindow) {
  return Math.min(100, Math.max(0, 100 - window.used_percent));
}

function quotaTone(value: number) {
  if (value <= 15) return "danger";
  if (value <= 35) return "warning";
  return "good";
}

function resetLabel(window: ProviderQuotaWindow) {
  if (!window.resets_at) return null;
  const date = new Date(window.resets_at * 1000);
  const time = `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;

  if (window.window_minutes === 300) {
    return `${time} 重置`;
  }

  const weekdays = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
  return `${date.getMonth() + 1}月${date.getDate()}日 ${weekdays[date.getDay()]} ${time} 重置`;
}

function copyText(text: string) {
  return navigator.clipboard.writeText(text);
}

function shellQuote(text: string) {
  return `'${text.replace(/'/g, `'\"'\"'`)}'`;
}

export function App() {
  const [providers, setProviders] = React.useState<GatewayProvider[]>([]);
  const [selected, setSelected] = React.useState<SelectedProvider>({ updated_at: 0 });
  const [models, setModels] = React.useState<GatewayModel[]>([]);
  const [quotas, setQuotas] = React.useState<QuotaMap>({});
  const [quotaErrors, setQuotaErrors] = React.useState<ErrorMap>({});
  const [loadingQuotas, setLoadingQuotas] = React.useState<Set<string>>(new Set());
  const [loading, setLoading] = React.useState(true);
  const [loadingModels, setLoadingModels] = React.useState(false);
  const [serverOnline, setServerOnline] = React.useState(false);
  const [dialog, setDialog] = React.useState<Dialog>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [deleting, setDeleting] = React.useState<Set<string>>(new Set());

  const selectedProvider = providers.find((provider) => provider.id === selected.provider_id);
  const accountProviders = providers.filter((provider) => provider.auth_mode === "account");
  const apiProviders = providers.filter((provider) => provider.auth_mode === "api_key");

  const loadQuotas = React.useCallback(async (items: GatewayProvider[], visibleLoading = true) => {
    const ids = items.filter((item) => item.auth_mode === "account").map((item) => item.id);
    if (!ids.length) return;

    if (visibleLoading) {
      setLoadingQuotas((current) => new Set([...current, ...ids]));
    }
    await Promise.all(
      ids.map(async (id) => {
        try {
          const quota = await gatewayApi.quota(id);
          setQuotas((current) => ({ ...current, [id]: quota }));
          setQuotaErrors((current) => ({ ...current, [id]: undefined }));
        } catch (quotaError) {
          setQuotaErrors((current) => ({ ...current, [id]: errorMessage(quotaError) }));
        } finally {
          setLoadingQuotas((current) => {
            const next = new Set(current);
            next.delete(id);
            return next;
          });
        }
      }),
    );
  }, []);

  const loadModels = React.useCallback(async (force = false) => {
    setLoadingModels(true);
    try {
      const data = await gatewayApi.models(force);
      setModels([...data].sort((a, b) => a.id.localeCompare(b.id)));
    } catch (modelError) {
      setModels([]);
      setError(errorMessage(modelError));
    } finally {
      setLoadingModels(false);
    }
  }, []);

  const refresh = React.useCallback(async () => {
    setLoading(true);
    try {
      const [health, providerList, route] = await Promise.all([
        gatewayApi.health(),
        gatewayApi.providers(),
        gatewayApi.selectedProvider(),
      ]);
      setServerOnline(health.trim() === "ok");
      const sorted = [...providerList].sort((a, b) => a.name.localeCompare(b.name));
      setProviders(sorted);
      setSelected(route);
      setError(null);
      if (route.provider_id) {
        void loadModels();
      } else {
        setModels([]);
      }
      void loadQuotas(sorted);
    } catch (loadError) {
      setServerOnline(false);
      setError(errorMessage(loadError));
    } finally {
      setLoading(false);
    }
  }, [loadModels, loadQuotas]);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  React.useEffect(() => {
    const timer = window.setInterval(() => {
      void loadQuotas(providers, false);
    }, 60_000);
    return () => window.clearInterval(timer);
  }, [loadQuotas, providers]);

  async function selectProvider(provider: GatewayProvider) {
    if (provider.id === selected.provider_id || deleting.has(provider.id)) return;
    setSelected((current) => ({ ...current, provider_id: provider.id, selected_model: undefined }));
    setModels([]);
    try {
      const route = await gatewayApi.selectProvider(provider.id);
      setSelected(route);
      await Promise.all([loadModels(), loadQuotas([provider])]);
    } catch (selectionError) {
      setError(errorMessage(selectionError));
      await refresh();
    }
  }

  async function selectModel(model: string) {
    setLoadingModels(true);
    try {
      const route = model
        ? await gatewayApi.selectModel(model)
        : await gatewayApi.clearSelectedModel();
      setSelected(route);
    } catch (modelError) {
      setError(errorMessage(modelError));
    } finally {
      setLoadingModels(false);
    }
  }

  async function deleteProvider(provider: GatewayProvider) {
    if (!window.confirm(`确定要删除供应商“${provider.name}”吗？`)) return;
    setDeleting((current) => new Set(current).add(provider.id));
    try {
      await gatewayApi.deleteProvider(provider.id);
      await refresh();
    } catch (deleteError) {
      setError(errorMessage(deleteError));
    } finally {
      setDeleting((current) => {
        const next = new Set(current);
        next.delete(provider.id);
        return next;
      });
    }
  }

  async function refreshQuota(provider: GatewayProvider) {
    await loadQuotas([provider]);
  }

  return (
    <div className="min-h-screen">
      <header className="border-b border-white/50 bg-white/55 backdrop-blur-xl dark:border-white/8 dark:bg-slate-950/55">
        <div className="mx-auto flex h-16 max-w-[1480px] items-center gap-3 px-5 sm:px-8">
          <div className="flex size-9 items-center justify-center rounded-xl bg-slate-900 text-white shadow-lg shadow-slate-900/15 dark:bg-white dark:text-slate-950">
            <Cloud className="size-[18px]" />
          </div>
          <div>
            <div className="text-[15px] font-bold tracking-[-0.02em]">AI Gateway</div>
            <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-slate-400">
              Remote Console
            </div>
          </div>
          <div className="ml-auto flex items-center gap-2">
            <StatusPill online={serverOnline} />
            <Button
              variant="outline"
              size="sm"
              aria-label="Codex 接入"
              onClick={() => setDialog("setup")}
            >
              <Code2 className="size-3.5" />
              <span className="hidden sm:inline">Codex 接入</span>
            </Button>
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-[1480px] px-5 py-6 sm:px-8 sm:py-8">
        <section className="glass-panel mb-8 flex flex-col gap-5 rounded-[22px] p-4 lg:flex-row lg:items-center">
          <div className="flex min-w-0 flex-1 items-center gap-3">
            <div className="flex size-10 shrink-0 items-center justify-center rounded-2xl bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">
              <Server className="size-[18px]" />
            </div>
            <div className="min-w-0">
              <div className="eyebrow">远程 Server</div>
              <div className="truncate text-[13px] font-semibold">{window.location.origin}</div>
            </div>
          </div>

          <div className="hidden h-8 w-px bg-slate-200/70 lg:block dark:bg-white/10" />

          <div className="flex min-w-0 flex-1 items-center gap-3">
            <span className="eyebrow shrink-0">供应商</span>
            <span
              className={cn(
                "truncate rounded-full px-3 py-1.5 text-xs font-bold",
                selectedProvider
                  ? "bg-blue-500/10 text-blue-600 dark:text-blue-400"
                  : "bg-slate-500/10 text-slate-500",
              )}
            >
              {selectedProvider?.name ?? "未选择"}
            </span>
            <div className="ml-auto flex gap-2">
              <Button variant="outline" size="icon" title="添加 API Key 供应商" onClick={() => setDialog("api")}>
                <KeyRound className="size-4" />
              </Button>
              <Button variant="outline" size="icon" title="导入账户 Token" onClick={() => setDialog("account")}>
                <UserRound className="size-4" />
              </Button>
            </div>
          </div>

          <div className="hidden h-8 w-px bg-slate-200/70 lg:block dark:bg-white/10" />

          <div className="flex min-w-0 flex-1 items-center gap-3">
            <span className="eyebrow shrink-0">模型</span>
            <div className="relative min-w-0 flex-1">
              <select
                className="field h-9 w-full appearance-none pr-9 text-xs font-semibold"
                value={selected.selected_model ?? ""}
                disabled={!selected.provider_id || loadingModels}
                onChange={(event) => void selectModel(event.target.value)}
              >
                <option value="">跟随请求模型</option>
                {models.map((model) => (
                  <option key={model.id} value={model.id}>
                    {model.id}
                  </option>
                ))}
              </select>
              <ChevronDown className="pointer-events-none absolute right-3 top-1/2 size-3.5 -translate-y-1/2 text-slate-400" />
            </div>
            <Button
              variant="outline"
              size="icon"
              title="刷新模型"
              disabled={!selected.provider_id || loadingModels}
              onClick={() => void loadModels(true)}
            >
              <RefreshCw className={cn("size-4", loadingModels && "animate-spin")} />
            </Button>
          </div>
        </section>

        {loading ? (
          <LoadingState />
        ) : providers.length === 0 ? (
          <EmptyState onAddApi={() => setDialog("api")} onAddAccount={() => setDialog("account")} />
        ) : (
          <div className="space-y-9">
            <ProviderSection
              title="账户供应商"
              providers={accountProviders}
              selectedId={selected.provider_id}
              quotas={quotas}
              quotaErrors={quotaErrors}
              loadingQuotas={loadingQuotas}
              deleting={deleting}
              onSelect={selectProvider}
              onDelete={deleteProvider}
              onRefreshQuota={refreshQuota}
            />
            <ProviderSection
              title="API Key 供应商"
              providers={apiProviders}
              selectedId={selected.provider_id}
              quotas={quotas}
              quotaErrors={quotaErrors}
              loadingQuotas={loadingQuotas}
              deleting={deleting}
              onSelect={selectProvider}
              onDelete={deleteProvider}
              onRefreshQuota={refreshQuota}
            />
          </div>
        )}
      </main>

      {error ? <ErrorToast message={error} onClose={() => setError(null)} /> : null}
      {dialog === "api" ? (
        <ApiProviderDialog
          onClose={() => setDialog(null)}
          onCreated={async () => {
            setDialog(null);
            await refresh();
          }}
          onError={setError}
        />
      ) : null}
      {dialog === "account" ? (
        <AccountDialog
          onClose={() => setDialog(null)}
          onCreated={async () => {
            setDialog(null);
            await refresh();
          }}
          onError={setError}
        />
      ) : null}
      {dialog === "setup" ? <SetupDialog onClose={() => setDialog(null)} /> : null}
    </div>
  );
}

function StatusPill({ online }: { online: boolean }) {
  return (
    <div
      className={cn(
        "flex h-8 items-center gap-2 rounded-full px-3 text-xs font-bold",
        online
          ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
          : "bg-red-500/10 text-red-600 dark:text-red-400",
      )}
    >
      <span className={cn("size-1.5 rounded-full", online ? "bg-emerald-500" : "bg-red-500")} />
      {online ? "Server 在线" : "连接失败"}
    </div>
  );
}

function ProviderSection(props: {
  title: string;
  providers: GatewayProvider[];
  selectedId?: string;
  quotas: QuotaMap;
  quotaErrors: ErrorMap;
  loadingQuotas: Set<string>;
  deleting: Set<string>;
  onSelect: (provider: GatewayProvider) => void;
  onDelete: (provider: GatewayProvider) => void;
  onRefreshQuota: (provider: GatewayProvider) => void;
}) {
  if (!props.providers.length) return null;
  return (
    <section>
      <div className="mb-3 flex items-center gap-3 px-1">
        <h2 className="text-xs font-bold uppercase tracking-[0.12em] text-slate-500 dark:text-slate-400">
          {props.title}
        </h2>
        <span className="rounded-full bg-white/60 px-2 py-0.5 text-[10px] font-bold text-slate-400 dark:bg-white/5">
          {props.providers.length}
        </span>
      </div>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
        {props.providers.map((provider) => (
          <ProviderCard
            key={provider.id}
            provider={provider}
            selected={provider.id === props.selectedId}
            quota={props.quotas[provider.id]}
            quotaError={props.quotaErrors[provider.id]}
            loadingQuota={props.loadingQuotas.has(provider.id)}
            deleting={props.deleting.has(provider.id)}
            onSelect={() => props.onSelect(provider)}
            onDelete={() => props.onDelete(provider)}
            onRefreshQuota={() => props.onRefreshQuota(provider)}
          />
        ))}
      </div>
    </section>
  );
}

function ProviderCard({
  provider,
  selected,
  quota,
  quotaError,
  loadingQuota,
  deleting,
  onSelect,
  onDelete,
  onRefreshQuota,
}: {
  provider: GatewayProvider;
  selected: boolean;
  quota?: ProviderQuotaSummary;
  quotaError?: string;
  loadingQuota: boolean;
  deleting: boolean;
  onSelect: () => void;
  onDelete: () => void;
  onRefreshQuota: () => void;
}) {
  return (
    <article
      className={cn(
        "provider-card group relative flex min-h-[212px] cursor-pointer flex-col rounded-[24px] p-5",
        selected && "selected",
        deleting && "pointer-events-none opacity-60",
      )}
      onClick={onSelect}
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-lg font-bold tracking-[-0.025em]">{provider.name}</h3>
          <div className="mt-2 flex flex-wrap gap-1.5">
            <Badge tone={provider.auth_mode === "account" ? "green" : "blue"}>
              {provider.auth_mode === "account" ? "账户" : "API Key"}
            </Badge>
            <Badge tone={provider.billing_mode === "subscription" ? "purple" : "amber"}>
              {provider.billing_mode === "subscription" ? "订阅" : "按量"}
            </Badge>
            <Badge tone="slate">
              {provider.upstream_protocol === "openai_chat_completions"
                ? "Chat Completions"
                : "Responses"}
            </Badge>
            <Badge tone="slate">
              {provider.compatibility_profile === "official_openai"
                ? "OpenAI 官方"
                : provider.compatibility_profile === "openai_codex"
                  ? "Codex 账户"
                  : "兼容接口"}
            </Badge>
          </div>
        </div>
        {selected ? (
          <CheckCircle2 className="size-5 shrink-0 text-blue-500" />
        ) : deleting ? (
          <LoaderCircle className="size-5 animate-spin text-slate-400" />
        ) : (
          <button
            className="flex size-8 shrink-0 items-center justify-center rounded-xl text-slate-400 opacity-0 transition hover:bg-black/5 hover:text-red-500 group-hover:opacity-100 dark:hover:bg-white/8"
            type="button"
            title="删除供应商"
            onClick={(event) => {
              event.stopPropagation();
              onDelete();
            }}
          >
            <Trash2 className="size-3.5" />
          </button>
        )}
      </div>

      {provider.auth_mode === "account" ? (
        <div className="mt-4">
          <div className="eyebrow">邮箱</div>
          <div className="mt-1 truncate font-mono text-xs font-semibold">
            {provider.account_email ?? "等待 Token 导入"}
          </div>
        </div>
      ) : (
        <div className="mt-auto pt-7">
          <div className="eyebrow">Base URL</div>
          <div className="mt-1.5 truncate font-mono text-[11px] text-slate-500 dark:text-slate-400">
            {provider.base_url}
          </div>
        </div>
      )}

      {provider.auth_mode === "account" ? (
        <QuotaPanel
          quota={quota}
          error={quotaError}
          loading={loadingQuota}
          onRefresh={onRefreshQuota}
        />
      ) : null}
    </article>
  );
}

function QuotaPanel({
  quota,
  error,
  loading,
  onRefresh,
}: {
  quota?: ProviderQuotaSummary;
  error?: string;
  loading: boolean;
  onRefresh: () => void;
}) {
  const snapshot = quota?.snapshot;
  const primary = snapshot?.primary;
  const secondary = snapshot?.secondary;

  return (
    <div className="mt-4 rounded-2xl border border-white/65 bg-white/45 p-3 dark:border-white/8 dark:bg-white/[0.035]">
      <div className="mb-2.5 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <Gauge className="size-3.5 text-slate-400" />
          <span className="text-[10px] font-bold uppercase tracking-[0.1em] text-slate-500">额度窗口</span>
        </div>
        <button
          type="button"
          title="刷新额度"
          className="text-slate-400 transition hover:text-slate-700 dark:hover:text-white"
          disabled={loading}
          onClick={(event) => {
            event.stopPropagation();
            onRefresh();
          }}
        >
          <RefreshCw className={cn("size-3.5", loading && "animate-spin")} />
        </button>
      </div>
      {loading && !quota ? (
        <div className="flex h-[68px] items-center justify-center text-xs text-slate-400">同步中…</div>
      ) : error ? (
        <div className="line-clamp-2 text-xs leading-5 text-red-500">{error}</div>
      ) : quota?.status === "unsupported" ? (
        <div className="text-xs leading-5 text-slate-400">{quota.message ?? "暂不支持额度快照"}</div>
      ) : primary || secondary ? (
        <div className="space-y-2.5">
          {primary ? <QuotaRow title={windowTitle(primary, "五小时窗口")} window={primary} /> : null}
          {secondary ? <QuotaRow title={windowTitle(secondary, "周窗口")} window={secondary} /> : null}
          {quota ? <QuotaFootnote quota={quota} /> : null}
        </div>
      ) : (
        <div className="text-xs text-slate-400">还没有拿到额度信息</div>
      )}
    </div>
  );
}

function windowTitle(window: ProviderQuotaWindow, fallback: string) {
  if (!window.window_minutes) return fallback;
  if (window.window_minutes === 300) return "五小时窗口";
  if (window.window_minutes >= 7 * 24 * 60) return "周窗口";
  if (window.window_minutes % (24 * 60) === 0) {
    return `${window.window_minutes / (24 * 60)} 天窗口`;
  }
  if (window.window_minutes % 60 === 0) {
    return `${window.window_minutes / 60} 小时窗口`;
  }
  return `${window.window_minutes} 分钟窗口`;
}

function QuotaRow({ title, window }: { title: string; window: ProviderQuotaWindow }) {
  const value = remaining(window);
  const tone = quotaTone(value);
  return (
    <div>
      <div className="mb-1 flex items-center gap-2 text-[10px] font-semibold">
        <span className="text-slate-500">{title}</span>
        <span
          className={cn(
            "ml-auto font-bold",
            tone === "good" && "text-emerald-600 dark:text-emerald-400",
            tone === "warning" && "text-amber-600 dark:text-amber-400",
            tone === "danger" && "text-red-500",
          )}
        >
          {Math.round(value)}%
        </span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-slate-200/75 dark:bg-white/10">
        <div
          className={cn(
            "h-full rounded-full transition-all",
            tone === "good" && "bg-emerald-500",
            tone === "warning" && "bg-amber-500",
            tone === "danger" && "bg-red-500",
          )}
          style={{ width: `${Math.max(value, 2)}%` }}
        />
      </div>
      {resetLabel(window) ? <div className="mt-1 text-[9px] text-slate-400">{resetLabel(window)}</div> : null}
    </div>
  );
}

function QuotaFootnote({ quota }: { quota: ProviderQuotaSummary }) {
  const snapshots = [
    ...(quota.snapshot ? [quota.snapshot] : []),
    ...(quota.additional_snapshots ?? []),
  ];
  const credits = snapshots.map((snapshot) => snapshot.credits).filter(Boolean);
  const unlimited = credits.some((item) => item?.unlimited);
  const balance = credits.find((item) => item?.balance)?.balance;
  const parts = [
    unlimited ? "账户余额无限" : balance ? `余额 ${balance}` : null,
    quota.snapshot?.plan_type ? `Plan ${quota.snapshot.plan_type}` : null,
  ].filter(Boolean);
  return parts.length ? <div className="text-[9px] text-slate-400">{parts.join(" · ")}</div> : null;
}

function Badge({ children, tone }: { children: React.ReactNode; tone: "green" | "blue" | "purple" | "amber" | "slate" }) {
  return (
    <span
      className={cn(
        "rounded-full px-2.5 py-1 text-[10px] font-bold",
        tone === "green" && "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
        tone === "blue" && "bg-blue-500/10 text-blue-600 dark:text-blue-400",
        tone === "purple" && "bg-violet-500/10 text-violet-600 dark:text-violet-400",
        tone === "amber" && "bg-amber-500/10 text-amber-700 dark:text-amber-400",
        tone === "slate" && "bg-slate-500/10 text-slate-500 dark:text-slate-400",
      )}
    >
      {children}
    </span>
  );
}

function DialogFrame({
  title,
  description,
  children,
  onClose,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
  onClose: () => void;
}) {
  React.useEffect(() => {
    const handler = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/30 p-4 backdrop-blur-sm" onMouseDown={onClose}>
      <div
        className="dialog-panel max-h-[calc(100vh-2rem)] w-full max-w-xl overflow-y-auto rounded-[26px] p-6 sm:p-7"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="mb-6 flex items-start gap-4">
          <div className="min-w-0 flex-1">
            <h2 className="text-xl font-bold tracking-[-0.025em]">{title}</h2>
            <p className="mt-1 text-xs leading-5 text-slate-500 dark:text-slate-400">{description}</p>
          </div>
          <Button variant="ghost" size="icon" aria-label="关闭弹窗" onClick={onClose}>
            <X className="size-4" />
          </Button>
        </div>
        {children}
      </div>
    </div>
  );
}

function ApiProviderDialog({
  onClose,
  onCreated,
  onError,
}: {
  onClose: () => void;
  onCreated: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [name, setName] = React.useState("");
  const [baseUrl, setBaseUrl] = React.useState("");
  const [apiKey, setApiKey] = React.useState("");
  const [billing, setBilling] = React.useState<GatewayBillingMode>("metered");
  const [upstreamProtocol, setUpstreamProtocol] =
    React.useState<GatewayUpstreamProtocol>("openai_responses");
  const [compatibilityProfile, setCompatibilityProfile] =
    React.useState<GatewayCompatibilityProfile>("generic_openai");
  const [submitting, setSubmitting] = React.useState(false);
  const valid = name.trim() && baseUrl.trim() && apiKey.trim();

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!valid) return;
    setSubmitting(true);
    try {
      await gatewayApi.createProvider({
        name: name.trim(),
        base_url: baseUrl.trim(),
        api_key: apiKey.trim(),
        billing_mode: billing,
        upstream_protocol: upstreamProtocol,
        compatibility_profile: compatibilityProfile,
      });
      await onCreated();
    } catch (submitError) {
      onError(errorMessage(submitError));
      setSubmitting(false);
    }
  }

  return (
    <DialogFrame title="添加 API Key 供应商" description="接入 OpenAI 兼容接口。" onClose={onClose}>
      <form className="space-y-5" onSubmit={submit}>
        <FormField label="名称">
          <input className="field" value={name} onChange={(event) => setName(event.target.value)} placeholder="openai-proxy" autoFocus />
        </FormField>
        <FormField label="Base URL">
          <input
            className="field font-mono text-xs"
            value={baseUrl}
            onChange={(event) => {
              const value = event.target.value;
              setBaseUrl(value);
              setCompatibilityProfile(suggestedCompatibilityProfile(value));
            }}
            placeholder="https://api.example.com/v1"
          />
        </FormField>
        <FormField label="API Key">
          <input className="field font-mono text-xs" type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} placeholder="sk-..." />
        </FormField>
        <div className="grid gap-4 sm:grid-cols-2">
          <FormField label="计费">
            <div className="grid grid-cols-2 rounded-xl bg-slate-100 p-1 dark:bg-white/5">
              {(["metered", "subscription"] as GatewayBillingMode[]).map((mode) => (
                <button
                  key={mode}
                  type="button"
                  className={cn(
                    "rounded-lg px-3 py-2 text-xs font-bold transition",
                    billing === mode ? "bg-white shadow-sm dark:bg-white/10" : "text-slate-400",
                  )}
                  onClick={() => setBilling(mode)}
                >
                  {mode === "metered" ? "按量" : "订阅"}
                </button>
              ))}
            </div>
          </FormField>
          <FormField label="上游协议">
            <select
              className="field text-xs"
              value={upstreamProtocol}
              onChange={(event) =>
                setUpstreamProtocol(event.target.value as GatewayUpstreamProtocol)
              }
            >
              <option value="openai_responses">OpenAI Responses</option>
              <option value="openai_chat_completions">OpenAI Chat Completions</option>
            </select>
          </FormField>
        </div>
        <FormField label="兼容 Profile">
          <select
            className="field text-xs"
            value={compatibilityProfile}
            onChange={(event) =>
              setCompatibilityProfile(
                event.target.value as GatewayCompatibilityProfile,
              )
            }
          >
            <option value="generic_openai">通用 OpenAI 兼容接口</option>
            <option value="official_openai">OpenAI 官方 API</option>
          </select>
          <p className="mt-2 text-[11px] leading-5 text-slate-400">
            通用 Profile 会移除已知不兼容的 Codex 客户端工具；官方 Profile 保持 Responses 请求 Body 原样。
          </p>
        </FormField>
        <DialogActions onClose={onClose} disabled={!valid || submitting} submitting={submitting} label="创建供应商" />
      </form>
    </DialogFrame>
  );
}

function AccountDialog({
  onClose,
  onCreated,
  onError,
}: {
  onClose: () => void;
  onCreated: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [json, setJson] = React.useState("");
  const [submitting, setSubmitting] = React.useState(false);
  let parsed: CodexAuthPayload | null = null;
  try {
    const candidate = JSON.parse(json) as CodexAuthPayload;
    if (candidate.tokens?.access_token && candidate.tokens?.refresh_token) parsed = candidate;
  } catch {
    parsed = null;
  }

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!parsed) return;
    setSubmitting(true);
    try {
      await gatewayApi.importAccount(parsed);
      await onCreated();
    } catch (submitError) {
      onError(errorMessage(submitError));
      setSubmitting(false);
    }
  }

  return (
    <DialogFrame title="导入账户 Token" description="粘贴 Codex auth.json，导入后会自动生成账户供应商。" onClose={onClose}>
      <form onSubmit={submit}>
        <FormField label="OpenAI Codex Token">
          <textarea
            className="field min-h-64 resize-y font-mono text-[11px] leading-5"
            value={json}
            onChange={(event) => setJson(event.target.value)}
            placeholder={'{\n  "tokens": {\n    "access_token": "...",\n    "refresh_token": "..."\n  }\n}'}
            autoFocus
          />
        </FormField>
        <div className={cn("mt-2 flex items-center gap-2 text-[11px]", !json || parsed ? "text-slate-400" : "text-red-500")}>
          {parsed ? <Check className="size-3.5 text-emerald-500" /> : <CircleAlert className="size-3.5" />}
          {!json || parsed
            ? "需要 tokens.access_token 和 tokens.refresh_token。"
            : "JSON 格式无效，或缺少必要 Token。"}
        </div>
        <DialogActions onClose={onClose} disabled={!parsed || submitting} submitting={submitting} label="导入 Token" />
      </form>
    </DialogFrame>
  );
}

function SetupDialog({ onClose }: { onClose: () => void }) {
  const [copied, setCopied] = React.useState<"setup" | "restore" | null>(null);
  const gatewayUrl = `${window.location.origin}/openai/v1`;
  const setupScriptUrl = `${window.location.origin}/codex/setup.sh`;
  const restoreScriptUrl = `${window.location.origin}/codex/restore.sh`;
  const setupCommand = `curl -fsSL ${shellQuote(setupScriptUrl)} | sh -s -- ${shellQuote(gatewayUrl)}`;
  const restoreCommand = `curl -fsSL ${shellQuote(restoreScriptUrl)} | sh`;

  function copyCommand(kind: "setup" | "restore", command: string) {
    void copyText(command);
    setCopied(kind);
    window.setTimeout(() => setCopied(null), 1500);
  }

  return (
    <DialogFrame
      title="Codex 接入"
      description="运行一次脚本即可写入本机 Codex 配置并同步历史别名，不需要安装或启动本地 Agent。"
      onClose={onClose}
    >
      <div className="space-y-5">
        <div className="rounded-2xl border border-blue-500/15 bg-blue-500/5 p-4 text-xs leading-5 text-blue-700 dark:text-blue-300">
          在本机终端执行接入命令。脚本会记录原模型供应商、修改 <code>~/.codex/config.toml</code> 并为旧任务创建历史别名，不会安装程序或启动后台服务。
        </div>
        <FormField label="Gateway Base URL">
          <div className="flex gap-2">
            <input className="field min-w-0 flex-1 font-mono text-xs" readOnly value={gatewayUrl} />
            <Button
              variant="outline"
              size="icon"
              aria-label="复制 Gateway Base URL"
              onClick={() => void copyText(gatewayUrl)}
            >
              <Copy className="size-4" />
            </Button>
          </div>
        </FormField>
        <FormField label="接入命令">
          <div className="relative">
            <pre className="overflow-x-auto rounded-2xl bg-slate-950 p-4 pr-12 text-[11px] leading-5 text-slate-200">{setupCommand}</pre>
            <button
              className="absolute right-3 top-3 flex size-8 items-center justify-center rounded-lg bg-white/10 text-white hover:bg-white/15"
              type="button"
              aria-label="复制接入命令"
              onClick={() => copyCommand("setup", setupCommand)}
            >
              {copied === "setup" ? <Check className="size-4" /> : <Copy className="size-4" />}
            </button>
          </div>
        </FormField>
        <FormField label="恢复原配置">
          <div className="relative">
            <pre className="overflow-x-auto rounded-2xl bg-slate-950 p-4 pr-12 text-[11px] leading-5 text-slate-200">{restoreCommand}</pre>
            <button
              className="absolute right-3 top-3 flex size-8 items-center justify-center rounded-lg bg-white/10 text-white hover:bg-white/15"
              type="button"
              aria-label="复制恢复命令"
              onClick={() => copyCommand("restore", restoreCommand)}
            >
              {copied === "restore" ? <Check className="size-4" /> : <Copy className="size-4" />}
            </button>
          </div>
        </FormField>
        <div className="flex justify-end">
          <Button onClick={onClose}>完成</Button>
        </div>
      </div>
    </DialogFrame>
  );
}

function FormField({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-2 block text-[11px] font-bold text-slate-500 dark:text-slate-400">{label}</span>
      {children}
    </label>
  );
}

function DialogActions({
  onClose,
  disabled,
  submitting,
  label,
}: {
  onClose: () => void;
  disabled: boolean;
  submitting: boolean;
  label: string;
}) {
  return (
    <div className="mt-7 flex justify-end gap-2">
      <Button type="button" variant="outline" onClick={onClose}>取消</Button>
      <Button type="submit" disabled={disabled}>
        {submitting ? <LoaderCircle className="size-4 animate-spin" /> : <Plus className="size-4" />}
        {submitting ? "处理中" : label}
      </Button>
    </div>
  );
}

function LoadingState() {
  return (
    <div className="flex min-h-[420px] items-center justify-center">
      <div className="text-center">
        <LoaderCircle className="mx-auto size-7 animate-spin text-slate-400" />
        <div className="mt-3 text-xs font-semibold text-slate-400">正在连接远程 Server</div>
      </div>
    </div>
  );
}

function EmptyState({ onAddApi, onAddAccount }: { onAddApi: () => void; onAddAccount: () => void }) {
  return (
    <div className="glass-panel flex min-h-[420px] flex-col items-center justify-center rounded-[28px] px-6 text-center">
      <div className="flex size-14 items-center justify-center rounded-2xl bg-slate-900 text-white dark:bg-white dark:text-slate-950">
        <Activity className="size-6" />
      </div>
      <h2 className="mt-5 text-xl font-bold">还没有供应商</h2>
      <p className="mt-2 max-w-md text-sm leading-6 text-slate-500">
        添加一个 OpenAI 兼容 API，或者导入 Codex Token 自动生成账户供应商。
      </p>
      <div className="mt-6 flex flex-wrap justify-center gap-2">
        <Button onClick={onAddApi}><KeyRound className="size-4" />添加 API Key</Button>
        <Button variant="outline" onClick={onAddAccount}><UserRound className="size-4" />导入账户</Button>
      </div>
    </div>
  );
}

function ErrorToast({ message, onClose }: { message: string; onClose: () => void }) {
  return (
    <div className="fixed bottom-5 left-1/2 z-[70] flex w-[calc(100%-2rem)] max-w-lg -translate-x-1/2 items-start gap-3 rounded-2xl border border-red-500/20 bg-white/95 p-4 shadow-2xl backdrop-blur dark:bg-slate-900/95">
      <CircleAlert className="mt-0.5 size-4 shrink-0 text-red-500" />
      <div className="min-w-0 flex-1 break-words text-xs leading-5 text-slate-700 dark:text-slate-200">{message}</div>
      <button type="button" onClick={onClose}><X className="size-4 text-slate-400" /></button>
    </div>
  );
}

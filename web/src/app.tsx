import * as React from "react";
import {
  Activity,
  BarChart3,
  Bug,
  Check,
  CheckCircle2,
  ChevronDown,
  CircleAlert,
  Cloud,
  Code2,
  Copy,
  Eye,
  EyeOff,
  Gauge,
  KeyRound,
  LayoutDashboard,
  LoaderCircle,
  Plus,
  RefreshCw,
  Route,
  Server,
  Settings,
  Trash2,
  Wrench,
  UserRound,
  Users,
  UserPlus,
  Share2,
  LogOut,
  X,
} from "lucide-react";

import { authApi, gatewayApi } from "./api";
import { AuthProvider, useAuth } from "./auth";
import { LoginPage } from "./login";
import { Button } from "./components/ui/button";
import { cn } from "./lib/utils";
import type {
  AutoRoutingSettings,
  CodexAuthPayload,
  CodexClientVersionSetting,
  GatewayModel,
  GatewayIssue,
  GatewayProvider,
  ProviderQuotaSummary,
  ProviderQuotaWindow,
  ReasoningEffort,
  SelectedProvider,
  TurnRouteLog,
  UsageSummary,
  DailyUsageSummary,
  GatewayCompatibilityProfile,
  RoutingModelTarget,
  InstanceRoutingConfig,
  ModelBenchmarkResult,
  OpenAiDeviceLoginStart,
  SecuritySettings,
  GatewayGroup,
  GatewayGroupDetail,
  GatewayUser,
} from "./types";

type Dialog = "provider" | "instances" | "scripts" | null;
type Page = "overview" | "groups" | "usage" | "benchmark" | "routing" | "issues" | "account" | "admin";

const NAV_TABS: {
  id: Exclude<Page, "account" | "admin">;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
}[] = [
  { id: "overview", label: "概览", icon: LayoutDashboard },
  { id: "groups", label: "群组", icon: Users },
  { id: "usage", label: "Token 用量", icon: BarChart3 },
  { id: "benchmark", label: "吞吐测试", icon: Gauge },
  { id: "routing", label: "路由日志", icon: Route },
  { id: "issues", label: "网关问题", icon: Bug },
];

type QuotaMap = Record<string, ProviderQuotaSummary | undefined>;
type ErrorMap = Record<string, string | undefined>;
type UsagePeriod = "total" | "today" | "week";

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
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

function parseCodexAuthPayload(value: unknown): CodexAuthPayload | null {
  const entries = Array.isArray(value) ? value : [value];
  if (entries.length === 0) return null;

  const isSupported = entries.every((entry) => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) return false;
    const record = entry as Record<string, unknown>;
    return hasTokenPair(record.tokens) || hasTokenPair(record);
  });

  return isSupported ? (value as CodexAuthPayload) : null;
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

const NINEBOT_PRIVATE_DEPLOYMENT_PRESET = {
  name: "九号私有部署",
  baseUrl: "https://ai-service.segway-ninebot.com/v1",
  compatibilityProfile: "generic_openai" as const,
};

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

function formatTokenCount(value: number) {
  return new Intl.NumberFormat("zh-CN", {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}

export function App() {
  return (
    <AuthProvider>
      <AuthenticatedApp />
    </AuthProvider>
  );
}

function AuthenticatedApp() {
  const { loading, user } = useAuth();
  if (loading) {
    return <div className="flex min-h-screen items-center justify-center text-sm font-medium text-slate-500">正在验证登录状态…</div>;
  }
  return user ? <GatewayDashboard /> : <LoginPage />;
}

export function GatewayDashboard() {
  const { user, logout, authMode } = useAuth();
  const currentUser = user!;
  const [providers, setProviders] = React.useState<GatewayProvider[]>([]);
  const [groups, setGroups] = React.useState<GatewayGroup[]>([]);
  const [instances, setInstances] = React.useState<InstanceRoutingConfig[]>([]);
  const [defaultAutomaticRouting, setDefaultAutomaticRouting] = React.useState<AutoRoutingSettings>({
    enabled: false,
  });
  const [instanceToEdit, setInstanceToEdit] = React.useState<string | null>(null);
  const [scriptInstanceId, setScriptInstanceId] = React.useState<string | null>(null);
  const [selected, setSelected] = React.useState<SelectedProvider>({ updated_at: 0 });
  const [models, setModels] = React.useState<GatewayModel[]>([]);
  const [turnLogs, setTurnLogs] = React.useState<TurnRouteLog[]>([]);
  const [gatewayIssues, setGatewayIssues] = React.useState<GatewayIssue[]>([]);
  const [usageByPeriod, setUsageByPeriod] = React.useState<Record<UsagePeriod, UsageSummary[]>>({
    total: [],
    today: [],
    week: [],
  });
  const [dailyUsage, setDailyUsage] = React.useState<DailyUsageSummary[]>([]);
  const [quotas, setQuotas] = React.useState<QuotaMap>({});
  const [quotaErrors, setQuotaErrors] = React.useState<ErrorMap>({});
  const [loadingQuotas, setLoadingQuotas] = React.useState<Set<string>>(new Set());
  const [loading, setLoading] = React.useState(true);
  const [loadingModels, setLoadingModels] = React.useState(false);
  const [dialog, setDialog] = React.useState<Dialog>(null);
  const [activePage, setActivePage] = React.useState<Page>("overview");
  const [error, setError] = React.useState<string | null>(null);
  const [deleting, setDeleting] = React.useState<Set<string>>(new Set());

  const selectedProvider = providers.find((provider) => provider.id === selected.provider_id);

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
      const data = await gatewayApi.models(undefined, force);
      setModels([...data].sort((a, b) => a.id.localeCompare(b.id)));
    } catch (modelError) {
      setModels([]);
      setError(errorMessage(modelError));
    } finally {
      setLoadingModels(false);
    }
  }, []);

  const loadUsage = React.useCallback(async (showError = true) => {
    try {
      const [total, today, week, daily] = await Promise.all([
        gatewayApi.usageSummary("total"),
        gatewayApi.usageSummary("today"),
        gatewayApi.usageSummary("week"),
        gatewayApi.usageDaily(30),
      ]);
      setUsageByPeriod({ total, today, week });
      setDailyUsage(daily);
    } catch (usageError) {
      if (showError) setError(errorMessage(usageError));
    }
  }, []);

  const refresh = React.useCallback(async () => {
    setLoading(true);
    try {
      const [providerList, route, turns, issues, instanceList, automaticRouting, groupList] = await Promise.all([
        gatewayApi.providers(),
        gatewayApi.selectedProvider(),
        gatewayApi.routingTurns(),
        gatewayApi.gatewayIssues(),
        gatewayApi.instances(),
        gatewayApi.automaticRouting(),
        authMode === "required" ? gatewayApi.groups() : Promise.resolve([]),
      ]);
      const sorted = [...providerList].sort((a, b) => a.name.localeCompare(b.name));
      setProviders(sorted);
      setGroups(groupList);
      setInstances(instanceList);
      setDefaultAutomaticRouting(automaticRouting);
      setSelected(route);
      setTurnLogs(turns);
      setGatewayIssues(issues);
      setError(null);
      if (route.provider_id) {
        void loadModels();
      } else {
        setModels([]);
      }
      void loadQuotas(sorted);
      void loadUsage(false);
    } catch (loadError) {
      setError(errorMessage(loadError));
    } finally {
      setLoading(false);
    }
  }, [authMode, loadModels, loadQuotas, loadUsage]);

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
    setSelected((current) => ({
      ...current,
      provider_id: provider.id,
      selected_model: undefined,
      selected_reasoning_effort: undefined,
    }));
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

  async function selectReasoningEffort(effort: string) {
    try {
      const route = effort
        ? await gatewayApi.selectReasoningEffort(effort as ReasoningEffort)
        : await gatewayApi.clearSelectedReasoningEffort();
      setSelected(route);
    } catch (reasoningError) {
      setError(errorMessage(reasoningError));
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
      <header className="relative z-50 border-b border-white/50 bg-white/55 backdrop-blur-xl dark:border-white/8 dark:bg-slate-950/55">
        <div className="mx-auto flex h-16 max-w-[1480px] items-center gap-3 px-5 sm:px-8">
          <button
            type="button"
            className="flex items-center gap-3 rounded-xl text-left outline-none transition-opacity hover:opacity-80 focus-visible:ring-2 focus-visible:ring-blue-500"
            aria-label="返回首页"
            onClick={() => setActivePage("overview")}
          >
            <span className="flex size-9 items-center justify-center rounded-xl bg-slate-900 text-white shadow-lg shadow-slate-900/15 dark:bg-white dark:text-slate-950">
              <Cloud className="size-[18px]" />
            </span>
            <span className="text-[15px] font-bold tracking-[-0.02em]">AI网关</span>
          </button>
          <NavTabs active={activePage} onSelect={setActivePage} showGroups={authMode === "required"} />
          <div className="ml-auto flex items-center gap-2">
            <AccountMenu
              user={currentUser}
              showAccountSettings={authMode === "required"}
              showLogout={authMode === "required"}
              showAdminSettings={currentUser.role === "admin"}
              onOpenAccountSettings={() => setActivePage("account")}
              onOpenAdminSettings={() => setActivePage("admin")}
              onLogout={() => { void logout(); }}
            />
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-[1480px] px-5 py-6 sm:px-8 sm:py-8">
        {activePage === "account" ? (
          <AccountSettingsPage authMode={authMode} onError={setError} />
        ) : activePage === "admin" ? (
          <AdminSettingsPage
            onChanged={async () => {
              if (
                selectedProvider?.auth_mode === "account"
                && selectedProvider.compatibility_profile === "openai_codex"
              ) {
                await loadModels(true);
              }
            }}
          onError={setError}
          />
        ) : activePage === "groups" ? (
          <GroupsPage
            groups={groups}
            providers={providers}
            currentUserId={currentUser.id}
            onChanged={refresh}
            onError={setError}
          />
        ) : loading ? (
          <LoadingState />
        ) : activePage === "usage" ? (
          <UsageSection
            providers={providers}
            usageByPeriod={usageByPeriod}
            dailyUsage={dailyUsage}
          />
        ) : activePage === "benchmark" ? (
          <ModelBenchmarkSection
            providers={providers}
            initialProviderId={selected.provider_id}
            initialModel={selected.selected_model}
            onError={setError}
          />
        ) : activePage === "routing" ? (
          <TurnLogSection turns={turnLogs} />
        ) : activePage === "issues" ? (
          <GatewayIssueSection
            issues={gatewayIssues}
            onChanged={async () => {
              setGatewayIssues(await gatewayApi.gatewayIssues());
            }}
            onError={setError}
          />
        ) : (
          <>
            <section>
              <div className="mb-3 flex flex-wrap items-center gap-3 px-1">
                <h2 className="text-xs font-bold uppercase tracking-[0.12em] text-slate-500 dark:text-slate-400">
                  AI 网关
                </h2>
                <div className="ml-auto flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      setInstanceToEdit(null);
                      setDialog("instances");
                    }}
                  >
                    <Plus className="size-3.5" />
                    新建实例
                  </Button>
                  <Button variant="outline" size="sm" onClick={() => setDialog("provider")}>
                    <Plus className="size-3.5" />
                    添加供应商
                  </Button>
                </div>
              </div>
              <InstanceSection
                showHeader={false}
                instances={[{
                  instance_id: "default",
                  provider_id: selected.provider_id,
                  selected_model: selected.selected_model,
                  selected_reasoning_effort: selected.selected_reasoning_effort,
                  updated_at: selected.updated_at,
                  automatic_routing: defaultAutomaticRouting,
                }]}
                providers={providers}
                onShowScripts={(instanceId) => {
                  setScriptInstanceId(instanceId);
                  setDialog("scripts");
                }}
                onConfigure={(instanceId) => {
                if (instanceId === "default") {
                  setInstanceToEdit(instanceId);
                  setDialog("instances");
                }
              }}
                onChanged={refresh}
                onError={setError}
              />
            </section>

            {instances.length > 0 ? (
              <div className="mt-5">
                <InstanceSection
                  title="实例"
                  instances={instances}
                  providers={providers}
                  onShowScripts={(instanceId) => {
                    setScriptInstanceId(instanceId);
                    setDialog("scripts");
                  }}
                  onConfigure={(instanceId) => {
                    setInstanceToEdit(instanceId);
                    setDialog("instances");
                  }}
                  onChanged={refresh}
                  onError={setError}
                />
              </div>
            ) : null}

            {providers.length === 0 ? (
              <div className="mt-8"><EmptyState onAdd={() => setDialog("provider")} /></div>
            ) : (
              <div className="mt-8">
                <ProviderSection
                  title="供应商"
                  providers={providers}
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
          </>
        )}
      </main>

      {error ? <ErrorToast message={error} onClose={() => setError(null)} /> : null}
      {dialog === "provider" ? (
        <ProviderDialog
          onClose={() => setDialog(null)}
          onOpenSecuritySettings={() => {
            setDialog(null);
            setActivePage("admin");
          }}
          onCreated={async () => {
            setDialog(null);
            await refresh();
          }}
          onError={setError}
        />
      ) : null}
      {dialog === "instances" ? (
        <CodexInstancesDialog
          providers={providers}
          initialInstanceId={instanceToEdit ?? undefined}
          onClose={() => {
            setInstanceToEdit(null);
            setDialog(null);
          }}
          onChanged={refresh}
          onCreated={(instanceId) => {
            setInstanceToEdit(null);
            setScriptInstanceId(instanceId);
            setDialog("scripts");
          }}
          onError={setError}
        />
      ) : null}
      {dialog === "scripts" && scriptInstanceId ? (
        <CodexScriptsDialog
          instanceId={scriptInstanceId}
          onClose={() => {
            setScriptInstanceId(null);
            setDialog(null);
          }}
        />
      ) : null}
    </div>
  );
}


function NavTabs({
  active,
  onSelect,
  showGroups,
}: {
  active: Page;
  onSelect: (page: Page) => void;
  showGroups: boolean;
}) {
  return (
    <nav className="ml-2 flex min-w-0 items-center gap-1 overflow-x-auto whitespace-nowrap md:ml-6" aria-label="主导航">
      {NAV_TABS.filter(({ id }) => id !== "groups" || showGroups).map(({ id, label, icon: Icon }) => {
        const isActive = active === id;
        return (
          <button
            key={id}
            type="button"
            aria-current={isActive ? "page" : undefined}
            onClick={() => onSelect(id)}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-semibold transition-colors",
              isActive
                ? "bg-slate-900 text-white shadow-sm dark:bg-white dark:text-slate-950"
                : "text-slate-500 hover:bg-slate-900/5 hover:text-slate-900 dark:text-slate-400 dark:hover:bg-white/10 dark:hover:text-white",
            )}
          >
            <Icon className="size-4" />
            {label}
          </button>
        );
      })}
    </nav>
  );
}

function GroupsPage({
  groups,
  providers,
  currentUserId,
  onChanged,
  onError,
}: {
  groups: GatewayGroup[];
  providers: GatewayProvider[];
  currentUserId: number;
  onChanged: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [selectedGroupId, setSelectedGroupId] = React.useState<number | null>(groups[0]?.id ?? null);
  const [detail, setDetail] = React.useState<GatewayGroupDetail | null>(null);
  const [groupName, setGroupName] = React.useState("");
  const [search, setSearch] = React.useState("");
  const [searchResults, setSearchResults] = React.useState<GatewayUser[]>([]);
  const [busy, setBusy] = React.useState(false);

  React.useEffect(() => {
    if (selectedGroupId === null && groups[0]) setSelectedGroupId(groups[0].id);
    if (selectedGroupId !== null && !groups.some((group) => group.id === selectedGroupId)) {
      setSelectedGroupId(groups[0]?.id ?? null);
    }
  }, [groups, selectedGroupId]);

  const loadDetail = React.useCallback(async () => {
    if (selectedGroupId === null) {
      setDetail(null);
      return;
    }
    try {
      setDetail(await gatewayApi.group(selectedGroupId));
    } catch (loadError) {
      onError(errorMessage(loadError));
    }
  }, [onError, selectedGroupId]);

  React.useEffect(() => { void loadDetail(); }, [loadDetail]);

  React.useEffect(() => {
    if (search.trim().length < 1) {
      setSearchResults([]);
      return;
    }
    const timer = window.setTimeout(() => {
      void gatewayApi.searchUsers(search.trim())
        .then((payload) => setSearchResults(payload.users))
        .catch((searchError) => onError(errorMessage(searchError)));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [onError, search]);

  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    try {
      await action();
      await onChanged();
      await loadDetail();
    } catch (actionError) {
      onError(errorMessage(actionError));
    } finally {
      setBusy(false);
    }
  }

  async function createGroup(event: React.FormEvent) {
    event.preventDefault();
    if (!groupName.trim()) return;
    await run(async () => {
      const created = await gatewayApi.createGroup(groupName.trim());
      setGroupName("");
      setSelectedGroupId(created.id);
    });
  }

  const selectedGroup = detail?.group ?? groups.find((group) => group.id === selectedGroupId);
  const canManageMembers = selectedGroup?.role === "owner";
  const ownProviders = providers.filter((provider) => !provider.shared);
  const sharedProviderIds = new Set(detail?.providers.map((item) => item.provider.id) ?? []);

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <div className="eyebrow">协作空间</div>
          <h1 className="mt-1 text-2xl font-bold tracking-[-0.03em]">群组共享</h1>
          <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">邀请成员，并把自己的供应商放到群组里共同使用。</p>
        </div>
        <form className="flex gap-2" onSubmit={(event) => void createGroup(event)}>
          <input className="field h-9 w-48 text-xs" value={groupName} onChange={(event) => setGroupName(event.target.value)} placeholder="新群组名称" />
          <Button size="sm" type="submit" disabled={busy || !groupName.trim()}><Plus className="size-3.5" />新建群组</Button>
        </form>
      </div>

      {!groups.length ? (
        <div className="glass-panel rounded-[26px] p-10 text-center">
          <Users className="mx-auto size-10 text-slate-300" />
          <h2 className="mt-4 text-lg font-bold">还没有群组</h2>
          <p className="mt-1 text-sm text-slate-500">创建一个群组，邀请同事后即可共享供应商。</p>
        </div>
      ) : (
        <div className="grid gap-5 lg:grid-cols-[280px_1fr]">
          <div className="space-y-2">
            {groups.map((group) => (
              <button
                key={group.id}
                type="button"
                onClick={() => setSelectedGroupId(group.id)}
                className={cn(
                  "glass-panel w-full rounded-2xl p-4 text-left transition",
                  selectedGroupId === group.id && "ring-2 ring-blue-500/40",
                )}
              >
                <div className="flex items-center gap-3">
                  <span className="flex size-9 items-center justify-center rounded-xl bg-blue-500/10 text-blue-600"><Users className="size-4" /></span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-bold">{group.name}</span>
                    <span className="mt-1 block text-[11px] text-slate-400">{group.member_count} 位成员 · {group.provider_count} 个供应商</span>
                  </span>
                </div>
              </button>
            ))}
          </div>

          {selectedGroup && detail ? (
            <section className="glass-panel rounded-[26px] p-5 sm:p-6">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                  <div className="flex items-center gap-2">
                    <h2 className="text-xl font-bold">{detail.group.name}</h2>
                    <Badge tone={detail.group.role === "owner" ? "blue" : "slate"}>{detail.group.role === "owner" ? "创建者" : "成员"}</Badge>
                  </div>
                  <p className="mt-1 text-xs text-slate-400">创建者：{detail.group.owner_name}</p>
                </div>
              </div>

              <div className="mt-6 grid gap-6 xl:grid-cols-2">
                <div>
                  <div className="mb-3 flex items-center justify-between">
                    <h3 className="text-sm font-bold">群组成员</h3>
                    <span className="text-xs text-slate-400">{detail.members.length} 人</span>
                  </div>
                  {canManageMembers ? (
                    <div className="relative mb-3">
                      <div className="flex items-center gap-2">
                        <UserPlus className="size-4 text-slate-400" />
                        <input className="field h-9 text-xs" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索姓名或邮箱，添加成员" />
                      </div>
                      {searchResults.length ? (
                        <div className="absolute left-6 right-0 top-11 z-10 overflow-hidden rounded-xl border border-slate-200 bg-white shadow-xl dark:border-slate-700 dark:bg-slate-900">
                          {searchResults.map((user) => (
                            <button
                              key={user.id}
                              type="button"
                              className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs hover:bg-slate-100 dark:hover:bg-white/10"
                              onClick={() => void run(async () => {
                                await gatewayApi.addGroupMember(detail.group.id, user.id);
                                setSearch("");
                                setSearchResults([]);
                              })}
                            >
                              <span className="flex size-6 items-center justify-center overflow-hidden rounded-full bg-slate-200 text-[10px] font-bold">{user.avatar_url ? <img src={user.avatar_url} alt="" className="size-full object-cover" /> : user.name.slice(0, 1)}</span>
                              <span>{user.name}</span>
                            </button>
                          ))}
                        </div>
                      ) : null}
                    </div>
                  ) : null}
                  <div className="space-y-2">
                    {detail.members.map((member) => (
                      <div key={member.user_id} className="flex items-center gap-2 rounded-xl bg-slate-500/5 px-3 py-2">
                        <span className="flex size-7 items-center justify-center overflow-hidden rounded-full bg-slate-200 text-[10px] font-bold">{member.avatar_url ? <img src={member.avatar_url} alt="" className="size-full object-cover" /> : member.name.slice(0, 1)}</span>
                        <span className="min-w-0 flex-1 truncate text-xs font-semibold">{member.name}</span>
                        <Badge tone={member.role === "owner" ? "blue" : "slate"}>{member.role === "owner" ? "创建者" : "成员"}</Badge>
                        {canManageMembers && member.role !== "owner" ? <button type="button" className="text-[11px] text-red-500" disabled={busy} onClick={() => void run(() => gatewayApi.removeGroupMember(detail.group.id, member.user_id))}>移除</button> : null}
                      </div>
                    ))}
                  </div>
                </div>

                <div>
                  <div className="mb-3 flex items-center justify-between">
                    <h3 className="text-sm font-bold">共享供应商</h3>
                    <span className="text-xs text-slate-400">{detail.providers.length} 个</span>
                  </div>
                  <div className="mb-3 flex gap-2">
                    <select
                      className="field h-9 min-w-0 flex-1 text-xs"
                      defaultValue=""
                      disabled={busy}
                      onChange={(event) => {
                        const providerId = event.target.value;
                        event.target.value = "";
                        if (providerId) void run(() => gatewayApi.shareGroupProvider(detail.group.id, providerId));
                      }}
                    >
                      <option value="">选择自己的供应商并共享</option>
                      {ownProviders.filter((provider) => !sharedProviderIds.has(provider.id)).map((provider) => <option key={provider.id} value={provider.id}>{provider.name}</option>)}
                    </select>
                    <Share2 className="mt-2.5 size-4 shrink-0 text-slate-400" />
                  </div>
                  <div className="space-y-2">
                    {detail.providers.map((item) => (
                      <div key={item.provider.id} className="flex items-center gap-2 rounded-xl bg-slate-500/5 px-3 py-2">
                        <Server className="size-4 text-blue-500" />
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-xs font-semibold">{item.provider.name}</span>
                          <span className="block truncate text-[10px] text-slate-400">由 {item.shared_by_name} 共享</span>
                        </span>
                        {item.can_remove ? <button type="button" className="text-[11px] text-red-500" disabled={busy} onClick={() => void run(() => gatewayApi.unshareGroupProvider(detail.group.id, item.provider.id))}>取消共享</button> : null}
                      </div>
                    ))}
                    {!detail.providers.length ? <div className="rounded-xl border border-dashed border-slate-300/70 p-5 text-center text-xs text-slate-400 dark:border-slate-700">还没有共享供应商</div> : null}
                  </div>
                </div>
              </div>
            </section>
          ) : <div className="glass-panel rounded-[26px] p-8 text-sm text-slate-400">加载群组详情…</div>}
        </div>
      )}
    </div>
  );
}

function AccountMenu({

  user,
  showAccountSettings,
  showLogout,
  showAdminSettings,
  onOpenAccountSettings,
  onOpenAdminSettings,
  onLogout,
}: {
  user: { id: number; name: string; avatar_url: string; role?: "admin" | "user" };
  showAccountSettings: boolean;
  showLogout: boolean;
  showAdminSettings: boolean;
  onOpenAccountSettings: () => void;
  onOpenAdminSettings: () => void;
  onLogout: () => void;
}) {
  const [open, setOpen] = React.useState(false);
  const menuRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    function closeOnOutsideClick(event: MouseEvent) {
      if (!menuRef.current?.contains(event.target as Node)) setOpen(false);
    }
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", closeOnOutsideClick);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutsideClick);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, []);

  const initial = user.name.trim().charAt(0).toUpperCase();

  return (
    <div className="relative" ref={menuRef}>
      <button
        type="button"
        className="inline-flex h-8 items-center gap-2 rounded-xl border border-white/60 bg-white/55 px-2.5 text-xs font-semibold text-slate-700 shadow-sm transition hover:bg-white/85 dark:border-white/10 dark:bg-white/5 dark:text-slate-200 dark:hover:bg-white/10"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <span className="flex size-5 shrink-0 items-center justify-center overflow-hidden rounded-full bg-slate-900 text-[9px] font-bold text-white dark:bg-white dark:text-slate-950">
          {user.avatar_url ? <img src={user.avatar_url} alt="" className="size-full object-cover" /> : initial || <UserRound className="size-3" />}
        </span>
        <span className="hidden max-w-32 truncate sm:inline" title={user.name}>{user.name}</span>
        <ChevronDown className={cn("size-3.5 text-slate-400 transition-transform", open && "rotate-180")} />
      </button>
      {open ? (
        <div
          role="menu"
          className="absolute right-0 z-50 mt-2 w-52 overflow-hidden rounded-2xl border border-slate-200 bg-white py-1.5 shadow-xl dark:border-slate-700 dark:bg-slate-900"
        >
          <div className="border-b border-slate-200/70 px-3.5 py-2.5 dark:border-white/10">
            <div className="truncate text-xs font-bold text-slate-800 dark:text-slate-100">{user.name}</div>
            <div className="mt-0.5 text-[10px] text-slate-400">{user.role === "admin" ? "管理员" : "成员"}</div>
          </div>
          {showAccountSettings ? (
            <button
            type="button"
            role="menuitem"
            className="mx-1.5 flex w-[calc(100%-0.75rem)] items-center gap-2.5 rounded-xl px-3 py-2 text-left text-xs font-medium text-slate-600 transition hover:bg-slate-900/5 dark:text-slate-300 dark:hover:bg-white/8"
            onClick={() => {
              setOpen(false);
              onOpenAccountSettings();
            }}
          >
            <Settings className="size-3.5 text-slate-400" />
            账户设置
          </button>
          ) : null}
          {showAdminSettings ? (
            <button
              type="button"
              role="menuitem"
              className="mx-1.5 flex w-[calc(100%-0.75rem)] items-center gap-2.5 rounded-xl px-3 py-2 text-left text-xs font-medium text-slate-600 transition hover:bg-slate-900/5 dark:text-slate-300 dark:hover:bg-white/8"
              onClick={() => {
                setOpen(false);
                onOpenAdminSettings();
              }}
            >
              <Settings className="size-3.5 text-slate-400" />
              管理员设置
            </button>
          ) : null}
          {showLogout ? (
            <button
              type="button"
              role="menuitem"
              className="mx-1.5 flex w-[calc(100%-0.75rem)] items-center gap-2.5 rounded-xl px-3 py-2 text-left text-xs font-medium text-red-600 transition hover:bg-red-500/5 dark:text-red-400"
              onClick={() => {
                setOpen(false);
                onLogout();
              }}
            >
              <LogOut className="size-3.5" />
              退出登录
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function InstanceSection({
  title,
  showHeader = true,
  instances,
  providers,
  onShowScripts,
  onConfigure,
  onChanged,
  onError,
}: {
  title?: string;
  showHeader?: boolean;
  instances: InstanceRoutingConfig[];
  providers: GatewayProvider[];
  onShowScripts: (instanceId: string) => void;
  onConfigure: (instanceId: string) => void;
  onChanged: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const allInstances = instances;
  const [modelsByProvider, setModelsByProvider] = React.useState<Record<string, GatewayModel[]>>({});
  const [loadingModels, setLoadingModels] = React.useState(true);
  const [savingIds, setSavingIds] = React.useState<Set<string>>(new Set());
  const [instancePendingDeletion, setInstancePendingDeletion] = React.useState<InstanceRoutingConfig | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    if (!providers.length) {
      setModelsByProvider({});
      setLoadingModels(false);
      return () => { cancelled = true; };
    }
    setLoadingModels(true);
    void Promise.all(providers.map(async (provider) => [provider.id, await gatewayApi.models(provider.id)] as const))
      .then((entries) => {
        if (!cancelled) {
          setModelsByProvider(Object.fromEntries(entries.map(([id, models]) => [
            id,
            [...models].sort((a, b) => a.id.localeCompare(b.id)),
          ])));
        }
      })
      .catch((loadError) => {
        if (!cancelled) onError(`加载实例模型失败：${errorMessage(loadError)}`);
      })
      .finally(() => {
        if (!cancelled) setLoadingModels(false);
      });
    return () => { cancelled = true; };
  }, [providers, onError]);

  async function run(instanceId: string, action: () => Promise<unknown>) {
    setSavingIds((current) => new Set(current).add(instanceId));
    try {
      await action();
      await onChanged();
    } catch (saveError) {
      onError(errorMessage(saveError));
    } finally {
      setSavingIds((current) => {
        const next = new Set(current);
        next.delete(instanceId);
        return next;
      });
    }
  }

  function updateNamedInstance(
    instance: InstanceRoutingConfig,
    patch: Partial<Omit<InstanceRoutingConfig, "instance_id" | "updated_at" | "automatic_routing">> & { automatic_routing?: AutoRoutingSettings },
  ) {
    const providerId = "provider_id" in patch ? patch.provider_id : instance.provider_id;
    return gatewayApi.setInstanceConfig(instance.instance_id, {
      provider_id: providerId,
      selected_model: "selected_model" in patch ? patch.selected_model : instance.selected_model,
      selected_reasoning_effort: "selected_reasoning_effort" in patch
        ? patch.selected_reasoning_effort
        : instance.selected_reasoning_effort,
      automatic_routing: patch.automatic_routing ?? instance.automatic_routing,
    });
  }

  function updateProvider(instance: InstanceRoutingConfig, providerId: string) {
    void run(instance.instance_id, async () => {
      if (instance.instance_id === "default") {
        if (!providerId) return;
        await gatewayApi.selectProvider(providerId);
      } else {
        await updateNamedInstance(instance, {
          provider_id: providerId || undefined,
          selected_model: undefined,
          selected_reasoning_effort: undefined,
        });
      }
    });
  }

  function updateModel(instance: InstanceRoutingConfig, model: string) {
    void run(instance.instance_id, async () => {
      if (instance.instance_id === "default") {
        if (model) await gatewayApi.selectModel(model);
        else await gatewayApi.clearSelectedModel();
      } else {
        await updateNamedInstance(instance, { selected_model: model || undefined });
      }
    });
  }

  function updateReasoning(instance: InstanceRoutingConfig, effort: ReasoningEffort | "") {
    void run(instance.instance_id, async () => {
      if (instance.instance_id === "default") {
        if (effort) await gatewayApi.selectReasoningEffort(effort);
        else await gatewayApi.clearSelectedReasoningEffort();
      } else {
        await updateNamedInstance(instance, { selected_reasoning_effort: effort || undefined });
      }
    });
  }

  function toggleAutomaticRouting(instance: InstanceRoutingConfig) {
    const next = { ...instance.automatic_routing, enabled: !instance.automatic_routing.enabled };
    void run(instance.instance_id, async () => {
      if (instance.instance_id === "default") {
        await gatewayApi.setAutomaticRouting(next);
      } else {
        await updateNamedInstance(instance, { automatic_routing: next });
      }
    });
  }

  function deleteInstance(instance: InstanceRoutingConfig) {
    if (instance.instance_id !== "default") setInstancePendingDeletion(instance);
  }

  function confirmDeleteInstance() {
    if (!instancePendingDeletion) return;
    const instance = instancePendingDeletion;
    setInstancePendingDeletion(null);
    void run(instance.instance_id, () => gatewayApi.deleteInstance(instance.instance_id));
  }

  return (
    <section>
      {showHeader ? (
        <div className="mb-3 flex items-center gap-3 px-1">
          {title ? <h2 className="text-xs font-bold uppercase tracking-[0.12em] text-slate-500 dark:text-slate-400">{title}</h2> : null}
          <span className="rounded-full bg-white/60 px-2 py-0.5 text-[10px] font-bold text-slate-400 dark:bg-white/5">{allInstances.length}</span>
        </div>
      ) : null}
      <div className="space-y-3">
        {allInstances.map((instance) => (
          <InstanceCard
            key={instance.instance_id}
            instance={instance}
            providers={providers}
            models={modelsByProvider[instance.provider_id ?? ""] ?? []}
            loadingModels={loadingModels}
            saving={savingIds.has(instance.instance_id)}
            onProviderChange={(providerId) => updateProvider(instance, providerId)}
            onModelChange={(model) => updateModel(instance, model)}
            onReasoningChange={(effort) => updateReasoning(instance, effort)}
            onToggleAutomatic={() => toggleAutomaticRouting(instance)}
            onConfigureAutomatic={() => onConfigure(instance.instance_id)}
            onCodexScripts={() => onShowScripts(instance.instance_id)}
            onDelete={instance.instance_id === "default" ? undefined : () => deleteInstance(instance)}
          />
        ))}
      </div>
      {instancePendingDeletion ? (
        <DeleteInstanceDialog
          instanceId={instancePendingDeletion.instance_id}
          deleting={savingIds.has(instancePendingDeletion.instance_id)}
          onClose={() => setInstancePendingDeletion(null)}
          onConfirm={confirmDeleteInstance}
        />
      ) : null}
    </section>
  );
}

function InstanceCard({
  instance,
  providers,
  models,
  loadingModels,
  saving,
  onProviderChange,
  onModelChange,
  onReasoningChange,
  onToggleAutomatic,
  onConfigureAutomatic,
  onCodexScripts,
  onDelete,
}: {
  instance: InstanceRoutingConfig;
  providers: GatewayProvider[];
  models: GatewayModel[];
  loadingModels: boolean;
  saving: boolean;
  onProviderChange: (providerId: string) => void;
  onModelChange: (model: string) => void;
  onReasoningChange: (effort: ReasoningEffort | "") => void;
  onToggleAutomatic: () => void;
  onConfigureAutomatic: () => void;
  onCodexScripts: () => void;
  onDelete?: () => void;
}) {
  const isDefault = instance.instance_id === "default";
  const automaticReady = [
    instance.automatic_routing.light,
    instance.automatic_routing.standard,
    instance.automatic_routing.pro,
    instance.automatic_routing.max,
  ].every((target) => Boolean(target?.provider_id && target.model));
  const controlsDisabled = saving || instance.automatic_routing.enabled;

  return (
    <article className="glass-panel flex flex-col gap-4 rounded-[22px] p-4 lg:flex-row lg:items-center lg:gap-5">
      <div className="min-w-0 flex-1">
        <h3 className="truncate text-lg font-bold tracking-[-0.025em]">{isDefault ? "AI网关" : instance.instance_id}</h3>
        <div className="mt-1.5 truncate font-mono text-[11px] text-slate-400" title={isDefault ? "/openai/v1" : `/instances/${instance.instance_id}/openai/v1`}>
          {isDefault ? "/openai/v1" : `/instances/${instance.instance_id}/openai/v1`}
        </div>
      </div>

      <div className="hidden h-8 w-px bg-slate-200/70 lg:block dark:bg-white/10" />

      <div className="flex min-w-0 flex-1 items-center gap-3">
        <span className="eyebrow shrink-0">供应商</span>
        <div className="relative min-w-0 flex-1">
          <select className="field h-9 w-full appearance-none pr-9 text-xs font-semibold" value={instance.provider_id ?? ""} disabled={controlsDisabled} onChange={(event) => onProviderChange(event.target.value)}>
            <option value="">选择供应商</option>
            {providers.map((provider) => <option key={provider.id} value={provider.id}>{provider.account_email ? `${provider.name} (${provider.account_email})` : provider.name}</option>)}
          </select>
          <ChevronDown className="pointer-events-none absolute right-3 top-1/2 size-3.5 -translate-y-1/2 text-slate-400" />
        </div>
      </div>

      <div className="hidden h-8 w-px bg-slate-200/70 lg:block dark:bg-white/10" />

      <div className="flex min-w-0 flex-1 items-center gap-3">
        <span className="eyebrow shrink-0">模型</span>
        <div className="relative min-w-0 flex-1">
          <select className="field h-9 w-full appearance-none pr-9 font-mono text-xs font-semibold" value={instance.selected_model ?? ""} disabled={controlsDisabled || loadingModels || !instance.provider_id} onChange={(event) => onModelChange(event.target.value)}>
            <option value="">跟随请求模型</option>
            {models.map((model) => <option key={model.id} value={model.id}>{model.id}</option>)}
          </select>
          <ChevronDown className="pointer-events-none absolute right-3 top-1/2 size-3.5 -translate-y-1/2 text-slate-400" />
        </div>
      </div>

      <div className="hidden h-8 w-px bg-slate-200/70 lg:block dark:bg-white/10" />

      <div className="flex min-w-0 flex-1 items-center gap-3">
        <span className="eyebrow shrink-0">推理强度</span>
        <div className="relative min-w-0 flex-1">
          <select className="field h-9 w-full appearance-none pr-9 text-xs font-semibold" value={instance.selected_reasoning_effort ?? ""} disabled={controlsDisabled || !instance.provider_id} onChange={(event) => onReasoningChange(event.target.value as ReasoningEffort | "")}>
            <option value="">跟随请求</option>
            <option value="low">低（low）</option>
            <option value="medium">中（medium）</option>
            <option value="high">高（high）</option>
            <option value="xhigh">极高（xhigh）</option>
          </select>
          <ChevronDown className="pointer-events-none absolute right-3 top-1/2 size-3.5 -translate-y-1/2 text-slate-400" />
        </div>
      </div>

      <div className="hidden h-8 w-px bg-slate-200/70 lg:block dark:bg-white/10" />

      <div className="flex shrink-0 items-center justify-between gap-3 lg:justify-end">
        <button
          type="button"
          role="switch"
          aria-label="启用自动模型路由"
          aria-checked={instance.automatic_routing.enabled}
          disabled={saving || (!instance.automatic_routing.enabled && !automaticReady)}
          title={!instance.automatic_routing.enabled && !automaticReady ? "请先配置完整的自动路由" : "自动模型路由"}
          className={cn(
            "flex h-8 items-center gap-2 rounded-xl px-2 transition",
            instance.automatic_routing.enabled ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-300" : "bg-slate-500/8 text-slate-500",
            (saving || (!instance.automatic_routing.enabled && !automaticReady)) && "cursor-not-allowed opacity-50",
          )}
          onClick={onToggleAutomatic}
        >
          <span className={cn("relative block h-5 w-9 rounded-full p-0.5 transition", instance.automatic_routing.enabled ? "bg-emerald-500" : "bg-slate-300 dark:bg-white/15")}>
            <span className={cn("block size-4 rounded-full bg-white shadow-sm transition", instance.automatic_routing.enabled && "translate-x-4")} />
          </span>
          <span className="text-xs font-bold">自动路由</span>
        </button>
        <Button type="button" variant="outline" size="sm" onClick={onConfigureAutomatic}>
          路由配置
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={onCodexScripts}>
          <Code2 className="size-3.5" />
          Codex 脚本
        </Button>
        {onDelete ? (
          <Button type="button" variant="outline" size="icon" title="删除实例" disabled={saving} onClick={onDelete}>
            <Trash2 className="size-4 text-red-500" />
          </Button>
        ) : null}
      </div>
    </article>
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

function ModelBenchmarkSection({
  providers,
  initialProviderId,
  initialModel,
  onError,
}: {
  providers: GatewayProvider[];
  initialProviderId?: string;
  initialModel?: string;
  onError: (message: string) => void;
}) {
  const [providerId, setProviderId] = React.useState(initialProviderId ?? "");
  const [model, setModel] = React.useState(initialModel ?? "");
  const [models, setModels] = React.useState<GatewayModel[]>([]);
  const [loadingModels, setLoadingModels] = React.useState(false);
  const [running, setRunning] = React.useState(false);
  const [result, setResult] = React.useState<ModelBenchmarkResult | null>(null);
  const [accountUsageConfirmed, setAccountUsageConfirmed] = React.useState(false);
  const selectedProvider = providers.find((provider) => provider.id === providerId);
  const isAccountProvider = selectedProvider?.auth_mode === "account";

  React.useEffect(() => {
    if (!providerId) {
      setModels([]);
      setModel("");
      setAccountUsageConfirmed(false);
      return;
    }
    let cancelled = false;
    setLoadingModels(true);
    void gatewayApi.models(providerId)
      .then((items) => {
        if (!cancelled) {
          const sorted = [...items].sort((a, b) => a.id.localeCompare(b.id));
          setModels(sorted);
          setModel((current) => sorted.some((item) => item.id === current) ? current : "");
        }
      })
      .catch((loadError) => {
        if (!cancelled) onError(`加载压测模型失败：${errorMessage(loadError)}`);
      })
      .finally(() => {
        if (!cancelled) setLoadingModels(false);
      });
    return () => { cancelled = true; };
  }, [providerId, onError]);

  async function run() {
    if (!providerId || !model) return;
    if (isAccountProvider && !accountUsageConfirmed) return;
    setRunning(true);
    try {
      setResult(await gatewayApi.benchmarkModel({
        provider_id: providerId,
        model,
        runs: 3,
        account_usage_confirmed: accountUsageConfirmed,
      }));
    } catch (benchmarkError) {
      onError(errorMessage(benchmarkError));
    } finally {
      setRunning(false);
    }
  }

  return (
    <section>
      <div className="mb-3 flex items-center gap-3 px-1">
        <h2 className="text-xs font-bold uppercase tracking-[0.12em] text-slate-500 dark:text-slate-400">模型实测吞吐</h2>
        <span className="text-[11px] text-slate-400">3 次流式请求取中位数</span>
      </div>
      <div className="rounded-[22px] border border-white/70 bg-white/55 p-4 shadow-sm backdrop-blur-xl dark:border-white/8 dark:bg-white/[0.035]">
        <p className="mb-4 text-xs leading-5 text-slate-500 dark:text-slate-400">
          测量 TTFT（首个输出 token）、总耗时及生成吞吐。固定任务：Rust 2021 迭代版斐波那契函数及单元测试。
        </p>
        <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1.25fr)_auto] md:items-end">
          <FormField label="供应商">
            <select className="field h-10 w-full text-xs" value={providerId} disabled={running} onChange={(event) => {
              setProviderId(event.target.value);
              setAccountUsageConfirmed(false);
            }}>
              <option value="">选择供应商</option>
              {providers.map((provider) => <option key={provider.id} value={provider.id}>{provider.name}</option>)}
            </select>
          </FormField>
          <FormField label="模型">
            <select className="field h-10 w-full font-mono text-xs" value={model} disabled={running || loadingModels || !providerId} onChange={(event) => setModel(event.target.value)}>
              <option value="">{loadingModels ? "加载模型中…" : "选择模型"}</option>
              {models.map((item) => <option key={item.id} value={item.id}>{item.id}</option>)}
            </select>
          </FormField>
          <Button type="button" className="h-10" disabled={running || !providerId || !model || (isAccountProvider && !accountUsageConfirmed)} onClick={() => void run()}>
            {running ? <LoaderCircle className="size-4 animate-spin" /> : <Gauge className="size-4" />}
            {running ? "测试中…" : "开始测试"}
          </Button>
        </div>
        {isAccountProvider ? (
          <label className="mt-4 flex items-start gap-3 rounded-2xl border border-amber-500/25 bg-amber-500/5 p-3 text-xs text-amber-800 dark:text-amber-200">
            <input
              type="checkbox"
              className="mt-0.5 size-4"
              checked={accountUsageConfirmed}
              disabled={running}
              onChange={(event) => setAccountUsageConfirmed(event.target.checked)}
            />
            <span>
              我确认这会消耗当前 ChatGPT/Codex 账户额度，并理解测试将发起 3 次真实流式请求。
            </span>
          </label>
        ) : null}
        {result ? (
          <div className="mt-5 grid gap-3 sm:grid-cols-3">
            <BenchmarkMetric label="TTFT 中位数" value={`${result.median_ttft_ms} ms`} />
            <BenchmarkMetric label="总耗时中位数" value={`${(result.median_total_ms / 1000).toFixed(2)} s`} />
            <BenchmarkMetric
              label="生成吞吐中位数"
              value={result.median_generation_tokens_per_second
                ? `${result.median_generation_tokens_per_second.toFixed(1)} tok/s`
                : "上游未返回 token 用量"}
            />
            <div className="sm:col-span-3 rounded-xl bg-slate-900/[0.035] px-3 py-2 text-[11px] text-slate-500 dark:bg-white/[0.05] dark:text-slate-400">
              单次：{result.samples.map((sample) => `${sample.ttft_ms}ms TTFT / ${(sample.total_ms / 1000).toFixed(2)}s${sample.generation_tokens_per_second ? ` / ${sample.generation_tokens_per_second.toFixed(1)} tok/s` : ""}`).join(" · ")}
            </div>
            <div className="sm:col-span-3 space-y-2">
              {result.samples.map((sample, index) => (
                <details key={index} className="rounded-xl bg-slate-900/[0.035] px-3 py-2 text-xs dark:bg-white/[0.05]">
                  <summary className="cursor-pointer font-semibold text-slate-600 dark:text-slate-300">
                    查看第 {index + 1} 次模型回复
                  </summary>
                  <p className="mt-2 whitespace-pre-wrap leading-5 text-slate-500 dark:text-slate-400">
                    {sample.output_text || "上游未返回文本 delta。"}
                  </p>
                </details>
              ))}
            </div>
          </div>
        ) : null}
      </div>
    </section>
  );
}

function BenchmarkMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-2xl bg-slate-900/[0.035] p-3 dark:bg-white/[0.05]">
      <div className="text-[10px] font-bold uppercase tracking-[0.1em] text-slate-400">{label}</div>
      <div className="mt-1.5 font-mono text-lg font-bold tracking-[-0.025em]">{value}</div>
    </div>
  );
}

function UsageSection({
  providers,
  usageByPeriod,
  dailyUsage,
}: {
  providers: GatewayProvider[];
  usageByPeriod: Record<UsagePeriod, UsageSummary[]>;
  dailyUsage: DailyUsageSummary[];
}) {
  const providerNames = React.useMemo(
    () => new Map(providers.map((provider) => [provider.id, provider.name])),
    [providers],
  );
  const providerRows = usageByPeriod.today.filter((row) => !row.model);
  const modelRows = usageByPeriod.today.filter((row) => row.model);
  const totalFor = (period: UsagePeriod) =>
    usageByPeriod[period]
      .filter((row) => !row.model)
      .reduce((sum, row) => sum + row.total_tokens, 0);

  return (
    <section>
      <div className="mb-3 flex items-center gap-2 px-1">
        <BarChart3 className="size-4 text-blue-600 dark:text-blue-400" />
        <h2 className="text-sm font-bold">Token 用量</h2>
        <span className="text-xs text-slate-400">按实际的上游响应累计</span>
      </div>

      <div className="grid gap-3 sm:grid-cols-3">
        <UsageMetric label="累计总用量" value={totalFor("total")} />
        <UsageMetric label="今日用量" value={totalFor("today")} />
        <UsageMetric label="本周用量" value={totalFor("week")} />
      </div>
      <UsageDailyChart providers={providers} rows={dailyUsage} />

      <div className="glass-panel mt-3 overflow-hidden rounded-2xl">
        <div className="flex items-center justify-between border-b border-slate-900/6 px-5 py-3 dark:border-white/8">
          <div>
            <div className="text-sm font-bold">今日按供应商 / 模型</div>
            <div className="mt-0.5 text-[11px] text-slate-400">包含输入、输出、缓存与推理 token。</div>
          </div>
          <span className="rounded-full bg-blue-500/10 px-2.5 py-1 text-[10px] font-bold text-blue-700 dark:text-blue-300">
            UTC+8
          </span>
        </div>
        {providerRows.length === 0 ? (
          <div className="px-5 py-8 text-center text-xs text-slate-400">
            今天还没有收到带 usage 的模型响应。
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full min-w-[760px] text-left text-xs">
              <thead className="bg-slate-900/[0.025] text-[10px] font-bold uppercase tracking-[0.08em] text-slate-400 dark:bg-white/[0.035]">
                <tr>
                  <th className="px-5 py-3">供应商 / 模型</th>
                  <th className="px-3 py-3 text-right">请求</th>
                  <th className="px-3 py-3 text-right">输入</th>
                  <th className="px-3 py-3 text-right">输出</th>
                  <th className="px-3 py-3 text-right">缓存</th>
                  <th className="px-3 py-3 text-right">推理</th>
                  <th className="px-5 py-3 text-right">总计</th>
                </tr>
              </thead>
              <tbody>
                {providerRows.flatMap((provider) => {
                  const models = modelRows.filter((row) => row.provider_id === provider.provider_id);
                  return [
                    <UsageRow
                      key={`provider-${provider.provider_id}`}
                      row={provider}
                      label={providerNames.get(provider.provider_id) ?? provider.provider_id}
                      emphasized
                    />,
                    ...models.map((model) => (
                      <UsageRow
                        key={`model-${model.provider_id}-${model.model}`}
                        row={model}
                        label={model.model ?? "未知模型"}
                        nested
                      />
                    )),
                  ];
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </section>
  );
}

function UsageMetric({ label, value }: { label: string; value: number }) {
  return (
    <div className="glass-panel rounded-2xl p-4">
      <div className="text-[10px] font-bold uppercase tracking-[0.1em] text-slate-400">{label}</div>
      <div className="mt-1.5 font-mono text-2xl font-bold tracking-[-0.035em]">{formatTokenCount(value)}</div>
      <div className="mt-0.5 text-[11px] text-slate-400">tokens</div>
    </div>
  );
}

function UsageDailyChart({
  providers,
  rows,
}: {
  providers: GatewayProvider[];
  rows: DailyUsageSummary[];
}) {
  const providerNames = new Map(providers.map((provider) => [provider.id, provider.name]));
  const series = Array.from(new Set(rows.map((row) => `${row.provider_id}::${row.model}`)));
  const colors = ["bg-blue-500", "bg-emerald-500", "bg-violet-500", "bg-amber-500", "bg-rose-500", "bg-cyan-500"];
  const byDate = new Map<string, DailyUsageSummary[]>();
  rows.forEach((row) => byDate.set(row.date, [...(byDate.get(row.date) ?? []), row]));
  const dates = Array.from(byDate.keys()).sort();
  const totals = dates.map((date) => (byDate.get(date) ?? []).reduce((sum, row) => sum + row.total_tokens, 0));
  const max = Math.max(...totals, 1);

  return (
    <div className="glass-panel mt-3 rounded-2xl p-5">
      <div className="flex items-start justify-between gap-3">
        <div>
          <div className="text-sm font-bold">近 30 天每日消耗</div>
          <div className="mt-0.5 text-[11px] text-slate-400">按供应商 / 模型堆叠显示总 token。</div>
        </div>
        <div className="flex max-w-[60%] flex-wrap justify-end gap-x-3 gap-y-1">
          {series.map((key, index) => {
            const [providerId, model] = key.split("::");
            return (
              <span key={key} className="inline-flex items-center gap-1 text-[10px] text-slate-500 dark:text-slate-400">
                <span className={cn("size-2 rounded-full", colors[index % colors.length])} />
                {providerNames.get(providerId) ?? providerId} / {model}
              </span>
            );
          })}
        </div>
      </div>
      {dates.length === 0 ? (
        <div className="flex h-44 items-center justify-center text-xs text-slate-400">暂无历史用量数据。</div>
      ) : (
        <div className="mt-5 flex h-52 items-end gap-1.5 overflow-x-auto pb-6 sm:gap-2">
          {dates.map((date, dateIndex) => {
            const dateRows = byDate.get(date) ?? [];
            const total = totals[dateIndex];
            return (
              <div key={date} className="group relative flex h-full min-w-7 flex-1 flex-col items-center justify-end">
                <div className="pointer-events-none mb-1 rounded-lg bg-slate-900 px-2 py-1 text-[10px] font-bold text-white opacity-0 shadow-lg transition group-hover:opacity-100 dark:bg-white dark:text-slate-900">
                  {date}: {formatTokenCount(total)}
                </div>
                <div
                  className="flex w-full max-w-11 flex-col-reverse overflow-hidden rounded-t-lg bg-slate-900/5 dark:bg-white/8"
                  style={{ height: `${Math.max(4, (total / max) * 100)}%` }}
                >
                  {dateRows.map((row) => {
                    const key = `${row.provider_id}::${row.model}`;
                    const index = series.indexOf(key);
                    return (
                      <div
                        key={key}
                        className={cn(colors[index % colors.length], "min-h-0 opacity-85")}
                        style={{ height: `${(row.total_tokens / total) * 100}%` }}
                        title={`${row.model}: ${formatTokenCount(row.total_tokens)} tokens`}
                      />
                    );
                  })}
                </div>
                <span className="absolute mt-56 whitespace-nowrap text-[9px] text-slate-400">
                  {date.slice(5)}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function UsageRow({
  row,
  label,
  nested = false,
  emphasized = false,
}: {
  row: UsageSummary;
  label: string;
  nested?: boolean;
  emphasized?: boolean;
}) {
  const cells = [
    row.request_count,
    row.input_tokens,
    row.output_tokens,
    row.cached_input_tokens,
    row.reasoning_tokens,
    row.total_tokens,
  ];
  return (
    <tr className={cn(
      "border-t border-slate-900/5 dark:border-white/6",
      emphasized && "bg-slate-900/[0.025] dark:bg-white/[0.025]",
    )}>
      <td className={cn("px-5 py-3", nested && "pl-9 text-slate-500 dark:text-slate-400", emphasized && "font-bold")}>
        {nested ? <span className="mr-2 text-slate-300 dark:text-slate-600">└</span> : null}
        {label}
      </td>
      {cells.map((value, index) => (
        <td
          key={index}
          className={cn(
            "px-3 py-3 text-right font-mono text-slate-600 dark:text-slate-300",
            index === cells.length - 1 && "px-5 font-bold text-slate-900 dark:text-slate-100",
          )}
        >
          {formatTokenCount(value)}
        </td>
      ))}
    </tr>
  );
}

function GatewayIssueSection({
  issues,
  onChanged,
  onError,
}: {
  issues: GatewayIssue[];
  onChanged: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [copyingId, setCopyingId] = React.useState<string | null>(null);
  const [copiedId, setCopiedId] = React.useState<string | null>(null);
  const [clearing, setClearing] = React.useState(false);

  async function copyRepairPrompt(issue: GatewayIssue) {
    setCopyingId(issue.id);
    try {
      const { prompt } = await gatewayApi.gatewayIssueRepairPrompt(issue.id);
      await copyText(prompt);
      setCopiedId(issue.id);
      window.setTimeout(
        () => setCopiedId((current) => (current === issue.id ? null : current)),
        2_000,
      );
    } catch (copyError) {
      onError(errorMessage(copyError));
    } finally {
      setCopyingId(null);
    }
  }

  async function clearIssues() {
    if (!issues.length || !window.confirm(`确定清空全部 ${issues.length} 条网关问题记录吗？`)) {
      return;
    }
    setClearing(true);
    try {
      await gatewayApi.clearGatewayIssues();
      await onChanged();
    } catch (clearError) {
      onError(errorMessage(clearError));
    } finally {
      setClearing(false);
    }
  }

  return (
    <section>
      <div className="mb-3 flex flex-wrap items-center gap-3 px-1">
        <div className="flex items-center gap-2">
          <Bug className="size-4 text-red-500" />
          <h2 className="text-xs font-bold uppercase tracking-[0.12em] text-slate-500 dark:text-slate-400">
            网关问题
          </h2>
        </div>
        <span className="rounded-full bg-red-500/10 px-2 py-0.5 text-[10px] font-bold text-red-600 dark:text-red-400">
          {issues.length} / 200
        </span>
        <Button
          className="ml-auto"
          variant="outline"
          size="sm"
          disabled={!issues.length || clearing}
          onClick={() => void clearIssues()}
        >
          {clearing ? <LoaderCircle className="size-3.5 animate-spin" /> : <Trash2 className="size-3.5" />}
          一键清空
        </Button>
      </div>
      <div className="overflow-hidden rounded-[22px] border border-white/70 bg-white/55 shadow-sm backdrop-blur-xl dark:border-white/8 dark:bg-white/[0.035]">
        {!issues.length ? (
          <div className="px-5 py-10 text-center">
            <CheckCircle2 className="mx-auto size-7 text-emerald-500" />
            <div className="mt-2 text-sm font-semibold text-slate-600 dark:text-slate-300">
              暂无网关问题
            </div>
            <div className="mt-1 text-xs text-slate-400">成功请求不会写入该列表。</div>
          </div>
        ) : (
          <div className="divide-y divide-slate-100/80 dark:divide-white/[0.055]">
            {issues.map((issue) => (
              <article key={issue.id} className="p-5">
                <div className="flex flex-col gap-4 lg:flex-row lg:items-start">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge tone="red">
                        {issue.status_code ? `HTTP ${issue.status_code}` : gatewayIssueKindLabel(issue.failure_kind)}
                      </Badge>
                      <span className="font-mono text-[11px] font-semibold text-slate-700 dark:text-slate-200">
                        {issue.model}
                      </span>
                      <span className="text-[10px] text-slate-400">
                        {issue.provider_name} · {issue.instance_id ?? "default"}
                      </span>
                      <span className="text-[10px] text-slate-400">{turnLogTime(issue.created_at)}</span>
                    </div>
                    <div className="mt-2 break-words text-xs font-medium leading-5 text-red-600 dark:text-red-400">
                      {issue.error_message}
                    </div>
                    <div className="mt-1 truncate font-mono text-[10px] text-slate-400" title={issue.upstream_url}>
                      {issue.upstream_url}
                    </div>
                    <details className="mt-3 rounded-xl bg-slate-900/[0.035] px-3 py-2 text-[10px] dark:bg-white/[0.05]">
                      <summary className="cursor-pointer font-semibold text-slate-500 dark:text-slate-400">
                        查看上游原始请求 / 响应
                      </summary>
                      <IssuePayload
                        label={`请求${issue.request_truncated ? "（已截断）" : ""}`}
                        value={issue.request_body}
                      />
                      <IssuePayload
                        label={`响应${issue.response_truncated ? "（已截断）" : ""}`}
                        value={issue.response_body ?? "（无响应体）"}
                      />
                    </details>
                  </div>
                  <Button
                    variant="default"
                    size="sm"
                    disabled={copyingId === issue.id}
                    onClick={() => void copyRepairPrompt(issue)}
                  >
                    {copyingId === issue.id ? (
                      <LoaderCircle className="size-3.5 animate-spin" />
                    ) : copiedId === issue.id ? (
                      <Check className="size-3.5" />
                    ) : (
                      <Wrench className="size-3.5" />
                    )}
                    {copiedId === issue.id ? "提示词已复制" : "复制修复提示词"}
                  </Button>
                </div>
              </article>
            ))}
          </div>
        )}
      </div>
      <p className="mt-2 px-1 text-[11px] leading-5 text-slate-400">
        仅记录上游连接失败、非 2xx、响应读取失败和流中断；每位用户最多保留 200 条，单个请求和响应各最多 128 KiB。
        “复制修复提示词”只负责把故障证据和安全约束组成提示词并复制到剪贴板，不会在网关内执行修复；
        复制后可粘贴到 Codex 或其他 Agent 的用户输入中。
      </p>
    </section>
  );
}

function IssuePayload({ label, value }: { label: string; value: string }) {
  return (
    <div className="mt-3">
      <div className="font-bold uppercase tracking-[0.08em] text-slate-400">{label}</div>
      <pre className="mt-1 max-h-64 overflow-auto whitespace-pre-wrap break-words font-mono leading-4 text-slate-600 dark:text-slate-300">
        {value}
      </pre>
    </div>
  );
}

function gatewayIssueKindLabel(kind: string) {
  const labels: Record<string, string> = {
    upstream_connect_error: "连接上游失败",
    upstream_http_error: "上游响应异常",
    response_read_error: "读取响应失败",
    stream_interrupted: "响应流中断",
  };
  return labels[kind] ?? kind;
}

function TurnLogSection({ turns }: { turns: TurnRouteLog[] }) {
  const [copiedTurnId, setCopiedTurnId] = React.useState<string | null>(null);

  async function copyRawRoutingRecord(turn: TurnRouteLog) {
    const sections = [
      turn.classifier_raw_input ? `【输入】\n${turn.classifier_raw_input}` : null,
      turn.classifier_raw_output ? `【输出】\n${turn.classifier_raw_output}` : null,
    ].filter((section): section is string => section !== null);
    await copyText(sections.join("\n\n"));
    setCopiedTurnId(turn.turn_id);
    window.setTimeout(() => setCopiedTurnId((current) => current === turn.turn_id ? null : current), 1500);
  }

  return (
    <section>
      <div className="mb-3 flex items-center gap-3 px-1">
        <h2 className="text-xs font-bold uppercase tracking-[0.12em] text-slate-500 dark:text-slate-400">
          最近路由 Turn
        </h2>
        <span className="rounded-full bg-white/60 px-2 py-0.5 text-[10px] font-bold text-slate-400 dark:bg-white/5">
          {turns.length} / 1000
        </span>
      </div>
      <div className="overflow-x-auto rounded-[22px] border border-white/70 bg-white/55 shadow-sm backdrop-blur-xl dark:border-white/8 dark:bg-white/[0.035]">
        {!turns.length ? (
          <div className="px-5 py-8 text-center text-xs text-slate-400">还没有可观测的路由 turn。</div>
        ) : (
          <table className="w-full min-w-[980px] text-left text-xs">
            <thead className="border-b border-slate-200/70 text-[10px] uppercase tracking-[0.1em] text-slate-400 dark:border-white/8">
              <tr>
                <th className="px-5 py-3 font-bold">时间</th>
                <th className="px-4 py-3 font-bold">用户输入</th>
                <th className="px-4 py-3 font-bold">模型</th>
                <th className="px-4 py-3 font-bold">路由原因</th>
                <th className="px-4 py-3 font-bold">推理度</th>
                <th className="px-5 py-3 text-right font-bold">回合</th>
              </tr>
            </thead>
            <tbody>
              {turns.map((turn) => (
                <tr key={turn.turn_id} className="border-b border-slate-100/80 last:border-0 dark:border-white/[0.055]">
                  <td className="whitespace-nowrap px-5 py-3.5 text-slate-500 dark:text-slate-400">{turnLogTime(turn.updated_at)}</td>
                  <td className="max-w-[300px] px-4 py-3.5">
                    <div className="truncate font-medium" title={turn.user_input_preview}>
                      {turn.user_input_preview ?? ""}
                    </div>
                    <div className="mt-1 font-mono text-[9px] text-slate-400">{turn.turn_id.slice(0, 17)}</div>
                  </td>
                  <td className="px-4 py-3.5 font-mono font-semibold">{turn.model}</td>
                  <td className="px-4 py-3.5">
                    <div className="flex items-center gap-2">
                      <Badge tone={turn.routing_tier === "xhigh" ? "purple" : turn.routing_tier === "low" ? "green" : "blue"}>
                        {routingTierLabel(turn.routing_tier) ?? turn.routing_mode}
                      </Badge>
                      {turn.classifier_confidence !== undefined ? (
                        <span className="font-mono text-[10px] text-slate-400">
                          {(turn.classifier_confidence * 100).toFixed(0)}%
                        </span>
                      ) : null}
                    </div>
                    <div className="mt-1 text-[10px] text-slate-400" title={turn.routing_reason}>
                      {routingReasonLabel(turn.routing_reason)}
                    </div>
                    {turn.routing_detail ? (
                      <div className="mt-1 max-w-[360px] break-words text-[10px] leading-4 text-red-500" title={turn.routing_detail}>
                        {turn.routing_detail}
                      </div>
                    ) : null}
                    {turn.classifier_output ? (
                      <div className="mt-1 max-w-[360px] truncate font-mono text-[10px] text-slate-500 dark:text-slate-400" title={turn.classifier_output}>
                        返回：{turn.classifier_output}
                      </div>
                    ) : null}
                    {turn.classifier_raw_input || turn.classifier_raw_output ? (
                      <details className="mt-2 max-w-[420px] rounded-lg bg-slate-900/[0.035] px-2 py-1.5 text-[10px] dark:bg-white/[0.05]">
                        <div className="flex items-center justify-between gap-2">
                          <summary className="cursor-pointer font-semibold text-slate-500 dark:text-slate-400">
                            查看路由模型原始输入 / 输出
                          </summary>
                          <button
                            type="button"
                            className="inline-flex shrink-0 items-center gap-1 rounded-md px-1.5 py-1 font-semibold text-slate-500 hover:bg-slate-900/5 dark:text-slate-400 dark:hover:bg-white/10"
                            aria-label="复制路由模型原始输入和输出"
                            onClick={(event) => {
                              event.preventDefault();
                              void copyRawRoutingRecord(turn);
                            }}
                          >
                            {copiedTurnId === turn.turn_id ? <Check className="size-3" /> : <Copy className="size-3" />}
                            {copiedTurnId === turn.turn_id ? "已复制" : "一键复制"}
                          </button>
                        </div>
                        {turn.classifier_raw_input ? (
                          <div className="mt-2">
                            <div className="font-bold uppercase tracking-[0.08em] text-slate-400">输入</div>
                            <pre className="mt-1 max-h-52 overflow-auto whitespace-pre-wrap break-words font-mono leading-4 text-slate-600 dark:text-slate-300">
                              {turn.classifier_raw_input}
                            </pre>
                          </div>
                        ) : null}
                        {turn.classifier_raw_output ? (
                          <div className="mt-2">
                            <div className="font-bold uppercase tracking-[0.08em] text-slate-400">输出</div>
                            <pre className="mt-1 max-h-52 overflow-auto whitespace-pre-wrap break-words font-mono leading-4 text-slate-600 dark:text-slate-300">
                              {turn.classifier_raw_output}
                            </pre>
                          </div>
                        ) : null}
                      </details>
                    ) : null}
                  </td>
                  <td className="px-4 py-3.5 text-slate-500 dark:text-slate-400">{turn.reasoning_effort ?? "—"}</td>
                  <td className="px-5 py-3.5 text-right text-slate-500 dark:text-slate-400">
                    {turn.request_count} 请求 · {turn.tool_round_count} 工具
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      <p className="mt-2 px-1 text-[11px] leading-5 text-slate-400">
        自动路由时会保存路由模型的原始请求和响应（其中可能包含用户输入），供排障使用；不保存工具结果和最终回答。超过 1000 条按最近访问淘汰。
      </p>
    </section>
  );
}

function routingReasonLabel(reason: string) {
  const labels: Record<string, string> = {
    automatic_routing_disabled: "自动路由未开启",
    selected_model_override: "手动选择模型覆盖",
    same_turn_model_reuse: "同一 Turn 复用既定模型",
    visual_input_requires_strong_model: "图片输入保守使用高级别模型",
    visual_input_requires_max_model: "图片输入保守使用高级别模型",
    tool_continuation_without_turn_binding: "工具后续请求未关联到原 Turn",
    light_model_not_configured: "低级别路由模型未配置",
    classifier_provider_not_found: "路由分类供应商不存在",
    classifier_request_failed: "路由分类模型请求失败",
    classifier_output_text_missing: "路由分类模型没有返回文本",
    classifier_output_invalid: "分类结果无法解析",
    classifier_selected: "路由分类模型直接选择",
    classifier_confidence_below_threshold: "分类置信度低于阈值",
  };
  return labels[reason] ?? reason;
}

function routingTierLabel(tier: TurnRouteLog["routing_tier"]) {
  const labels = {
    low: "低",
    medium: "中",
    high: "高",
    xhigh: "极高",
  };
  return tier ? labels[tier] : undefined;
}

function turnLogTime(timestamp: number) {
  return new Date(timestamp * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
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
        "provider-card group relative flex h-[252px] cursor-pointer flex-col rounded-[24px] p-5",
        selected && "selected",
        deleting && "pointer-events-none opacity-60",
      )}
      onClick={onSelect}
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-lg font-bold tracking-[-0.025em]">
            {provider.auth_mode === "account"
              ? (provider.account_email ?? "等待账户登录")
              : provider.name}
          </h3>
          <div className="mt-2 flex flex-wrap gap-1.5">
            <Badge tone={provider.auth_mode === "account" ? "green" : "blue"}>
              {provider.auth_mode === "account" ? "账户" : "API Key"}
            </Badge>
            <Badge tone="slate">
              {provider.compatibility_profile === "official_openai"
                ? "OpenAI 官方"
                : provider.compatibility_profile === "openai_codex"
                  ? "Codex 账户"
                  : "兼容接口"}
            </Badge>
            {provider.shared ? <Badge tone="purple"><Share2 className="mr-1 inline size-3" />群组共享</Badge> : null}
          </div>
        </div>
        {!provider.shared && selected ? (
          <div className="relative size-5 shrink-0">
            <CheckCircle2 className="size-5 text-blue-500" />
            {deleting ? (
              <LoaderCircle className="absolute right-0 top-6 size-5 animate-spin text-slate-400" />
            ) : (
              <button
                className="absolute -right-1.5 top-6 flex size-8 items-center justify-center rounded-xl text-slate-400 opacity-0 transition hover:bg-black/5 hover:text-red-500 group-hover:opacity-100 focus-visible:opacity-100 dark:hover:bg-white/8"
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
        ) : !provider.shared && deleting ? (
          <LoaderCircle className="size-5 shrink-0 animate-spin text-slate-400" />
        ) : !provider.shared ? (
          <button
            className="flex size-8 shrink-0 items-center justify-center rounded-xl text-slate-400 opacity-0 transition hover:bg-black/5 hover:text-red-500 group-hover:opacity-100 focus-visible:opacity-100 dark:hover:bg-white/8"
            type="button"
            title="删除供应商"
            onClick={(event) => {
              event.stopPropagation();
              onDelete();
            }}
          >
            <Trash2 className="size-3.5" />
          </button>
        ) : null}
      </div>

      {provider.auth_mode !== "account" ? (
        <div className="mt-auto pt-3">
          <div className="eyebrow">Base URL</div>
          <div className="mt-1.5 truncate font-mono text-[11px] text-slate-500 dark:text-slate-400">
            {provider.base_url}
          </div>
        </div>
      ) : null}

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
    <div className="mt-auto rounded-2xl border border-white/65 bg-white/45 p-3 dark:border-white/8 dark:bg-white/[0.035]">
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

function Badge({ children, tone }: { children: React.ReactNode; tone: "green" | "blue" | "purple" | "amber" | "slate" | "red" }) {
  return (
    <span
      className={cn(
        "rounded-full px-2.5 py-1 text-[10px] font-bold",
        tone === "green" && "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
        tone === "blue" && "bg-blue-500/10 text-blue-600 dark:text-blue-400",
        tone === "purple" && "bg-violet-500/10 text-violet-600 dark:text-violet-400",
        tone === "amber" && "bg-amber-500/10 text-amber-700 dark:text-amber-400",
        tone === "slate" && "bg-slate-500/10 text-slate-500 dark:text-slate-400",
        tone === "red" && "bg-red-500/10 text-red-600 dark:text-red-400",
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
  wide = false,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  React.useEffect(() => {
    const handler = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/30 p-4 backdrop-blur-sm" onMouseDown={onClose}>
      <div
        className={cn(
          "dialog-panel max-h-[calc(100vh-2rem)] w-full overflow-y-auto rounded-[26px] p-6 sm:p-7",
          wide ? "max-w-4xl" : "max-w-xl",
        )}
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

function ProviderDialog({
  onClose,
  onOpenSecuritySettings,
  onCreated,
  onError,
}: {
  onClose: () => void;
  onOpenSecuritySettings: () => void;
  onCreated: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [providerType, setProviderType] = React.useState<"api" | "account">("account");
  const [apiTabVisited, setApiTabVisited] = React.useState(false);
  const [security, setSecurity] = React.useState<SecuritySettings | null>(null);

  React.useEffect(() => {
    void gatewayApi.securitySettings().then(setSecurity).catch(() => {
      // Non-admin users cannot read security settings. A missing key cannot occur
      // while account mode is enabled, so keep the form available for them.
      setSecurity({
        encryption_key_configured: true,
        feishu_app_id: "",
        feishu_app_secret_configured: true,
        auth_required: true,
      });
    });
  }, []);

  return (
    <DialogFrame
      title="添加供应商"
      description="选择使用 API Key 接入 OpenAI 兼容接口，或添加 ChatGPT 账户。"
      onClose={onClose}
    >
      {security && !security.encryption_key_configured ? (
        <div className="mb-5 rounded-2xl border border-amber-500/25 bg-amber-500/10 p-4 text-sm leading-6 text-amber-800 dark:text-amber-200">
          <div className="font-bold">尚未设置数据库加密密钥</div>
          <p className="mt-1 text-xs">
            为避免 API Key 和账户 Token 以明文保存，当前不能添加供应商。请先完成安全设置。
          </p>
          <Button className="mt-3" size="sm" variant="outline" onClick={onOpenSecuritySettings}>
            <Settings className="size-4" />
            前往管理员设置
          </Button>
        </div>
      ) : null}
      <div className="mb-5 flex rounded-xl bg-slate-100 p-1 text-xs font-semibold dark:bg-white/[0.06]">
        <button
          type="button"
          className={cn(
            "flex flex-1 items-center justify-center gap-2 rounded-lg px-3 py-2 transition",
            providerType === "account"
              ? "bg-white text-slate-900 shadow-sm dark:bg-slate-800 dark:text-white"
              : "text-slate-500",
          )}
          onClick={() => setProviderType("account")}
        >
          <UserRound className="size-3.5" />
          ChatGPT 账户
        </button>
        <button
          type="button"
          className={cn(
            "flex flex-1 items-center justify-center gap-2 rounded-lg px-3 py-2 transition",
            providerType === "api"
              ? "bg-white text-slate-900 shadow-sm dark:bg-slate-800 dark:text-white"
              : "text-slate-500",
          )}
          onClick={() => {
            setApiTabVisited(true);
            setProviderType("api");
          }}
        >
          <KeyRound className="size-3.5" />
          API Key
        </button>
      </div>
      {security?.encryption_key_configured ? (
        <>
          <div className={providerType === "account" ? undefined : "hidden"}>
            <AccountProviderForm onClose={onClose} onCreated={onCreated} onError={onError} />
          </div>
          {apiTabVisited ? (
            <div className={providerType === "api" ? undefined : "hidden"}>
              <ApiProviderForm onClose={onClose} onCreated={onCreated} onError={onError} />
            </div>
          ) : null}
        </>
      ) : null}
    </DialogFrame>
  );
}

function ApiProviderForm({
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
        compatibility_profile: compatibilityProfile,
      });
      await onCreated();
    } catch (submitError) {
      onError(errorMessage(submitError));
      setSubmitting(false);
    }
  }

  function applyNinebotPrivateDeploymentPreset() {
    setName(NINEBOT_PRIVATE_DEPLOYMENT_PRESET.name);
    setBaseUrl(NINEBOT_PRIVATE_DEPLOYMENT_PRESET.baseUrl);
    setCompatibilityProfile(
      NINEBOT_PRIVATE_DEPLOYMENT_PRESET.compatibilityProfile,
    );
  }

  return (
    <form className="space-y-5" onSubmit={submit}>
      <div>
        <div className="mb-2 text-[11px] font-bold text-slate-500 dark:text-slate-400">预置供应商</div>
        <Button
          type="button"
          variant="outline"
          className="h-auto w-full justify-start px-3 py-3 text-left"
          onClick={applyNinebotPrivateDeploymentPreset}
        >
          <Server className="size-4 shrink-0 text-blue-500" />
          <span className="min-w-0">
            <span className="block text-xs font-bold">九号私有部署</span>
            <span className="mt-0.5 block truncate font-mono text-[10px] font-normal text-slate-400">
              https://ai-service.segway-ninebot.com/v1
            </span>
          </span>
        </Button>
      </div>
      <FormField label="名称">
        <input className="field" value={name} onChange={(event) => setName(event.target.value)} placeholder="例如 my-openai" autoFocus />
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
  );
}

function AccountProviderForm({
  onClose,
  onCreated,
  onError,
}: {
  onClose: () => void;
  onCreated: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [mode, setMode] = React.useState<"login" | "token">("login");
  const [json, setJson] = React.useState("");
  const [submitting, setSubmitting] = React.useState(false);
  const [deviceLogin, setDeviceLogin] = React.useState<OpenAiDeviceLoginStart | null>(null);
  const [loginState, setLoginState] = React.useState<"idle" | "starting" | "waiting" | "finalizing" | "failed">("idle");
  const [loginError, setLoginError] = React.useState<string | null>(null);
  let parsed: CodexAuthPayload | null = null;
  try {
    parsed = parseCodexAuthPayload(JSON.parse(json));
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

  const startLogin = React.useCallback(async () => {
    setLoginState("starting");
    setLoginError(null);
    try {
      const login = await gatewayApi.startOpenAiDeviceLogin();
      setDeviceLogin(login);
      setLoginState("waiting");
    } catch (startError) {
      setLoginState("failed");
      setLoginError(errorMessage(startError));
    }
  }, []);

  React.useEffect(() => {
    if (mode !== "login" || loginState !== "idle") return;
    void startLogin();
  }, [loginState, mode, startLogin]);

  React.useEffect(() => {
    if (
      mode !== "login" ||
      !deviceLogin ||
      (loginState !== "waiting" && loginState !== "finalizing")
    ) {
      return;
    }
    const timer = window.setInterval(() => {
      void (async () => {
        try {
          const result = await gatewayApi.pollOpenAiDeviceLogin(deviceLogin.login_id);
          if (result.status === "finalizing") {
            setLoginState("finalizing");
            return;
          }
          if (result.status === "failed") {
            setLoginState("failed");
            setLoginError(result.error ?? "账户登录失败");
            return;
          }
          if (result.status === "completed") {
            await onCreated();
          }
        } catch (pollError) {
          setLoginState("failed");
          setLoginError(errorMessage(pollError));
        }
      })();
    }, Math.max(2_000, deviceLogin.interval_seconds * 1_000));
    return () => window.clearInterval(timer);
  }, [deviceLogin, loginState, mode, onCreated]);

  React.useEffect(() => () => {
    if (deviceLogin) {
      void gatewayApi.cancelOpenAiDeviceLogin(deviceLogin.login_id).catch(() => {});
    }
  }, [deviceLogin]);

  return (
    <div>
      <p className="mb-5 text-xs leading-5 text-slate-500 dark:text-slate-400">
        通过 OpenAI 官方设备授权登录；凭据会加密保存在本机。
      </p>
      <div className="mb-5 flex rounded-xl bg-slate-100 p-1 text-xs font-semibold dark:bg-white/[0.06]">
        <button
          type="button"
          className={cn("flex-1 rounded-lg px-3 py-2 transition", mode === "login" ? "bg-white text-slate-900 shadow-sm dark:bg-slate-800 dark:text-white" : "text-slate-500")}
          onClick={() => setMode("login")}
        >
          登录账户
        </button>
        <button
          type="button"
          className={cn("flex-1 rounded-lg px-3 py-2 transition", mode === "token" ? "bg-white text-slate-900 shadow-sm dark:bg-slate-800 dark:text-white" : "text-slate-500")}
          onClick={() => setMode("token")}
        >
          导入 Token
        </button>
      </div>

      {mode === "login" ? (
        <div className="space-y-4">
          {loginState === "failed" ? (
            <div className="rounded-2xl border border-red-200 bg-red-50 p-4 text-sm text-red-600 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-300">
              <div>{loginError ?? "无法创建账户登录。"}</div>
              <Button className="mt-3" variant="outline" size="sm" onClick={() => {
                setDeviceLogin(null);
                setLoginState("idle");
                setLoginError(null);
              }}>
                重试
              </Button>
            </div>
          ) : loginState === "starting" || !deviceLogin ? (
            <div className="flex min-h-36 items-center justify-center gap-2 text-sm text-slate-500">
              <LoaderCircle className="size-4 animate-spin" /> 正在创建授权…
            </div>
          ) : (
            <>
              <p className="text-sm leading-6 text-slate-600 dark:text-slate-300">
                在新标签页完成 OpenAI 登录，然后输入下面的设备代码。完成后此窗口会自动保存账户。
              </p>
              <div className="rounded-2xl border border-blue-200/70 bg-blue-50/70 p-4 dark:border-blue-400/15 dark:bg-blue-500/[0.07]">
                <div className="text-[11px] font-bold uppercase tracking-[0.12em] text-blue-600 dark:text-blue-300">设备代码</div>
                <div className="mt-2 flex items-center gap-3">
                  <code className="text-xl font-bold tracking-[0.14em] text-slate-900 dark:text-white">{deviceLogin.user_code}</code>
                  <Button variant="outline" size="sm" onClick={() => void copyText(deviceLogin.user_code)}>复制</Button>
                </div>
              </div>
              <Button className="w-full" onClick={() => window.open(deviceLogin.verification_uri, "_blank", "noopener,noreferrer")}>
                <UserRound className="size-4" /> 打开 OpenAI 登录页
              </Button>
              <div className="flex items-center justify-center gap-2 text-xs text-slate-400">
                <LoaderCircle className="size-3.5 animate-spin" />
                {loginState === "finalizing" ? "正在保存账户…" : "等待授权完成…"}
              </div>
            </>
          )}
        </div>
      ) : (
        <form onSubmit={submit}>
          <FormField label="OpenAI Codex Token">
            <textarea
              className="field min-h-56 resize-y font-mono text-[11px] leading-5"
              value={json}
              onChange={(event) => setJson(event.target.value)}
              placeholder={'{\n  "tokens": {\n    "access_token": "...",\n    "refresh_token": "..."\n  }\n}\n\n或 Cockpit Tools 导出的 JSON 数组：\n[\n  {\n    "access_token": "...",\n    "refresh_token": "...",\n    "type": "codex"\n  }\n]'}
              autoFocus
            />
          </FormField>
          <div className={cn("mt-2 flex items-center gap-2 text-[11px]", !json || parsed ? "text-slate-400" : "text-red-500")}>
            {parsed ? <Check className="size-3.5 text-emerald-500" /> : <CircleAlert className="size-3.5" />}
            {!json || parsed
              ? "支持官方 auth.json，以及 Cockpit Tools 导出的单个或多个 Codex 账号。"
              : "JSON 格式无效，或缺少 access_token / refresh_token。"}
          </div>
          <DialogActions onClose={onClose} disabled={!parsed || submitting} submitting={submitting} label="导入 Token" />
        </form>
      )}
    </div>
  );
}

function CodexInstancesDialog({
  providers,
  initialInstanceId,
  onClose,
  onChanged,
  onCreated,
  onError,
}: {
  providers: GatewayProvider[];
  initialInstanceId?: string;
  onClose: () => void;
  onChanged: () => Promise<void>;
  onCreated: (instanceId: string) => void;
  onError: (message: string) => void;
}) {
  const [instanceIds, setInstanceIds] = React.useState<string[]>([]);
  const [instanceId, setInstanceId] = React.useState("");
  const [isCreating, setIsCreating] = React.useState(!initialInstanceId);
  const [providerId, setProviderId] = React.useState("");
  const [selectedModel, setSelectedModel] = React.useState("");
  const [reasoningEffort, setReasoningEffort] = React.useState<ReasoningEffort | "">("");
  const [automaticRouting, setAutomaticRouting] = React.useState<AutoRoutingSettings>({
    enabled: false,
  });
  const [modelsByProvider, setModelsByProvider] = React.useState<Record<string, GatewayModel[]>>({});
  const [loadingModels, setLoadingModels] = React.useState(true);
  const [saving, setSaving] = React.useState(false);

  const refreshInstances = React.useCallback(async () => {
    const items = await gatewayApi.instances();
    setInstanceIds(items.map((item) => item.instance_id));
  }, []);

  React.useEffect(() => {
    void refreshInstances()
      .catch((loadError) => onError(errorMessage(loadError)));
  }, [onError, refreshInstances]);

  React.useEffect(() => {
    if (initialInstanceId) {
      void loadInstance(initialInstanceId);
    }
  }, [initialInstanceId]);

  React.useEffect(() => {
    let cancelled = false;
    if (!providers.length) {
      setModelsByProvider({});
      setLoadingModels(false);
      return () => { cancelled = true; };
    }
    setLoadingModels(true);
    void Promise.all(providers.map(async (provider) => [provider.id, await gatewayApi.models(provider.id)] as const))
      .then((entries) => {
        if (cancelled) return;
        setModelsByProvider(Object.fromEntries(entries.map(([id, models]) => [
          id,
          [...models].sort((a, b) => a.id.localeCompare(b.id)),
        ])));
      })
      .catch((loadError) => {
        if (!cancelled) onError(`加载供应商模型失败：${errorMessage(loadError)}`);
      })
      .finally(() => {
        if (!cancelled) setLoadingModels(false);
      });
    return () => { cancelled = true; };
  }, [providers, onError]);

  async function loadInstance(id: string) {
    try {
      const config = await gatewayApi.instanceConfig(id);
      setInstanceId(config.instance_id);
      setProviderId(config.provider_id ?? "");
      setSelectedModel(config.selected_model ?? "");
      setReasoningEffort(config.selected_reasoning_effort ?? "");
      setAutomaticRouting(config.automatic_routing);
      setIsCreating(false);
    } catch (loadError) {
      onError(errorMessage(loadError));
    }
  }

  const validInstanceId = /^[A-Za-z0-9_-]{1,64}$/.test(instanceId);
  const instanceNameTaken = isCreating && instanceIds.includes(instanceId);
  const canSave = Boolean(
    validInstanceId
      && !instanceNameTaken
      && !saving,
  );
  async function save() {
    if (!canSave) return;
    const creatingInstance = isCreating;
    setSaving(true);
    try {
      const config: Omit<InstanceRoutingConfig, "instance_id" | "updated_at"> = {
        provider_id: providerId || undefined,
        selected_model: selectedModel || undefined,
        selected_reasoning_effort: reasoningEffort || undefined,
        automatic_routing: automaticRouting,
      };
      const saved = await gatewayApi.setInstanceConfig(instanceId, config);
      setInstanceId(saved.instance_id);
      setAutomaticRouting(saved.automatic_routing);
      await Promise.all([refreshInstances(), onChanged()]);
      if (creatingInstance) {
        onCreated(saved.instance_id);
        return;
      }
      setIsCreating(false);
      onClose();
    } catch (saveError) {
      onError(errorMessage(saveError));
    } finally {
      setSaving(false);
    }
  }

  function updateAutomatic<K extends keyof AutoRoutingSettings>(key: K, value: AutoRoutingSettings[K]) {
    setAutomaticRouting((current) => ({ ...current, [key]: value }));
  }

  return (
    <DialogFrame
      title={isCreating ? "新建实例" : "路由配置"}
      description={isCreating ? "只需填写实例名称，其他配置可直接在实例卡片中调整。" : `为实例“${instanceId}”配置各路由级别的供应商、模型和推理强度。`}
      onClose={onClose}
      wide={!isCreating}
    >
      <div className="min-w-0 space-y-5">
        {isCreating ? (
          <>
            <FormField label="实例名称">
              <input className="field w-full font-mono text-sm" value={instanceId} maxLength={64} disabled={saving} placeholder="例如 codex-a" onChange={(event) => setInstanceId(event.target.value)} />
            </FormField>
            {instanceId && !validInstanceId ? <p className="-mt-3 text-[11px] text-red-500">实例名称只能包含字母、数字、_ 或 -，最多 64 个字符。</p> : null}
            {instanceNameTaken ? <p className="-mt-3 text-[11px] text-red-500">该实例名称已存在，请换一个名称。</p> : null}
            <div className="flex flex-wrap justify-end gap-2">
              <Button type="button" variant="outline" onClick={onClose}>取消</Button>
              <Button type="button" disabled={!canSave} onClick={() => void save()}>
                {saving ? <LoaderCircle className="size-4 animate-spin" /> : <Check className="size-4" />}
                {saving ? "创建中" : "创建实例"}
              </Button>
            </div>
          </>
        ) : (
          <>
            <section>
              <p className="text-[11px] leading-5 text-slate-400">
                低级别模型同时承担路由分类；按低、中、高、极高配置供应商、模型和推理强度。留空推理强度时跟随请求；低置信度阈值固定为 0.7，并回退到高级别模型。
              </p>
              <div className="mt-5 space-y-4">
                <div className="rounded-[22px] border border-white/65 bg-white/35 p-3 sm:p-4 dark:border-white/8 dark:bg-white/[0.025]">
                  <div className="space-y-2.5">
                    <div className="hidden grid-cols-[5.5rem_minmax(0,1.15fr)_minmax(0,1.15fr)_8rem] gap-3 px-2 text-[10px] font-bold uppercase tracking-[0.1em] text-slate-400 lg:grid">
                      <span>路由级别</span><span>供应商</span><span>模型</span><span>推理强度</span>
                    </div>
                    <RouteTargetSelect label="低（low）" value={automaticRouting.light} providers={providers} modelsByProvider={modelsByProvider} disabled={saving || loadingModels} onChange={(value) => updateAutomatic("light", value)} />
                    <RouteTargetSelect label="中（medium）" value={automaticRouting.standard} providers={providers} modelsByProvider={modelsByProvider} disabled={saving || loadingModels} onChange={(value) => updateAutomatic("standard", value)} />
                    <RouteTargetSelect label="高（high）" value={automaticRouting.pro} providers={providers} modelsByProvider={modelsByProvider} disabled={saving || loadingModels} onChange={(value) => updateAutomatic("pro", value)} />
                    <RouteTargetSelect label="极高（xhigh）" value={automaticRouting.max} providers={providers} modelsByProvider={modelsByProvider} disabled={saving || loadingModels} onChange={(value) => updateAutomatic("max", value)} />
                  </div>
                </div>
              </div>
            </section>

          <div className="flex flex-wrap justify-end gap-2">
            <Button type="button" variant="outline" onClick={onClose}>取消</Button>
            <Button type="button" disabled={!canSave} onClick={() => void save()}>
              {saving ? <LoaderCircle className="size-4 animate-spin" /> : <Check className="size-4" />}
              {saving ? "保存中" : "保存路由配置"}
            </Button>
          </div>
          </>
          )}
      </div>
    </DialogFrame>
  );
}

function DeleteInstanceDialog({
  instanceId,
  deleting,
  onClose,
  onConfirm,
}: {
  instanceId: string;
  deleting: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const [copied, setCopied] = React.useState(false);
  const instancesScriptUrl = `${window.location.origin}/codex/instances.sh`;
  const deleteCommand = `curl -fsSL ${shellQuote(instancesScriptUrl)} | sh -s -- delete ${shellQuote(instanceId)}`;

  function copyDeleteCommand() {
    void copyText(deleteCommand);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  return (
    <DialogFrame
      title={`删除实例：${instanceId}`}
      description="删除前请按需保存该 Codex 实例的本地清理命令。"
      onClose={onClose}
    >
      <div className="space-y-5">
        <div className="rounded-2xl border border-amber-500/20 bg-amber-500/5 p-4 text-xs leading-5 text-amber-700 dark:text-amber-300">
          确认删除只会删除网关中的实例路由配置。下方命令会删除本机的 Codex 实例文件夹，包括独立配置、登录信息、会话和 Electron 数据。
        </div>
        <FormField label="清理 / 删除 Codex 实例">
          <CommandBlock
            command={deleteCommand}
            copied={copied}
            copyLabel="复制 Codex 实例删除命令"
            onCopy={copyDeleteCommand}
          />
        </FormField>
        <div className="flex flex-wrap justify-end gap-2">
          <Button type="button" variant="outline" disabled={deleting} onClick={onClose}>取消</Button>
          <Button type="button" disabled={deleting} onClick={onConfirm} className="bg-red-600 text-white hover:bg-red-700">
            {deleting ? <LoaderCircle className="size-4 animate-spin" /> : <Trash2 className="size-4" />}
            {deleting ? "删除中" : "确认删除实例"}
          </Button>
        </div>
      </div>
    </DialogFrame>
  );
}

function CodexScriptsDialog({
  instanceId,
  onClose,
}: {
  instanceId: string;
  onClose: () => void;
}) {
  const [copied, setCopied] = React.useState<"setup" | "cleanup" | null>(null);
  const [accessToken, setAccessToken] = React.useState<string | null>(null);
  const [tokenError, setTokenError] = React.useState<string | null>(null);
  const { authMode } = useAuth();
  const isDefault = instanceId === "default";
  const setupScriptUrl = `${window.location.origin}/codex/setup.sh`;
  const cleanupScriptUrl = `${window.location.origin}/codex/restore.sh`;
  const instancesScriptUrl = `${window.location.origin}/codex/instances.sh`;
  const gatewayUrl = isDefault
    ? `${window.location.origin}/openai/v1`
    : `${window.location.origin}/instances/${encodeURIComponent(instanceId)}/openai/v1`;

  React.useEffect(() => {
    if (authMode === "disabled") {
      setAccessToken("");
      return;
    }
    void authApi
      .gatewayAccessToken()
      .then((value) => setAccessToken(value.access_token))
      .catch((error) => setTokenError(errorMessage(error)));
  }, [authMode]);

  const setupCommand = isDefault
    ? `curl -fsSL ${shellQuote(setupScriptUrl)} | sh -s -- ${shellQuote(gatewayUrl)}${accessToken ? ` ${shellQuote(accessToken)}` : ""}`
    : `curl -fsSL ${shellQuote(instancesScriptUrl)} | sh -s -- create ${shellQuote(instanceId)} ${shellQuote(gatewayUrl)}${accessToken ? ` ${shellQuote(accessToken)}` : ""}`;
  const displaySetupCommand = accessToken
    ? setupCommand.replace(accessToken, "********")
    : setupCommand;
  const cleanupCommand = isDefault
    ? `curl -fsSL ${shellQuote(cleanupScriptUrl)} | sh`
    : `curl -fsSL ${shellQuote(instancesScriptUrl)} | sh -s -- delete ${shellQuote(instanceId)}`;

  function copyCommand(kind: "setup" | "cleanup", command: string) {
    void copyText(command);
    setCopied(kind);
    window.setTimeout(() => setCopied(null), 1500);
  }

  return (
    <DialogFrame
      title={isDefault ? "默认 Codex 脚本" : `Codex 实例脚本：${instanceId}`}
      description={isDefault ? "设置默认 Codex，或清理 AI Gateway 写入的默认实例配置。" : "创建独立的 Codex 窗口，或删除该窗口的全部本地实例文件。"}
      onClose={onClose}
    >
      <div className="space-y-5">
        <div className="rounded-2xl border border-blue-500/15 bg-blue-500/5 p-4 text-xs leading-5 text-blue-700 dark:text-blue-300">
          {isDefault
            ? <>设置脚本会修改 <code>~/.codex/config.toml</code> 并同步历史别名；清理脚本会还原原模型供应商，并移除 AI Gateway 写入的 TOML 配置。</>
            : <>新建脚本会创建独立的 <code>CODEX_HOME</code> 和 Electron 数据目录，请在新窗口中单独登录账号。清理脚本会删除该实例文件夹及其中的登录信息、会话和 Electron 数据。</>}
        </div>
        {tokenError ? (
          <div className="rounded-2xl border border-red-500/20 bg-red-500/5 p-4 text-xs text-red-700 dark:text-red-300">
            {tokenError}
          </div>
        ) : null}
        {authMode === "required" && accessToken === null ? (
          <div className="flex min-h-20 items-center justify-center">
            <LoaderCircle className="size-5 animate-spin text-slate-400" />
          </div>
        ) : (
        <FormField label={isDefault ? "设置默认 Codex" : "新建 Codex 实例（macOS）"}>
          <CommandBlock
            command={setupCommand}
            displayCommand={displaySetupCommand}
            copied={copied === "setup"}
            copyLabel={isDefault ? "复制默认 Codex 设置命令" : "复制新建 Codex 实例命令"}
            onCopy={() => copyCommand("setup", setupCommand)}
          />
        </FormField>
        )}
        <FormField label={isDefault ? "清理默认 Codex 设置" : "清理 / 删除 Codex 实例"}>
          <CommandBlock
            command={cleanupCommand}
            copied={copied === "cleanup"}
            copyLabel={isDefault ? "复制默认 Codex 清理命令" : "复制 Codex 实例删除命令"}
            onCopy={() => copyCommand("cleanup", cleanupCommand)}
          />
        </FormField>
        <div className="flex justify-end">
          <Button onClick={onClose}>完成</Button>
        </div>
      </div>
    </DialogFrame>
  );
}

function CommandBlock({
  command,
  displayCommand,
  copied,
  copyLabel,
  onCopy,
}: {
  command: string;
  displayCommand?: string;
  copied: boolean;
  copyLabel: string;
  onCopy: () => void;
}) {
  return (
    <div className="relative">
      <pre className="overflow-x-auto rounded-2xl bg-slate-950 p-4 pr-12 text-[11px] leading-5 text-slate-200">{displayCommand ?? command}</pre>
      <button
        className="absolute right-3 top-3 flex size-8 items-center justify-center rounded-lg bg-white/10 text-white hover:bg-white/15"
        type="button"
        aria-label={copyLabel}
        onClick={onCopy}
      >
        {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
      </button>
    </div>
  );
}

function RegenerateEncryptionKeyDialog({
  configured,
  submitting,
  onClose,
  onConfirm,
}: {
  configured: boolean;
  submitting: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  return (
    <DialogFrame
      title={configured ? "重新生成数据库加密密钥" : "生成数据库加密密钥"}
      description="系统将生成新的 32 字节随机密钥，密钥内容不会显示或返回到浏览器。"
      onClose={submitting ? () => {} : onClose}
    >
      <div className="space-y-5">
        <div className="rounded-2xl border border-amber-500/20 bg-amber-500/5 p-4 text-xs leading-5 text-amber-700 dark:text-amber-300">
          {configured
            ? "确认后会立即替换当前密钥，并在同一 SQLite 事务中重新加密全部已保存凭据。操作过程中请勿关闭或重启服务；任一记录处理失败时会自动回滚。"
            : "确认后会立即生成并启用数据库加密密钥。"}
        </div>
        <div className="flex justify-end gap-2">
          <Button type="button" variant="outline" disabled={submitting} onClick={onClose}>
            取消
          </Button>
          <Button
            type="button"
            disabled={submitting}
            onClick={onConfirm}
            className="bg-amber-600 text-white hover:bg-amber-700"
          >
            {submitting ? <LoaderCircle className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
            {submitting ? "正在重新加密" : configured ? "确认重新生成" : "确认生成"}
          </Button>
        </div>
      </div>
    </DialogFrame>
  );
}

function AdminSettingsPage({
  onChanged,
  onError,
}: {
  onChanged: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [setting, setSetting] = React.useState<CodexClientVersionSetting | null>(null);
  const [security, setSecurity] = React.useState<SecuritySettings | null>(null);
  const [version, setVersion] = React.useState("");
  const [feishuAppId, setFeishuAppId] = React.useState("");
  const [feishuAppSecret, setFeishuAppSecret] = React.useState("");
  const [authRequired, setAuthRequired] = React.useState(false);
  const [submitting, setSubmitting] = React.useState(false);
  const [securitySaving, setSecuritySaving] = React.useState(false);
  const [securitySaved, setSecuritySaved] = React.useState(false);
  const [feishuAppSecretVisible, setFeishuAppSecretVisible] = React.useState(false);
  const [revealingFeishuAppSecret, setRevealingFeishuAppSecret] = React.useState(false);
  const [showKeyConfirmation, setShowKeyConfirmation] = React.useState(false);
  const [regeneratingKey, setRegeneratingKey] = React.useState(false);
  const credentialSaveTimer = React.useRef<number | null>(null);
  const securitySavingRef = React.useRef(false);

  React.useEffect(() => {
    void gatewayApi
      .codexClientVersion()
      .then((value) => {
        setSetting(value);
        setVersion(value.override_version ?? "");
      })
      .catch((loadError) => onError(errorMessage(loadError)));
    void gatewayApi
      .securitySettings()
      .then((value) => {
        setSecurity(value);
        setFeishuAppId(value.feishu_app_id);
        setAuthRequired(value.auth_required);
      })
      .catch((loadError) => onError(errorMessage(loadError)));
  }, [onError]);

  React.useEffect(() => () => {
    if (credentialSaveTimer.current !== null) {
      window.clearTimeout(credentialSaveTimer.current);
    }
  }, []);

  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (!version.trim()) return;
    setSubmitting(true);
    try {
      const value = await gatewayApi.setCodexClientVersion(version.trim());
      setSetting(value);
      setVersion(value.override_version ?? "");
      await onChanged();
    } catch (saveError) {
      onError(errorMessage(saveError));
    } finally {
      setSubmitting(false);
    }
  }

  function clearCredentialSaveTimer() {
    if (credentialSaveTimer.current !== null) {
      window.clearTimeout(credentialSaveTimer.current);
      credentialSaveTimer.current = null;
    }
  }

  function hasFeishuCredentials() {
    return Boolean(
      feishuAppId.trim()
      && (feishuAppSecret.trim() || security?.feishu_app_secret_configured),
    );
  }

  async function saveFeishuCredentials() {
    if (
      !security
      || securitySavingRef.current
      || !hasFeishuCredentials()
      || (
        feishuAppId.trim() === security.feishu_app_id
        && !feishuAppSecret.trim()
      )
    ) {
      return;
    }

    securitySavingRef.current = true;
    setSecuritySaving(true);
    setSecuritySaved(false);
    try {
      const value = await gatewayApi.setSecuritySettings({
        feishu_app_id: feishuAppId.trim(),
        feishu_app_secret: feishuAppSecret.trim() || undefined,
        auth_required: authRequired,
      });
      setSecurity(value);
      setFeishuAppSecret("");
      setFeishuAppSecretVisible(false);
      setFeishuAppId(value.feishu_app_id);
      setAuthRequired(value.auth_required);
      setSecuritySaved(true);
      await onChanged();
    } catch (saveError) {
      setSecuritySaved(false);
      onError(errorMessage(saveError));
    } finally {
      securitySavingRef.current = false;
      setSecuritySaving(false);
    }
  }

  function scheduleFeishuCredentialsSave() {
    clearCredentialSaveTimer();
    credentialSaveTimer.current = window.setTimeout(() => {
      credentialSaveTimer.current = null;
      void saveFeishuCredentials();
    }, 150);
  }

  async function saveAuthRequired(nextAuthRequired: boolean) {
    clearCredentialSaveTimer();
    if (!security || securitySavingRef.current) return;
    if (!hasFeishuCredentials()) {
      onError("请先填写飞书 App ID 和飞书 App Secret，再切换账户登录模式");
      return;
    }

    const previousAuthRequired = authRequired;
    securitySavingRef.current = true;
    setSecuritySaving(true);
    setSecuritySaved(false);
    setAuthRequired(nextAuthRequired);
    try {
      const value = await gatewayApi.setSecuritySettings({
        feishu_app_id: feishuAppId.trim(),
        feishu_app_secret: feishuAppSecret.trim() || undefined,
        auth_required: nextAuthRequired,
      });
      setSecurity(value);
      setFeishuAppSecret("");
      setFeishuAppSecretVisible(false);
      setFeishuAppId(value.feishu_app_id);
      setAuthRequired(value.auth_required);
      await onChanged();
      window.location.reload();
    } catch (saveError) {
      setAuthRequired(previousAuthRequired);
      onError(errorMessage(saveError));
    } finally {
      securitySavingRef.current = false;
      setSecuritySaving(false);
    }
  }

  async function toggleFeishuAppSecretVisibility() {
    if (feishuAppSecretVisible) {
      setFeishuAppSecretVisible(false);
      return;
    }
    if (feishuAppSecret.trim() || !security?.feishu_app_secret_configured) {
      setFeishuAppSecretVisible(true);
      return;
    }

    setRevealingFeishuAppSecret(true);
    try {
      const value = await gatewayApi.feishuAppSecret();
      setFeishuAppSecret(value.feishu_app_secret);
      setFeishuAppSecretVisible(true);
    } catch (revealError) {
      onError(errorMessage(revealError));
    } finally {
      setRevealingFeishuAppSecret(false);
    }
  }

  async function regenerateEncryptionKey() {
    setRegeneratingKey(true);
    try {
      const value = await gatewayApi.regenerateDatabaseEncryptionKey();
      setSecurity(value);
      setShowKeyConfirmation(false);
      await onChanged();
    } catch (regenerateError) {
      onError(errorMessage(regenerateError));
    } finally {
      setRegeneratingKey(false);
    }
  }

  async function restoreDefault() {
    setSubmitting(true);
    try {
      const value = await gatewayApi.clearCodexClientVersion();
      setSetting(value);
      setVersion("");
      await onChanged();
    } catch (restoreError) {
      onError(errorMessage(restoreError));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <SettingsPageFrame
      title="管理员设置"
      description="集中管理网关配置。后续管理员设置会继续添加到此标签页。"
      tabs={["安全与登录", "Codex 版本"]}
    >
      <section className="mb-10">
        <div className="mb-4">
          <h2 className="text-base font-bold">安全与账户登录</h2>
          <p className="mt-1 text-xs leading-5 text-slate-400">
            配置保存在 SQLite。更换数据库加密密钥时，已保存的凭据会在同一事务中使用新密钥重新加密。
          </p>
        </div>
        {!security ? (
          <div className="flex min-h-24 items-center justify-center">
            <LoaderCircle className="size-5 animate-spin text-slate-400" />
          </div>
        ) : (
          <div className="space-y-4">
            <div className="grid gap-4 sm:grid-cols-2">
              <div>
                <div className="mb-2 text-[11px] font-bold text-slate-500 dark:text-slate-400">数据库加密密钥</div>
                <div className="flex min-h-11 items-center justify-between gap-3 rounded-xl border border-white/65 bg-white/45 px-3 dark:border-white/8 dark:bg-white/[0.035]">
                  <div className="flex min-w-0 items-center gap-2">
                    <KeyRound className="size-4 shrink-0 text-emerald-500" />
                    <span className="truncate text-xs font-semibold">
                      {security.encryption_key_configured ? "已自动生成并安全保存" : "尚未生成"}
                    </span>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    disabled={securitySaving || regeneratingKey}
                    onClick={() => setShowKeyConfirmation(true)}
                  >
                    <RefreshCw className="size-4" />
                    {security.encryption_key_configured ? "重新生成" : "立即生成"}
                  </Button>
                </div>
                <p className="mt-2 text-[11px] leading-5 text-slate-400">
                  密钥只能由系统随机生成，不支持手动输入。重新生成后会立即替换密钥并重新加密已有凭据。
                </p>
              </div>
            </div>
            <FormField label="飞书 App ID">
              <input
                className="field w-full font-mono text-sm"
                value={feishuAppId}
                placeholder="cli_..."
                disabled={securitySaving}
                onChange={(event) => {
                  clearCredentialSaveTimer();
                  setSecuritySaved(false);
                  setFeishuAppId(event.target.value);
                }}
                onBlur={scheduleFeishuCredentialsSave}
                onKeyDown={(event) => {
                  if (event.key === "Enter") event.currentTarget.blur();
                }}
              />
            </FormField>
            <FormField label={`飞书 App Secret${security.feishu_app_secret_configured ? "（已设置；留空不变）" : ""}`}>
              <div className="flex gap-2">
                <input
                  className="field min-w-0 flex-1 font-mono text-sm"
                  type={feishuAppSecretVisible ? "text" : "password"}
                  value={feishuAppSecret}
                  placeholder={security.feishu_app_secret_configured ? "********" : "App Secret"}
                  disabled={securitySaving || revealingFeishuAppSecret}
                  onChange={(event) => {
                    clearCredentialSaveTimer();
                    setSecuritySaved(false);
                    setFeishuAppSecret(event.target.value);
                  }}
                  onBlur={scheduleFeishuCredentialsSave}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") event.currentTarget.blur();
                  }}
                />
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  disabled={securitySaving || revealingFeishuAppSecret}
                  aria-label={feishuAppSecretVisible ? "隐藏飞书 App Secret" : "显示飞书 App Secret"}
                  onClick={() => void toggleFeishuAppSecretVisibility()}
                >
                  {revealingFeishuAppSecret ? (
                    <LoaderCircle className="size-4 animate-spin" />
                  ) : feishuAppSecretVisible ? (
                    <EyeOff className="size-4" />
                  ) : (
                    <Eye className="size-4" />
                  )}
                </Button>
              </div>
            </FormField>
            <label className="flex items-center justify-between gap-4 rounded-2xl border border-white/65 bg-white/45 p-4 dark:border-white/8 dark:bg-white/[0.035]">
              <span>
                <span className="block text-sm font-semibold">账户登录模式</span>
                <span className="mt-1 block text-xs text-slate-400">开启后管理端需通过飞书登录，网关请求需使用用户 API Key。</span>
              </span>
              <input
                type="checkbox"
                className="size-4"
                checked={authRequired}
                disabled={securitySaving}
                onChange={(event) => void saveAuthRequired(event.target.checked)}
              />
            </label>
            <div
              className="flex min-h-5 items-center justify-end gap-1.5 text-[11px] text-slate-400"
              aria-live="polite"
            >
              {securitySaving ? (
                <>
                  <LoaderCircle className="size-3.5 animate-spin" />
                  正在自动保存
                </>
              ) : securitySaved ? (
                <>
                  <CheckCircle2 className="size-3.5 text-emerald-500" />
                  已自动保存
                </>
              ) : (
                "飞书凭据填写完成后会自动保存"
              )}
            </div>
          </div>
        )}
      </section>
      <section>
        <div className="mb-4">
          <h2 className="text-base font-bold">Codex 版本</h2>
          <p className="mt-1 text-xs leading-5 text-slate-400">
            配置网关向 ChatGPT Codex 模型接口发送的客户端版本号。数据库配置优先于代码默认值。
          </p>
        </div>
        {!setting ? (
        <div className="flex min-h-40 items-center justify-center">
          <LoaderCircle className="size-6 animate-spin text-slate-400" />
        </div>
      ) : (
        <div>
          <form onSubmit={save}>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="rounded-2xl border border-white/65 bg-white/45 p-4 dark:border-white/8 dark:bg-white/[0.035]">
                <div className="eyebrow">代码默认版本</div>
                <div className="mt-2 font-mono text-sm font-bold">{setting.default_version}</div>
              </div>
              <div className="rounded-2xl border border-blue-500/15 bg-blue-500/5 p-4">
                <div className="eyebrow">当前生效版本</div>
                <div className="mt-2 flex items-center gap-2 font-mono text-sm font-bold text-blue-600 dark:text-blue-300">
                  {setting.effective_version}
                  <span className="rounded-full bg-blue-500/10 px-2 py-0.5 font-sans text-[10px]">
                    {setting.is_overridden ? "数据库覆盖" : "代码默认"}
                  </span>
                </div>
              </div>
            </div>

            <div className="mt-5">
              <FormField label="自定义 Codex Client Version">
                <input
                  className="field w-full font-mono text-sm"
                  value={version}
                  placeholder={setting.default_version}
                  disabled={submitting}
                  onChange={(event) => setVersion(event.target.value)}
                />
              </FormField>
              <div className="mt-2 text-[11px] leading-5 text-slate-400">
                保存后写入 SQLite，并自动清理 OpenAI 模型缓存。留空不会保存；需要取消覆盖时使用“恢复代码默认”。
              </div>
            </div>

            <div className="mt-7 flex flex-wrap justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                disabled={!setting.is_overridden || submitting}
                onClick={() => void restoreDefault()}
              >
                <RefreshCw className="size-4" />
                恢复代码默认
              </Button>
              <Button
                type="submit"
                disabled={submitting || !version.trim() || version.trim() === setting.override_version}
              >
                {submitting ? <LoaderCircle className="size-4 animate-spin" /> : <Check className="size-4" />}
                {submitting ? "保存中" : "保存覆盖"}
              </Button>
            </div>
          </form>
        </div>
      )}
      </section>
      {showKeyConfirmation ? (
        <RegenerateEncryptionKeyDialog
          configured={security?.encryption_key_configured ?? false}
          submitting={regeneratingKey}
          onClose={() => setShowKeyConfirmation(false)}
          onConfirm={() => void regenerateEncryptionKey()}
        />
      ) : null}
    </SettingsPageFrame>
  );
}

function AccountSettingsPage({
  authMode,
  onError,
}: {
  authMode: "disabled" | "required";
  onError: (message: string) => void;
}) {
  const [token, setToken] = React.useState<string | null>(null);
  const [creating, setCreating] = React.useState(false);
  const [copied, setCopied] = React.useState(false);
  const [visible, setVisible] = React.useState(false);

  React.useEffect(() => {
    if (authMode !== "required") {
      setToken(null);
      return;
    }
    void authApi.gatewayAccessToken()
      .then((value) => setToken(value.access_token))
      .catch((error) => onError(errorMessage(error)));
  }, [authMode, onError]);

  async function regenerateToken() {
    setCreating(true);
    try {
      const value = await authApi.regenerateAccessToken();
      setToken(value.access_token);
      setVisible(false);
      setCopied(false);
    } catch (error) {
      onError(errorMessage(error));
    } finally {
      setCreating(false);
    }
  }

  return (
    <SettingsPageFrame
      title="账户设置"
      description="管理用于访问 AI网关的个人 API Key。后续账户设置会继续添加到此标签页。"
      tabs={["API Key"]}
    >
      <section>
        <div className="mb-2">
          <h3 className="text-sm font-bold">API Key</h3>
          <p className="mt-1 text-[11px] leading-5 text-slate-400">
            用于账户模式下的 <code>/openai/v1</code> 请求。系统仅保存当前用户的一把 Key，并以加密形式存储。
          </p>
        </div>
        {authMode === "required" ? (
          <>
            <div className="rounded-2xl border border-amber-500/20 bg-amber-500/5 p-4 text-xs leading-5 text-amber-800 dark:text-amber-200">
              重新生成会立即使旧 Key 失效。请同步更新已经写入 Codex 配置的 Key。
            </div>
            {token ? (
              <div className="mt-4">
                <FormField label="当前 API Key">
                  <div className="flex gap-2">
                    <input
                      className="field min-w-0 flex-1 font-mono text-xs"
                      readOnly
                      value={visible ? token : "********"}
                    />
                    <Button
                      type="button"
                      variant="outline"
                      size="icon"
                      aria-label={visible ? "隐藏 API Key" : "显示 API Key"}
                      onClick={() => setVisible((value) => !value)}
                    >
                      {visible ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
                    </Button>
                    <Button
                      type="button"
                      variant="outline"
                      onClick={() => {
                        void copyText(token);
                        setCopied(true);
                        window.setTimeout(() => setCopied(false), 1500);
                      }}
                    >
                      {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
                      {copied ? "已复制" : "复制"}
                    </Button>
                  </div>
                </FormField>
              </div>
            ) : (
              <div className="mt-4 flex min-h-16 items-center justify-center">
                <LoaderCircle className="size-5 animate-spin text-slate-400" />
              </div>
            )}
          </>
        ) : (
          <div className="rounded-2xl border border-blue-500/15 bg-blue-500/5 p-4 text-xs leading-5 text-blue-700 dark:text-blue-300">
            当前服务未启用登录验证，不需要配置个人 API Key。
          </div>
        )}
      </section>
      <div className="mt-7 flex justify-end gap-2">
        {authMode === "required" ? (
          <Button type="button" disabled={creating || !token} onClick={() => void regenerateToken()}>
            {creating ? <LoaderCircle className="size-4 animate-spin" /> : <KeyRound className="size-4" />}
            {creating ? "生成中" : "重新生成"}
          </Button>
          ) : null}
      </div>
    </SettingsPageFrame>
  );
}

function SettingsPageFrame({
  title,
  description,
  tabs,
  children,
}: {
  title: string;
  description: string;
  tabs: string[];
  children: React.ReactNode;
}) {
  return (
    <section className="glass-panel mx-auto max-w-3xl rounded-[26px] p-5 sm:p-7">
      <div className="border-b border-slate-200/70 pb-5 dark:border-white/10">
        <h1 className="text-xl font-bold tracking-[-0.025em]">{title}</h1>
        <p className="mt-1.5 text-xs leading-5 text-slate-500 dark:text-slate-400">{description}</p>
      </div>
      <div className="mt-5 border-b border-slate-200/70 dark:border-white/10" role="tablist">
        {tabs.map((tab) => (
          <button
            key={tab}
            type="button"
            role="tab"
            aria-selected="true"
            className="-mb-px border-b-2 border-blue-600 px-1 pb-3 text-sm font-bold text-blue-600 dark:border-blue-400 dark:text-blue-300"
          >
            {tab}
          </button>
        ))}
      </div>
      <div className="pt-6">{children}</div>
    </section>
  );
}

function RouteTargetSelect({
  label,
  value,
  providers,
  modelsByProvider,
  disabled,
  onChange,
}: {
  label: string;
  value?: RoutingModelTarget;
  providers: GatewayProvider[];
  modelsByProvider: Record<string, GatewayModel[]>;
  disabled: boolean;
  onChange: (value: RoutingModelTarget | undefined) => void;
}) {
  const models = value?.provider_id ? modelsByProvider[value.provider_id] ?? [] : [];
  const providerLabel = (provider: GatewayProvider) =>
    provider.account_email ? `${provider.name} (${provider.account_email})` : provider.name;

  return (
    <div className="grid grid-cols-1 gap-2.5 rounded-[18px] bg-white/60 p-3 sm:grid-cols-2 lg:grid-cols-[5.5rem_minmax(0,1.15fr)_minmax(0,1.15fr)_8rem] lg:items-center lg:gap-3 dark:bg-white/[0.035]">
      <span className="px-1 text-xs font-bold text-slate-600 dark:text-slate-300">{label}</span>
      <label className="min-w-0">
        <span className="mb-1 block px-1 text-[10px] font-bold text-slate-400 lg:hidden">供应商</span>
        <select
          className="field h-10 min-w-0 rounded-2xl px-4 py-1.5 text-xs"
          aria-label={`${label}供应商`}
          value={value?.provider_id ?? ""}
          disabled={disabled}
          onChange={(event) => {
            const providerId = event.target.value;
            onChange(providerId
              ? { provider_id: providerId, model: "", reasoning_effort: value?.reasoning_effort }
              : undefined);
          }}
        >
          <option value="">选择供应商</option>
          {providers.map((provider) => (
            <option key={provider.id} value={provider.id}>{providerLabel(provider)}</option>
          ))}
        </select>
      </label>
      <label className="min-w-0">
        <span className="mb-1 block px-1 text-[10px] font-bold text-slate-400 lg:hidden">模型</span>
        <select
          className="field h-10 min-w-0 rounded-2xl px-4 py-1.5 font-mono text-xs"
          aria-label={`${label}模型`}
          value={value?.model ?? ""}
          disabled={disabled || !value?.provider_id}
          onChange={(event) => {
            if (!value?.provider_id) return;
            onChange({ ...value, model: event.target.value });
          }}
        >
          <option value="">选择模型</option>
          {models.map((model) => (
            <option key={model.id} value={model.id}>{model.id}</option>
          ))}
        </select>
      </label>
      <label className="min-w-0">
        <span className="mb-1 block px-1 text-[10px] font-bold text-slate-400 lg:hidden">推理强度</span>
        <select
          className="field h-10 min-w-0 rounded-2xl px-4 py-1.5 text-xs"
          aria-label={`${label}推理强度`}
          value={value?.reasoning_effort ?? ""}
          disabled={disabled || !value?.provider_id}
          onChange={(event) => {
            if (!value?.provider_id) return;
            onChange({ ...value, reasoning_effort: (event.target.value || undefined) as ReasoningEffort | undefined });
          }}
        >
          <option value="">跟随请求</option>
          <option value="low">低（low）</option>
          <option value="medium">中（medium）</option>
          <option value="high">高（high）</option>
          <option value="xhigh">极高（xhigh）</option>
        </select>
      </label>
    </div>
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

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="glass-panel flex min-h-[420px] flex-col items-center justify-center rounded-[28px] px-6 text-center">
      <div className="flex size-14 items-center justify-center rounded-2xl bg-slate-900 text-white dark:bg-white dark:text-slate-950">
        <Activity className="size-6" />
      </div>
      <h2 className="mt-5 text-xl font-bold">还没有供应商</h2>
      <p className="mt-2 max-w-md text-sm leading-6 text-slate-500">
        添加 OpenAI 兼容 API，或登录、导入 ChatGPT 账户。
      </p>
      <div className="mt-6">
        <Button onClick={onAdd}><Plus className="size-4" />添加供应商</Button>
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

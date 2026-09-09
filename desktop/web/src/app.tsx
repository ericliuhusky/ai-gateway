import * as React from "react";
import {
  Activity,
  Bug,
  Check,
  CheckCircle2,
  ChevronDown,
  CircleAlert,
  Copy,
  Gauge,
  KeyRound,
  LoaderCircle,
  Plus,
  RefreshCw,
  RotateCcw,
  Server,
  Trash2,
  UserRound,
  Wrench,
  Play,
  Square,
  X,
} from "lucide-react";

import { gatewayApi } from "./api";
import { Button } from "./components/ui/button";
import { cn } from "./lib/utils";
import type {
  CodexAuthPayload,
  GatewayModel,
  GatewayIssue,
  GatewayProvider,
  CodexUsageRateLimitWindow,
  CodexUsageResponse,
  ReasoningEffort,
  SelectedProvider,
  OpenAiDeviceLoginStart,
} from "./types";

const GATEWAY_ERROR_PREFIX = "AI网关错误：";
const UPSTREAM_ERROR_PREFIX = "上游服务错误：";
type Dialog = "provider" | "delete-provider" | null;
type QuotaMap = Record<string, CodexUsageResponse | undefined>;
type ErrorMap = Record<string, string | undefined>;
function errorMessage(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  return message.startsWith(GATEWAY_ERROR_PREFIX) || message.startsWith(UPSTREAM_ERROR_PREFIX) ? message : `${GATEWAY_ERROR_PREFIX}${message}`;
}
function hasTokenPair(value: unknown): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const token = value as Record<string, unknown>;
  return typeof token.access_token === "string" && token.access_token.trim().length > 0 && typeof token.refresh_token === "string" && token.refresh_token.trim().length > 0;
}
function parseCodexAuthPayload(value: unknown): CodexAuthPayload | null {
  const entries = Array.isArray(value) ? value : [value];
  if (!entries.length) return null;
  const supported = entries.every((entry) => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) return false;
    const record = entry as Record<string, unknown>;
    return hasTokenPair(record.tokens) || hasTokenPair(record);
  });
  return supported ? value as CodexAuthPayload : null;
}
const NINEBOT_PRIVATE_DEPLOYMENT_PRESET = { name: "九号私有部署", baseUrl: "https://ai-service.segway-ninebot.com/v1" };
function remaining(window: CodexUsageRateLimitWindow) { return Math.min(100, Math.max(0, 100 - window.used_percent)); }
function quotaTone(value: number) { return value <= 15 ? "danger" : value <= 35 ? "warning" : "good"; }
function resetLabel(window: CodexUsageRateLimitWindow) {
  if (!window.reset_at) return null;
  const date = new Date(window.reset_at * 1000);
  const time = `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
  if (window.limit_window_seconds === 5 * 60 * 60) return `${time} 重置`;
  const weekdays = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
  return `${date.getMonth() + 1}月${date.getDate()}日 ${weekdays[date.getDay()]} ${time} 重置`;
}
function authExpiryLabel(timestamp?: number) {
  if (!timestamp) return "未知";
  const date = new Date(timestamp * 1000);
  const expired = timestamp * 1000 <= Date.now();
  const weekdays = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
  const time = `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
  return `${date.getMonth() + 1}月${date.getDate()}日 ${weekdays[date.getDay()]} ${time} ${expired ? "已过期" : "到期"}`;
}
function copyText(text: string) { return navigator.clipboard.writeText(text); }
export function App() { return <GatewayDashboard />; }
export function GatewayDashboard() {
  const [providers, setProviders] = React.useState<GatewayProvider[]>([]);
  const [selected, setSelected] = React.useState<SelectedProvider>({ updated_at: 0 });
  const [gatewayIssues, setGatewayIssues] = React.useState<GatewayIssue[]>([]);
  const [quotas, setQuotas] = React.useState<QuotaMap>({});
  const [quotaErrors, setQuotaErrors] = React.useState<ErrorMap>({});
  const [loadingQuotas, setLoadingQuotas] = React.useState<Set<string>>(new Set());
  const [loading, setLoading] = React.useState(true);
  const [dialog, setDialog] = React.useState<Dialog>(null);
  const [providerToDelete, setProviderToDelete] = React.useState<GatewayProvider | null>(null);
  const [issuesOpen, setIssuesOpen] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [deleting, setDeleting] = React.useState<Set<string>>(new Set());
  const [refreshingProviders, setRefreshingProviders] = React.useState<Set<string>>(new Set());
  const loadQuotas = React.useCallback(async (items: GatewayProvider[], visibleLoading = true) => {
    const ids = items.filter((item) => item.auth_mode === "account").map((item) => item.id);
    if (!ids.length) return;
    if (visibleLoading) setLoadingQuotas((current) => new Set([...current, ...ids]));
    await Promise.all(ids.map(async (id) => {
      try { const quota = await gatewayApi.quota(id); setQuotas((current) => ({ ...current, [id]: quota })); setQuotaErrors((current) => ({ ...current, [id]: undefined })); }
      catch (quotaError) { setQuotaErrors((current) => ({ ...current, [id]: errorMessage(quotaError) })); }
      finally { setLoadingQuotas((current) => { const next = new Set(current); next.delete(id); return next; }); }
    }));
  }, []);
  const refresh = React.useCallback(async () => {
    setLoading(true);
    try {
      const [providerList, route, issues] = await Promise.all([gatewayApi.providers(), gatewayApi.selectedProvider(), gatewayApi.gatewayIssues(200)]);
      const sorted = [...providerList].sort((a, b) => a.name.localeCompare(b.name));
      setProviders(sorted); setSelected(route); setGatewayIssues(issues); setError(null); void loadQuotas(sorted);
      return sorted;
    } catch (loadError) { setError(errorMessage(loadError)); } finally { setLoading(false); }
  }, [loadQuotas]);
  React.useEffect(() => { void refresh(); }, [refresh]);
  React.useEffect(() => { const timer = window.setInterval(() => void loadQuotas(providers, false), 60000); return () => window.clearInterval(timer); }, [loadQuotas, providers]);
  async function selectProvider(provider: GatewayProvider) {
    if (provider.id === selected.provider_id || deleting.has(provider.id)) return;
    setSelected((current) => ({ ...current, provider_id: provider.id, selected_model: undefined, selected_reasoning_effort: undefined }));
    try { setSelected(await gatewayApi.selectProvider(provider.id)); await loadQuotas([provider]); } catch (selectionError) { setError(errorMessage(selectionError)); await refresh(); }
  }
  async function handleProviderCreated() {
    const shouldSelectFirst = providers.length === 0;
    setDialog(null);
    const nextProviders = await refresh();
    if (shouldSelectFirst && nextProviders?.[0]) {
      await selectProvider(nextProviders[0]);
    }
  }
  function requestDeleteProvider(provider: GatewayProvider) { if (!deleting.has(provider.id)) { setProviderToDelete(provider); setDialog("delete-provider"); } }
  async function refreshProvider(provider: GatewayProvider) {
    if (refreshingProviders.has(provider.id)) return;
    setRefreshingProviders((current) => new Set(current).add(provider.id));
    try { await gatewayApi.refreshProvider(provider.id); await refresh(); }
    catch (refreshError) { setError(errorMessage(refreshError)); }
    finally { setRefreshingProviders((current) => { const next = new Set(current); next.delete(provider.id); return next; }); }
  }
  async function confirmDeleteProvider() {
    const provider = providerToDelete; if (!provider || deleting.has(provider.id)) return;
    setDeleting((current) => new Set(current).add(provider.id));
    try { await gatewayApi.deleteProvider(provider.id); setProviderToDelete(null); setDialog(null); await refresh(); }
    catch (deleteError) { setError(errorMessage(deleteError)); }
    finally { setDeleting((current) => { const next = new Set(current); next.delete(provider.id); return next; }); }
  }
  return <div className="min-h-screen min-w-0">
    <main className="mx-auto max-w-[1480px] px-3 py-4 sm:px-8 sm:py-8">{loading ? <LoadingState /> : <><section><div className="mb-3 flex flex-wrap items-center gap-3 px-1"><h2 className="text-xs font-bold uppercase tracking-[0.12em] text-slate-500 dark:text-slate-400">AI 网关</h2><Button className="ml-auto" variant="outline" size="sm" onClick={() => setDialog("provider")}><Plus className="size-3.5" />添加供应商</Button></div><DefaultRouteSection providers={providers} selected={selected} onChanged={refresh} onError={setError} onOpenIssues={() => setIssuesOpen(true)} /></section>{providers.length === 0 ? <div className="mt-8"><EmptyState onAdd={() => setDialog("provider")} /></div> : <div className="mt-8"><ProviderSection title="供应商" providers={providers} selectedId={selected.provider_id} quotas={quotas} quotaErrors={quotaErrors} loadingQuotas={loadingQuotas} deleting={deleting} refreshingProviders={refreshingProviders} onSelect={selectProvider} onDelete={requestDeleteProvider} onRefreshQuota={(provider) => void loadQuotas([provider])} onRefreshProvider={(provider) => void refreshProvider(provider)} /></div>}</>}</main>
    {issuesOpen ? <GatewayIssueDialog issues={gatewayIssues} onChanged={async () => setGatewayIssues(await gatewayApi.gatewayIssues(200))} onError={setError} onClose={() => setIssuesOpen(false)} /> : null}{error ? <ErrorToast message={error} onClose={() => setError(null)} /> : null}{dialog === "provider" ? <ProviderDialog onClose={() => setDialog(null)} onCreated={handleProviderCreated} onError={setError} /> : null}{dialog === "delete-provider" && providerToDelete ? <DeleteProviderDialog provider={providerToDelete} deleting={deleting.has(providerToDelete.id)} onClose={() => { if (!deleting.has(providerToDelete.id)) { setProviderToDelete(null); setDialog(null); } }} onConfirm={() => void confirmDeleteProvider()} /> : null}
  </div>;
}
function DefaultCodexGatewayControl({ onError, onOpenIssues }: { onError: (message: string) => void; onOpenIssues: () => void }) {
  const [started, setStarted] = React.useState(false);
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [restartingChatGpt, setRestartingChatGpt] = React.useState(false);

  React.useEffect(() => {
    void gatewayApi.codexGatewayStatus()
      .then((status) => setStarted(status.started))
      .catch((statusError) => onError(errorMessage(statusError)))
      .finally(() => setLoading(false));
  }, [onError]);

  async function toggle() {
    if (busy || loading) return;
    setBusy(true);
    try {
      if (started) await gatewayApi.stopCodexGateway();
      else await gatewayApi.startCodexGateway();
      setStarted(!started);
    } catch (toggleError) {
      onError(errorMessage(toggleError));
    } finally {
      setBusy(false);
    }
  }

  async function restartChatGpt() {
    if (restartingChatGpt) return;
    setRestartingChatGpt(true);
    try {
      await gatewayApi.restartChatGpt();
    } catch (restartError) {
      onError(errorMessage(restartError));
    } finally {
      setRestartingChatGpt(false);
    }
  }

  return (
    <div className="flex items-center gap-2">
      <Button
        type="button"
        variant="outline"
        size="icon"
        disabled={loading || busy}
        title={started ? "停止 AI 网关并恢复默认 Codex 配置" : "启动 AI 网关"}
        aria-label={started ? "停止 AI 网关" : "启动 AI 网关"}
        onClick={() => void toggle()}
      >
        {loading || busy ? (
          <LoaderCircle className="size-4 animate-spin" />
        ) : started ? (
          <Square className="size-3.5 fill-current" />
        ) : (
          <Play className="size-4 fill-current" />
        )}
      </Button>
      <Button
        type="button"
        variant="outline"
        size="icon"
        disabled={restartingChatGpt}
        title="重新打开 ChatGPT.app"
        aria-label="重新打开 ChatGPT.app"
        onClick={() => void restartChatGpt()}
      >
        {restartingChatGpt ? <LoaderCircle className="size-4 animate-spin" /> : <RotateCcw className="size-4" />}
      </Button>
      <Button
        type="button"
        variant="outline"
        size="icon"
        title="查看网关问题"
        aria-label="查看网关问题"
        onClick={onOpenIssues}
      >
        <Bug className="size-4" />
      </Button>
    </div>
  );
}

function DefaultRouteSection({ providers, selected, onChanged, onError, onOpenIssues }: { providers: GatewayProvider[]; selected: SelectedProvider; onChanged: () => Promise<unknown>; onError: (message: string) => void; onOpenIssues: () => void }) {
  const [models, setModels] = React.useState<GatewayModel[]>([]); const [loadingModels, setLoadingModels] = React.useState(false); const [saving, setSaving] = React.useState(false);
  const provider = providers.find((item) => item.id === selected.provider_id);
  React.useEffect(() => { if (!selected.provider_id) { setModels([]); return; } let cancelled = false; setLoadingModels(true); void gatewayApi.models(selected.provider_id).then((items) => { if (!cancelled) setModels([...items].sort((a,b) => a.id.localeCompare(b.id))); }).catch((e) => onError(errorMessage(e))).finally(() => { if (!cancelled) setLoadingModels(false); }); return () => { cancelled = true; }; }, [selected.provider_id, onError]);
  async function run(action: () => Promise<unknown>) { setSaving(true); try { await action(); await onChanged(); } catch (e) { onError(errorMessage(e)); } finally { setSaving(false); } }
  return <article className="glass-panel flex flex-col gap-4 rounded-[22px] p-3.5 sm:p-4 lg:flex-row lg:items-center lg:gap-5"><DefaultCodexGatewayControl onError={onError} onOpenIssues={onOpenIssues} /><div className="flex min-w-0 flex-1 flex-col gap-3 sm:flex-row"><label className="min-w-0 flex-1"><span className="eyebrow">模型</span><select className="field mt-1 h-9 w-full font-mono text-xs font-semibold" value={selected.selected_model ?? ""} disabled={saving || loadingModels || !provider} onChange={(e) => void run(() => e.target.value ? gatewayApi.selectModel(e.target.value) : gatewayApi.clearSelectedModel())}><option value="">跟随请求模型</option>{models.map((item) => <option key={item.id} value={item.id}>{item.id}</option>)}</select></label><label className="min-w-0 flex-1"><span className="eyebrow">推理强度</span><select className="field mt-1 h-9 w-full text-xs font-semibold" value={selected.selected_reasoning_effort ?? ""} disabled={saving || !provider} onChange={(e) => void run(() => e.target.value ? gatewayApi.selectReasoningEffort(e.target.value as ReasoningEffort) : gatewayApi.clearSelectedReasoningEffort())}><option value="">跟随请求</option><option value="low">低（low）</option><option value="medium">中（medium）</option><option value="high">高（high）</option><option value="xhigh">极高（xhigh）</option></select></label></div></article>;
}

function ProviderSection(props: {
  title: string;
  providers: GatewayProvider[];
  selectedId?: string;
  quotas: QuotaMap;
  quotaErrors: ErrorMap;
  loadingQuotas: Set<string>;
  deleting: Set<string>;
  refreshingProviders: Set<string>;
  onSelect: (provider: GatewayProvider) => void;
  onDelete: (provider: GatewayProvider) => void;
  onRefreshQuota: (provider: GatewayProvider) => void;
  onRefreshProvider: (provider: GatewayProvider) => void;
}) {
  if (!props.providers.length) return null;
  return (
    <section>
      <div className="mb-3 flex flex-wrap items-center gap-2 px-1 sm:gap-3">
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
            refreshingAccount={props.refreshingProviders.has(provider.id)}
            onSelect={() => props.onSelect(provider)}
            onDelete={() => props.onDelete(provider)}
            onRefreshQuota={() => props.onRefreshQuota(provider)}
            onRefreshProvider={() => props.onRefreshProvider(provider)}
          />
        ))}
      </div>
    </section>
  );
}

function GatewayIssueDialog({
  issues,
  onChanged,
  onError,
  onClose,
}: {
  issues: GatewayIssue[];
  onChanged: () => Promise<void>;
  onError: (message: string) => void;
  onClose: () => void;
}) {
  React.useEffect(() => {
    const handler = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/25 p-3 backdrop-blur-sm sm:p-6"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="dialog-panel max-h-[calc(100dvh-6rem)] w-full max-w-3xl overflow-y-auto rounded-[22px] p-4 sm:max-h-[calc(100vh-8rem)] sm:rounded-[26px] sm:p-5"
        role="dialog"
        aria-modal="true"
        aria-label="网关问题"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <GatewayIssueSection issues={issues} onChanged={onChanged} onError={onError} onClose={onClose} />
      </div>
    </div>
  );
}


function GatewayIssueSection({
  issues,
  onChanged,
  onError,
  onClose,
}: {
  issues: GatewayIssue[];
  onChanged: () => Promise<void>;
  onError: (message: string) => void;
  onClose?: () => void;
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
          variant="outline"
          size="sm"
          disabled={!issues.length || clearing}
          onClick={() => void clearIssues()}
        >
          {clearing ? <LoaderCircle className="size-3.5 animate-spin" /> : <Trash2 className="size-3.5" />}
          一键清空
        </Button>
        {onClose ? (
          <Button className="ml-auto" variant="outline" size="icon" title="关闭" aria-label="关闭" onClick={onClose}>
            <X className="size-3.5" />
          </Button>
        ) : null}
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
              <article key={issue.id} className="p-4 sm:p-5">
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
                        {issue.provider_name}
                      </span>
                      <span className="text-[10px] text-slate-400">{formatIssueTime(issue.created_at)}</span>
                    </div>
                    <div className="mt-2 break-words text-xs font-medium leading-5 text-red-600 dark:text-red-400">
                      {issue.error_message}
                    </div>
                    <div className="mt-1 truncate font-mono text-[10px] text-slate-400" title={issue.upstream_url}>
                      {issue.upstream_url}
                    </div>
                    <details className="mt-3 rounded-xl bg-slate-900/[0.035] px-3 py-2 text-[10px] dark:bg-white/[0.05]">
                      <summary className="cursor-pointer font-semibold text-slate-500 dark:text-slate-400">
                        查看上游原始返回
                      </summary>
                      <IssuePayload
                        label={`上游原始返回${issue.upstream_response_truncated ? "（已截断）" : ""}`}
                        value={issue.upstream_response || "（上游返回空响应体）"}
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
        仅记录本机的上游连接失败、非 2xx、响应读取失败和流中断；最多保留 200 条，单个请求和响应各最多 128 KiB。
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

function formatIssueTime(timestamp: number) {
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
  refreshingAccount,
  onSelect,
  onDelete,
  onRefreshQuota,
  onRefreshProvider,
}: {
  provider: GatewayProvider;
  selected: boolean;
  quota?: CodexUsageResponse;
  quotaError?: string;
  loadingQuota: boolean;
  deleting: boolean;
  refreshingAccount: boolean;
  onSelect: () => void;
  onDelete: () => void;
  onRefreshQuota: () => void;
  onRefreshProvider: () => void;
}) {
  return (
    <article
      className={cn(
        "provider-card group relative flex min-h-[220px] cursor-pointer flex-col rounded-[24px] p-4 sm:min-h-[252px] sm:p-5",
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
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {deleting ? (
            <LoaderCircle className="size-5 animate-spin text-slate-400" />
          ) : (
            <button
              className={cn(
                "flex size-8 items-center justify-center rounded-xl text-slate-400 opacity-0 transition hover:bg-black/5 hover:text-red-500 group-hover:opacity-100 focus-visible:opacity-100 dark:hover:bg-white/8",
              )}
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
          {selected ? <CheckCircle2 className="size-5 text-blue-500" /> : null}
        </div>
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
        <div className="mt-auto">
          <AuthPanel
            expiresAt={provider.account_expires_at}
            refreshing={refreshingAccount}
            disabled={false}
            onRefresh={onRefreshProvider}
          />
          <QuotaPanel
            quota={quota}
            error={quotaError}
            loading={loadingQuota}
            onRefresh={onRefreshQuota}
          />
        </div>
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
  quota?: CodexUsageResponse;
  error?: string;
  loading: boolean;
  onRefresh: () => void;
}) {
  const primary = quota?.rate_limit?.primary_window;
  const secondary = quota?.rate_limit?.secondary_window;

  return (
    <ControlPanel icon={Gauge} title="额度窗口" actionLabel="刷新额度" loading={loading} onRefresh={onRefresh}>
      {loading && !quota ? (
        <div className="flex h-[68px] items-center justify-center text-xs text-slate-400">同步中…</div>
      ) : error ? (
        <div className="line-clamp-2 text-xs leading-5 text-red-500">{error}</div>
      ) : primary || secondary ? (
        <div className="space-y-2.5">
          {primary ? <QuotaRow title={windowTitle(primary, "五小时窗口")} window={primary} /> : null}
          {secondary ? <QuotaRow title={windowTitle(secondary, "周窗口")} window={secondary} /> : null}
          {quota ? <QuotaFootnote quota={quota} /> : null}
        </div>
      ) : (
        <div className="text-xs text-slate-400">还没有拿到额度信息</div>
      )}
    </ControlPanel>
  );
}

function AuthPanel({
  expiresAt,
  refreshing,
  disabled,
  onRefresh,
}: {
  expiresAt?: number;
  refreshing: boolean;
  disabled: boolean;
  onRefresh: () => void;
}) {
  return (
    <ControlPanel icon={KeyRound} title="Auth 到期时间" actionLabel="刷新授权" loading={refreshing} disabled={disabled} onRefresh={onRefresh} className="mb-3">
      <div className="text-[11px] font-semibold text-slate-600 dark:text-slate-300">{authExpiryLabel(expiresAt)}</div>
    </ControlPanel>
  );
}

function ControlPanel({
  icon: Icon,
  title,
  actionLabel,
  loading,
  disabled = false,
  onRefresh,
  className,
  children,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  actionLabel: string;
  loading: boolean;
  disabled?: boolean;
  onRefresh: () => void;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={cn("rounded-2xl border border-white/65 bg-white/45 p-3 dark:border-white/8 dark:bg-white/[0.035]", className)}>
      <div className="mb-2.5 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <Icon className="size-3.5 text-slate-400" />
          <span className="text-xs font-bold text-slate-500 dark:text-slate-400">{title}</span>
        </div>
        <button
          type="button"
          title={actionLabel}
          className="inline-flex shrink-0 items-center gap-1.5 rounded-lg px-2 py-1.5 text-[10px] font-semibold text-slate-500 transition hover:bg-black/5 hover:text-slate-800 disabled:cursor-not-allowed disabled:opacity-50 dark:hover:bg-white/10 dark:hover:text-white"
          disabled={loading || disabled}
          onClick={(event) => {
            event.stopPropagation();
            onRefresh();
          }}
        >
          {loading ? <LoaderCircle className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
          {actionLabel}
        </button>
      </div>
      {children}
    </div>
  );
}

function windowTitle(window: CodexUsageRateLimitWindow, fallback: string) {
  const windowMinutes = Math.round(window.limit_window_seconds / 60);
  if (!windowMinutes) return fallback;
  if (windowMinutes === 300) return "五小时窗口";
  if (windowMinutes >= 7 * 24 * 60) return "周窗口";
  if (windowMinutes % (24 * 60) === 0) {
    return `${windowMinutes / (24 * 60)} 天窗口`;
  }
  if (windowMinutes % 60 === 0) {
    return `${windowMinutes / 60} 小时窗口`;
  }
  return `${windowMinutes} 分钟窗口`;
}

function QuotaRow({ title, window }: { title: string; window: CodexUsageRateLimitWindow }) {
  const value = remaining(window);
  const tone = quotaTone(value);
  return (
    <div>
      <div className="mb-1 flex items-center gap-2 text-[11px] font-semibold text-slate-600 dark:text-slate-300">
        <span>{title}</span>
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
      {resetLabel(window) ? <div className="mt-1 text-[11px] font-semibold text-slate-600 dark:text-slate-300">{resetLabel(window)}</div> : null}
    </div>
  );
}

function QuotaFootnote({ quota }: { quota: CodexUsageResponse }) {
  const unlimited = quota.credits?.unlimited;
  const balance = quota.credits?.balance;
  const parts = [
    unlimited ? "账户余额无限" : balance ? `余额 ${balance}` : null,
    quota.plan_type ? `Plan ${quota.plan_type}` : null,
  ].filter(Boolean);
  return parts.length ? <div className="text-[10px] text-slate-400">{parts.join(" · ")}</div> : null;
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
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-slate-950/30 p-2 sm:items-center sm:p-4 backdrop-blur-sm" onMouseDown={onClose}>
      <div
        className={cn(
          "dialog-panel max-h-[calc(100dvh-1rem)] w-full overflow-y-auto rounded-[22px] p-4 sm:max-h-[calc(100vh-2rem)] sm:rounded-[26px] sm:p-7",
          wide ? "max-w-4xl" : "max-w-xl",
        )}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="mb-5 flex items-start gap-3 sm:mb-6 sm:gap-4">
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
  onCreated,
  onError,
}: {
  onClose: () => void;
  onCreated: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [providerType, setProviderType] = React.useState<"api" | "account">("account");
  const [apiTabVisited, setApiTabVisited] = React.useState(false);

  return (
    <DialogFrame
      title="添加供应商"
      description="选择使用 API Key 接入 Responses API，或添加 ChatGPT 账户。"
      onClose={onClose}
    >
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
      <div className={providerType === "account" ? undefined : "hidden"}>
        <ProviderAuthForm onClose={onClose} onCreated={onCreated} onError={onError} />
      </div>
      {apiTabVisited ? (
        <div className={providerType === "api" ? undefined : "hidden"}>
          <ApiProviderForm onClose={onClose} onCreated={onCreated} onError={onError} />
        </div>
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
            setBaseUrl(event.target.value);
          }}
          placeholder="https://api.example.com/v1"
        />
      </FormField>
      <FormField label="API Key">
        <input className="field font-mono text-xs" type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} placeholder="sk-..." />
      </FormField>
      <DialogActions onClose={onClose} disabled={!valid || submitting} submitting={submitting} label="创建供应商" />
    </form>
  );
}

function ProviderAuthForm({
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
      await gatewayApi.importProvider(parsed);
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
            setLoginError(result.error ? errorMessage(result.error) : `${GATEWAY_ERROR_PREFIX}账户登录失败`);
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
        通过 OpenAI 官方设备授权登录；凭据会直接保存在本机数据库中。
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
              <div>{loginError ?? `${GATEWAY_ERROR_PREFIX}无法创建账户登录。`}</div>
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
                <div className="mt-2 flex flex-wrap items-center gap-3">
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


function DeleteProviderDialog({
  provider,
  deleting,
  onClose,
  onConfirm,
}: {
  provider: GatewayProvider;
  deleting: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const providerName = provider.auth_mode === "account"
    ? (provider.account_email ?? provider.name)
    : provider.name;

  return (
    <DialogFrame
      title={`删除供应商：${providerName}`}
      description="删除后将清除该供应商的模型缓存及关联路由配置。"
      onClose={onClose}
    >
      <div className="space-y-5">
        <div className="rounded-2xl border border-amber-500/20 bg-amber-500/5 p-4 text-xs leading-5 text-amber-700 dark:text-amber-300">
          {provider.auth_mode === "account"
            ? "删除该供应商时，其本机登录信息也会一并删除。"
            : "此操作不可撤销；需要时可重新添加该供应商。"}
        </div>
        <div className="flex flex-wrap justify-end gap-2">
          <Button type="button" variant="outline" disabled={deleting} onClick={onClose}>取消</Button>
          <Button type="button" disabled={deleting} onClick={onConfirm} className="bg-red-600 text-white hover:bg-red-700">
            {deleting ? <LoaderCircle className="size-4 animate-spin" /> : <Trash2 className="size-4" />}
            {deleting ? "删除中" : "确认删除供应商"}
          </Button>
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
    <div className="mt-7 flex flex-col-reverse justify-end gap-2 sm:flex-row">
      <Button className="w-full sm:w-auto" type="button" variant="outline" onClick={onClose}>取消</Button>
      <Button className="w-full sm:w-auto" type="submit" disabled={disabled}>
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
        <div className="mt-3 text-xs font-semibold text-slate-400">正在连接本机 Gateway</div>
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

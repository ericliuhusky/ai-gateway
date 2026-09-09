import * as React from "react";

import { gatewayApi } from "../api";
import type { GatewayProvider, SelectedProvider } from "../types";
import {
  MODEL_CACHE_STORAGE_KEY,
  QUOTA_CACHE_STORAGE_KEY,
  errorMessage,
  quotasFromCache,
  readLocalCache,
  writeLocalCache,
  type Dialog,
  type ErrorMap,
  type ModelCache,
  type QuotaCache,
  type QuotaMap,
} from "./dashboard";

export function useGatewayDashboard() {
  const [providers, setProviders] = React.useState<GatewayProvider[]>([]);
  const [selected, setSelected] = React.useState<SelectedProvider>({ updated_at: 0 });
  const quotaCacheRef = React.useRef<QuotaCache>(readLocalCache(QUOTA_CACHE_STORAGE_KEY, {}));
  const quotaRequestsRef = React.useRef(new Map<string, Promise<void>>());
  const [quotas, setQuotas] = React.useState<QuotaMap>(() => quotasFromCache(quotaCacheRef.current));
  const [quotaErrors, setQuotaErrors] = React.useState<ErrorMap>({});
  const [loadingQuotas, setLoadingQuotas] = React.useState<Set<string>>(new Set());
  const [loading, setLoading] = React.useState(true);
  const [dialog, setDialog] = React.useState<Dialog>(null);
  const [providerToDelete, setProviderToDelete] = React.useState<GatewayProvider | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [deleting, setDeleting] = React.useState<Set<string>>(new Set());
  const [refreshingProviders, setRefreshingProviders] = React.useState<Set<string>>(new Set());

  const prefetchModels = React.useCallback(async (items: GatewayProvider[]) => {
    await Promise.all(items.map(async (provider) => {
      try {
        const fetched = await gatewayApi.models(provider.id);
        const models = [...fetched].sort((a, b) => a.id.localeCompare(b.id));
        const cache = readLocalCache<ModelCache>(MODEL_CACHE_STORAGE_KEY, {});
        cache[provider.id] = { models, fetchedAt: Date.now() };
        writeLocalCache(MODEL_CACHE_STORAGE_KEY, cache);
      } catch (modelError) {
        setError(errorMessage(modelError));
      }
    }));
  }, []);

  const loadQuotas = React.useCallback(async (
    items: GatewayProvider[],
    forceRefresh = false,
    visibleLoading = true,
  ) => {
    const ids = items.filter((item) => item.auth_mode === "account").map((item) => item.id);
    const requestIds = ids.filter((id) => forceRefresh || !quotaCacheRef.current[id]);
    if (!requestIds.length) return;

    const fetchQuota = (id: string) => {
      const inFlight = quotaRequestsRef.current.get(id);
      if (inFlight) return inFlight;
      if (visibleLoading) setLoadingQuotas((current) => new Set([...current, id]));

      const request = gatewayApi.quota(id)
        .then((quota) => {
          quotaCacheRef.current = { ...quotaCacheRef.current, [id]: { quota, fetchedAt: Date.now() } };
          writeLocalCache(QUOTA_CACHE_STORAGE_KEY, quotaCacheRef.current);
          setQuotas((current) => ({ ...current, [id]: quota }));
          setQuotaErrors((current) => ({ ...current, [id]: undefined }));
        })
        .catch((quotaError) => {
          setQuotaErrors((current) => ({ ...current, [id]: errorMessage(quotaError) }));
        })
        .finally(() => {
          quotaRequestsRef.current.delete(id);
          setLoadingQuotas((current) => {
            const next = new Set(current);
            next.delete(id);
            return next;
          });
        });
      quotaRequestsRef.current.set(id, request);
      return request;
    };

    await Promise.all(requestIds.map(fetchQuota));
  }, []);

  const refresh = React.useCallback(async () => {
    setLoading(true);
    try {
      const [providerList, route] = await Promise.all([
        gatewayApi.providers(),
        gatewayApi.selectedProvider(),
      ]);
      const sorted = [...providerList].sort((a, b) => a.name.localeCompare(b.name));
      const accountIds = new Set(
        sorted.filter((provider) => provider.auth_mode === "account").map((provider) => provider.id),
      );
      quotaCacheRef.current = Object.fromEntries(
        Object.entries(quotaCacheRef.current).filter(([id]) => accountIds.has(id)),
      );
      writeLocalCache(QUOTA_CACHE_STORAGE_KEY, quotaCacheRef.current);

      const modelCache = readLocalCache<ModelCache>(MODEL_CACHE_STORAGE_KEY, {});
      const providerIds = new Set(sorted.map((provider) => provider.id));
      writeLocalCache(
        MODEL_CACHE_STORAGE_KEY,
        Object.fromEntries(Object.entries(modelCache).filter(([id]) => providerIds.has(id))),
      );
      setQuotas(quotasFromCache(quotaCacheRef.current));
      setProviders(sorted);
      setSelected(route);
      setError(null);
      return sorted;
    } catch (loadError) {
      setError(errorMessage(loadError));
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => { void refresh(); }, [refresh]);
  React.useEffect(() => {
    const timer = window.setInterval(() => void loadQuotas(providers, true, false), 60000);
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
    try {
      setSelected(await gatewayApi.selectProvider(provider.id));
      await loadQuotas([provider]);
    } catch (selectionError) {
      setError(errorMessage(selectionError));
      await refresh();
    }
  }

  async function handleProviderCreated() {
    const existingProviderIds = new Set(providers.map((provider) => provider.id));
    const shouldSelectFirst = providers.length === 0;
    setDialog(null);
    const nextProviders = await refresh();
    const newProviders = nextProviders?.filter((provider) => !existingProviderIds.has(provider.id)) ?? [];
    await Promise.all([prefetchModels(newProviders), loadQuotas(newProviders, true)]);
    if (shouldSelectFirst && nextProviders?.[0]) await selectProvider(nextProviders[0]);
  }

  function requestDeleteProvider(provider: GatewayProvider) {
    if (!deleting.has(provider.id)) {
      setProviderToDelete(provider);
      setDialog("delete-provider");
    }
  }

  async function refreshProvider(provider: GatewayProvider) {
    if (refreshingProviders.has(provider.id)) return;
    setRefreshingProviders((current) => new Set(current).add(provider.id));
    try {
      await gatewayApi.refreshProvider(provider.id);
      await refresh();
    } catch (refreshError) {
      setError(errorMessage(refreshError));
    } finally {
      setRefreshingProviders((current) => {
        const next = new Set(current);
        next.delete(provider.id);
        return next;
      });
    }
  }

  async function confirmDeleteProvider() {
    const provider = providerToDelete;
    if (!provider || deleting.has(provider.id)) return;
    setDeleting((current) => new Set(current).add(provider.id));
    try {
      await gatewayApi.deleteProvider(provider.id);
      setProviderToDelete(null);
      setDialog(null);
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

  return {
    providers,
    selected,
    quotas,
    quotaErrors,
    loadingQuotas,
    loading,
    dialog,
    setDialog,
    providerToDelete,
    setProviderToDelete,
    error,
    setError,
    deleting,
    refreshingProviders,
    refresh,
    loadQuotas,
    selectProvider,
    handleProviderCreated,
    requestDeleteProvider,
    refreshProvider,
    confirmDeleteProvider,
  };
}

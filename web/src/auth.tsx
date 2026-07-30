import * as React from "react";

import { authApi, type GatewayUser } from "./api";

type AuthContextValue = {
  loading: boolean;
  user: GatewayUser | null;
  authMode: "disabled" | "required";
  feishuLoginConfigured: boolean;
  refresh: () => Promise<void>;
  logout: () => Promise<void>;
};

const AuthContext = React.createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [loading, setLoading] = React.useState(true);
  const [user, setUser] = React.useState<GatewayUser | null>(null);
  const [feishuLoginConfigured, setFeishuLoginConfigured] = React.useState(false);
  const [authMode, setAuthMode] = React.useState<"disabled" | "required">("disabled");

  const refresh = React.useCallback(async () => {
    const status = await authApi.status();
    setFeishuLoginConfigured(status.feishu_login_configured);
    setAuthMode(status.mode);
    try {
      const me = await authApi.me();
      setUser(me.user);
    } catch {
      setUser(null);
    }
  }, []);

  React.useEffect(() => {
    void refresh().finally(() => setLoading(false));
  }, [refresh]);

  const logout = React.useCallback(async () => {
    await authApi.logout();
    if (authMode === "disabled") {
      await refresh();
      return;
    }
    setUser(null);
  }, [authMode, refresh]);

  return <AuthContext.Provider value={{ loading, user, authMode, feishuLoginConfigured, refresh, logout }}>{children}</AuthContext.Provider>;
}

export function useAuth() {
  const value = React.useContext(AuthContext);
  if (!value) throw new Error("useAuth must be used within AuthProvider");
  return value;
}

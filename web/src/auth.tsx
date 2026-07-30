import * as React from "react";

import { authApi, type GatewayUser } from "./api";

type AuthContextValue = {
  loading: boolean;
  user: GatewayUser | null;
  feishuLoginConfigured: boolean;
  refresh: () => Promise<void>;
  logout: () => Promise<void>;
};

const AuthContext = React.createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [loading, setLoading] = React.useState(true);
  const [user, setUser] = React.useState<GatewayUser | null>(null);
  const [feishuLoginConfigured, setFeishuLoginConfigured] = React.useState(false);

  const refresh = React.useCallback(async () => {
    const status = await authApi.status();
    setFeishuLoginConfigured(status.feishu_login_configured);
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
    setUser(null);
  }, []);

  return <AuthContext.Provider value={{ loading, user, feishuLoginConfigured, refresh, logout }}>{children}</AuthContext.Provider>;
}

export function useAuth() {
  const value = React.useContext(AuthContext);
  if (!value) throw new Error("useAuth must be used within AuthProvider");
  return value;
}

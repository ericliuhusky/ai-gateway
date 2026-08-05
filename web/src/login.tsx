import * as React from "react";
import { KeyRound, LoaderCircle, LogIn, ShieldCheck } from "lucide-react";

import { authApi } from "./api";
import { useAuth } from "./auth";
import { Button } from "./components/ui/button";

export function LoginPage() {
  const { feishuLoginConfigured } = useAuth();
  const [email, setEmail] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [submitting, setSubmitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await authApi.loginEmail(email.trim(), password);
      window.location.reload();
    } catch (err) {
      const errMsg = err instanceof Error ? err.message : String(err);
      setError(
        errMsg.startsWith("AI网关错误：") || errMsg.startsWith("上游服务错误：")
          ? errMsg
          : `AI网关错误：${errMsg}`,
      );
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <main className="flex min-h-screen items-center justify-center px-5 py-12">
      <section className="dialog-panel w-full max-w-[430px] rounded-[28px] p-6 text-center sm:p-8">
        <div className="mx-auto flex size-12 items-center justify-center rounded-2xl bg-slate-900 text-white shadow-lg shadow-slate-900/15 dark:bg-white dark:text-slate-950">
          <ShieldCheck className="size-6" />
        </div>
        <h1 className="mt-5 text-2xl font-bold tracking-[-0.035em]">AI Gateway</h1>
        <p className="mt-2 text-sm leading-6 text-slate-500 dark:text-slate-400">
          {feishuLoginConfigured
            ? "使用飞书授权登录，或使用管理员创建的账号登录进入管理控制台。"
            : "使用管理员创建的账号登录进入管理控制台。"}
        </p>

        <form onSubmit={submit} className="mt-6 space-y-3 text-left">
          <input
            className="field w-full text-sm"
            type="email"
            value={email}
            placeholder="邮箱"
            autoComplete="email"
            disabled={submitting}
            onChange={(event) => setEmail(event.target.value)}
          />
          <input
            className="field w-full text-sm"
            type="password"
            value={password}
            placeholder="密码"
            autoComplete="current-password"
            disabled={submitting}
            onChange={(event) => setPassword(event.target.value)}
          />
          {error ? (
            <div className="rounded-xl bg-rose-500/10 px-4 py-3 text-sm leading-6 text-rose-600 dark:text-rose-300">
              {error}
            </div>
          ) : null}
          <Button type="submit" className="w-full" disabled={submitting || !email.trim() || !password}>
            {submitting ? <LoaderCircle className="size-4 animate-spin" /> : <KeyRound className="size-4" />}
            {submitting ? "正在登录" : "邮箱密码登录"}
          </Button>
        </form>

        {feishuLoginConfigured ? (
          <>
            <div className="mt-5 flex items-center gap-3 text-[11px] text-slate-400">
              <span className="h-px flex-1 bg-slate-200 dark:bg-white/10" />
              或
              <span className="h-px flex-1 bg-slate-200 dark:bg-white/10" />
            </div>
            <Button className="mt-4 w-full" variant="outline" onClick={() => { window.location.href = "/auth/feishu/authorize"; }}>
              <LogIn className="size-4" />
              飞书授权登录
            </Button>
          </>
        ) : (
          <div className="mt-5 rounded-xl bg-amber-500/10 px-4 py-3 text-left text-sm leading-6 text-amber-700 dark:text-amber-300">
            需要由管理员在“管理员设置 → 用户管理”中为你创建账号后才能登录。
          </div>
        )}
      </section>
    </main>
  );
}

import { LogIn, ShieldCheck } from "lucide-react";

import { useAuth } from "./auth";
import { Button } from "./components/ui/button";

export function LoginPage() {
  const { feishuLoginConfigured } = useAuth();

  return (
    <main className="flex min-h-screen items-center justify-center px-5 py-12">
      <section className="dialog-panel w-full max-w-[430px] rounded-[28px] p-6 text-center sm:p-8">
        <div className="mx-auto flex size-12 items-center justify-center rounded-2xl bg-slate-900 text-white shadow-lg shadow-slate-900/15 dark:bg-white dark:text-slate-950">
          <ShieldCheck className="size-6" />
        </div>
        <h1 className="mt-5 text-2xl font-bold tracking-[-0.035em]">AI Gateway</h1>
        <p className="mt-2 text-sm leading-6 text-slate-500 dark:text-slate-400">
          使用飞书授权登录后即可进入管理控制台。
        </p>

        {feishuLoginConfigured ? (
          <Button className="mt-7 w-full" onClick={() => { window.location.href = "/auth/feishu/authorize"; }}>
            <LogIn className="size-4" />
            飞书授权登录
          </Button>
        ) : (
          <div className="mt-7 rounded-xl bg-amber-500/10 px-4 py-3 text-left text-sm leading-6 text-amber-700 dark:text-amber-300">
            飞书登录尚未配置。请为服务设置 <code>FEISHU_APP_ID</code> 与 <code>FEISHU_APP_SECRET</code>，并在飞书应用中登记回调地址。
          </div>
        )}
      </section>
    </main>
  );
}

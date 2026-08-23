const LOCAL_API_ROOT = "http://127.0.0.1:4242";

type TauriCore = {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
};

function tauriCore(): TauriCore | null {
  const candidate = (
    globalThis as typeof globalThis & {
      __TAURI__?: { core?: TauriCore };
    }
  ).__TAURI__?.core;
  return candidate && typeof candidate.invoke === "function" ? candidate : null;
}

export function isTauriHost(): boolean {
  return tauriCore() !== null;
}

export async function invokeTauri<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const tauri = tauriCore();
  if (!tauri) {
    throw new Error("请使用 AI Gateway 客户端管理本机 Codex");
  }
  return (await tauri.invoke(cmd, args)) as T;
}

export function apiRoot(): string {
  return LOCAL_API_ROOT;
}

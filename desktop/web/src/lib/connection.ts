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

export async function invokeTauri<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const tauri = tauriCore();
  if (!tauri) {
    throw new Error("AI Gateway 管理界面只能在桌面客户端中使用");
  }
  return (await tauri.invoke(cmd, args)) as T;
}

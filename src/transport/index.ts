// 统一传输层：桌面端转调真实 Tauri IPC，浏览器端走同源 HTTP API。
// 对外函数保持与 @tauri-apps/api 相同的调用签名，现有调用点只需改 import 来源。
import { invoke as tauriInvoke, type InvokeArgs } from '@tauri-apps/api/core';
import {
  listen as tauriListen,
  emit as tauriEmit,
  type Event,
} from '@tauri-apps/api/event';
import {
  getCurrentWindow as tauriGetCurrentWindow,
  currentMonitor as tauriCurrentMonitor,
} from '@tauri-apps/api/window';

const SESSION_TOKEN_KEY = 'sugt_webui_session_token';
const AUTH_EXPIRED_EVENT = 'sugt://auth-expired';

/** 桌面端（Tauri IPC 可用）= false；普通浏览器 = true。 */
export function isBrowserMode(): boolean {
  return typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window);
}

export function getSessionToken(): string | null {
  try {
    return sessionStorage.getItem(SESSION_TOKEN_KEY);
  } catch {
    return null;
  }
}

export function setSessionToken(token: string): void {
  try {
    sessionStorage.setItem(SESSION_TOKEN_KEY, token);
  } catch {
    /* 隐私模式下仅丢失登录态持久化，不影响本次会话 */
  }
}

export function clearSessionToken(): void {
  try {
    sessionStorage.removeItem(SESSION_TOKEN_KEY);
  } catch {
    /* 忽略 */
  }
}

/** 与 Tauri invoke 同签名；浏览器端带 Bearer token 调 /api/invoke。 */
export async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isBrowserMode()) {
    return tauriInvoke<T>(cmd, args as InvokeArgs | undefined);
  }

  const token = getSessionToken();
  const res = await fetch('/api/invoke', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    body: JSON.stringify({ cmd, args: args ?? {} }),
  });

  if (res.status === 401) {
    clearSessionToken();
    // 通知外层 App 回到登录态；本地事件同步派发，不 await。
    void emit(AUTH_EXPIRED_EVENT);
    throw new Error('登录已过期，请重新登录');
  }
  if (!res.ok) {
    throw new Error(`请求失败（HTTP ${res.status}）`);
  }

  let payload: { ok?: boolean; data?: T; error?: string };
  try {
    payload = (await res.json()) as typeof payload;
  } catch {
    throw new Error('服务端响应格式错误');
  }
  if (!payload.ok) {
    throw new Error(payload.error || '未知错误');
  }
  return payload.data as T;
}

/**
 * 带超时兜底的 invoke：探测类命令可能长时间不返回，裸 invoke 会让 UI 永久停在「加载中」。
 * 超时后抛清晰的中文错误，让调用方落到真实错误态；正常返回时清除定时器。
 */
export async function invokeWithDeadline<T>(
  cmd: string,
  args?: Record<string, unknown>,
  // 略宽于后端 list_model_registry 的 25s 端到端超时：让后端的中文错误先返回，
  // 前端只在后端彻底不响应（进程卡死/IPC 丢失）时才兜底，避免吞掉真实错误。
  timeoutMs = 28_000,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      invoke<T>(cmd, args),
      new Promise<T>((_resolve, reject) => {
        timer = setTimeout(
          () => reject(new Error(`请求超时（${cmd}），请稍后重试`)),
          timeoutMs,
        );
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

// 浏览器端事件总线：与 Tauri listen/emit 同语义，应用内部可自消费、不抛错。
type BrowserHandler = (event: { payload: unknown }) => void;
const browserListeners = new Map<string, Set<BrowserHandler>>();

export async function listen<T>(
  event: string,
  handler: (e: { payload: T }) => void,
): Promise<() => void> {
  if (!isBrowserMode()) {
    return tauriListen<T>(event, handler as (e: Event<T>) => void);
  }
  let set = browserListeners.get(event);
  if (!set) {
    set = new Set();
    browserListeners.set(event, set);
  }
  const wrapped = handler as BrowserHandler;
  set.add(wrapped);
  return () => {
    set?.delete(wrapped);
  };
}

export async function emit(event: string, payload?: unknown): Promise<void> {
  if (!isBrowserMode()) {
    return tauriEmit(event, payload);
  }
  const set = browserListeners.get(event);
  if (!set) return;
  for (const handler of [...set]) {
    try {
      handler({ payload });
    } catch {
      /* 单个监听器异常不影响其它监听器 */
    }
  }
}

// 浏览器端窗口/显示器 stub：宠物 hook 在浏览器里会被跳过，这里仅保证 import 与偶发调用不抛错。
function asyncNoop(): Promise<void> {
  return Promise.resolve();
}

type BrowserWindow = ReturnType<typeof tauriGetCurrentWindow>;

function createBrowserWindow(): BrowserWindow {
  const base: Record<string | symbol, unknown> = {
    label: 'main',
    outerPosition: async () => ({ x: 0, y: 0 }),
    innerPosition: async () => ({ x: 0, y: 0 }),
    outerSize: async () => ({ width: 0, height: 0 }),
    innerSize: async () => ({ width: 0, height: 0 }),
    scaleFactor: async () => 1,
    setPosition: asyncNoop,
    setSize: asyncNoop,
    setFocus: asyncNoop,
    startDragging: asyncNoop,
    onMoved: async () => () => undefined,
  };
  // 未知成员一律返回 async no-op，任何调用都不会抛错。
  return new Proxy(base, {
    get(target, prop) {
      return prop in target ? target[prop] : asyncNoop;
    },
  }) as unknown as BrowserWindow;
}

export function getCurrentWindow(): BrowserWindow {
  if (!isBrowserMode()) return tauriGetCurrentWindow();
  return createBrowserWindow();
}

export async function currentMonitor(): Promise<
  Awaited<ReturnType<typeof tauriCurrentMonitor>>
> {
  if (!isBrowserMode()) return tauriCurrentMonitor();
  return {
    name: 'browser',
    isPrimary: true,
    position: { x: 0, y: 0 },
    size: { width: 1920, height: 1080 },
    workArea: { position: { x: 0, y: 0 }, size: { width: 1920, height: 1080 } },
    scaleFactor: 1,
  } as unknown as Awaited<ReturnType<typeof tauriCurrentMonitor>>;
}

export { AUTH_EXPIRED_EVENT };

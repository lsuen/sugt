// 流量悬浮窗：置顶 + 半透明 + 鼠标穿透的小窗。
// 轮询 get_status 展示今日 token 与最近来源；编辑模式（穿透关闭）可拖动并「保存位置」。
import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import './overlay.css';

type OverlayConfigView = {
  enabled: boolean;
  opacity: number;
  edit: boolean;
  show_tokens: boolean;
  show_client: boolean;
  x: number;
  y: number;
};

type Status = {
  running: boolean;
  last_proxy_client?: string | null;
  last_proxy_mode?: string | null;
  traffic: { today_input_tokens: number; today_output_tokens: number };
};

const fmt = (n: number) => (n >= 10000 ? `${(n / 10000).toFixed(1)}w` : n.toLocaleString());

const MODE_LABEL: Record<string, string> = {
  anthropic_native: 'Anthropic 原生',
  openai_adapter: 'OpenAI 适配',
  openai_direct: 'OpenAI 直连',
  responses_native: 'Responses 原生',
  chat_completions_adapter: 'Chat 适配',
};

function Overlay() {
  const [cfg, setCfg] = useState<OverlayConfigView | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    invoke<OverlayConfigView>('get_overlay_config')
      .then(setCfg)
      .catch(() => undefined);
    const un = listen<OverlayConfigView>('sugt://overlay-config-changed', (e) => setCfg(e.payload));
    return () => {
      un.then((fn) => fn()).catch(() => undefined);
    };
  }, []);

  useEffect(() => {
    const poll = () => invoke<Status>('get_status').then(setStatus).catch(() => undefined);
    poll();
    const timer = window.setInterval(poll, 3000);
    return () => window.clearInterval(timer);
  }, []);

  const savePos = async () => {
    if (!cfg || busy) return;
    setBusy(true);
    try {
      const pos = await getCurrentWindow().outerPosition();
      await invoke('set_overlay_config', { cfg: { ...cfg, x: pos.x, y: pos.y, edit: false } });
    } catch {
      // 窗口被关闭等场景：静默即可
    } finally {
      setBusy(false);
    }
  };

  const traffic = status?.traffic;
  const client = status?.last_proxy_client ?? '—';
  const mode = status?.last_proxy_mode
    ? MODE_LABEL[status.last_proxy_mode] ?? status.last_proxy_mode
    : '—';

  // 抓手按住拖动窗口（比 data-tauri-drag-region 属性更可靠）
  const startDrag = (e: React.MouseEvent<HTMLSpanElement>) => {
    e.preventDefault();
    void getCurrentWindow()
      .startDragging()
      .catch(() => undefined);
  };

  return (
    <div
      className={cfg?.edit ? 'overlay-box editing' : 'overlay-box'}
      style={{ background: `rgba(7, 11, 20, ${cfg?.opacity ?? 0.7})` }}
    >
      {cfg?.show_tokens !== false && (
        <div className="overlay-row">
          <span className="overlay-label">流量</span>
          <span className="overlay-tokens">
            ↑{traffic ? fmt(traffic.today_input_tokens) : '—'}
            <span className="overlay-dim">/</span>
            ↓{traffic ? fmt(traffic.today_output_tokens) : '—'}
          </span>
          {cfg?.edit && (
            <>
              <button
                type="button"
                className="overlay-save"
                onClick={() => void savePos()}
                disabled={busy}
              >
                {busy ? '…' : '保存'}
              </button>
              <span
                className="overlay-grip"
                onMouseDown={startDrag}
                title="按住拖动"
                aria-label="拖动悬浮窗"
              />
            </>
          )}
        </div>
      )}
      {cfg?.show_client !== false && (
        <div className="overlay-row">
          <span className="overlay-label">来源</span>
          <span className="overlay-client" title={`${client} · ${mode}`}>
            {client}
            <span className="overlay-dim">{mode}</span>
          </span>
        </div>
      )}
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Overlay />);

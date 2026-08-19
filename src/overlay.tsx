// 流量悬浮窗：置顶 + 半透明 + 鼠标穿透的小窗。
// 轮询 get_status 展示今日 token 与最近来源；编辑模式（穿透关闭）可拖动并「保存位置」。
import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import './overlay.css';
import { t, type TKey } from './i18n';

type OverlayConfigView = {
  enabled: boolean;
  opacity: number;
  edit: boolean;
  show_tokens: boolean;
  show_client: boolean;
  x: number;
  y: number;
  layout: 'column' | 'row';
  width: number;
  height: number;
};

type Status = {
  running: boolean;
  last_proxy_client?: string | null;
  last_proxy_mode?: string | null;
  traffic: { today_input_tokens: number; today_output_tokens: number };
};

const fmt = (n: number) => (n >= 10000 ? `${(n / 10000).toFixed(1)}w` : n.toLocaleString());

/** 后端 ProxyHit.mode 实际协议路径 → i18n 键（与网关日志 mode= 一一对应） */
const PROXY_MODE_KEY: Record<string, TKey> = {
  anthropic_native: 'proto.anthropicNative',
  openai_adapter: 'proto.openaiAdapter',
  openai_direct: 'proto.openaiDirect',
  responses_native: 'proto.responsesNative',
  chat_completions_adapter: 'proto.chatAdapter',
};

/** 协议模式可读标签；未知名保持原样。t() 依赖文件底部 dict，须在调用时求值（模块级会 TDZ 报错） */
function proxyModeLabel(mode: string): string {
  return PROXY_MODE_KEY[mode] ? t(PROXY_MODE_KEY[mode]) : mode;
}

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
  const mode = status?.last_proxy_mode ? proxyModeLabel(status.last_proxy_mode) : '—';

  // 抓手按住拖动窗口：后端直调 window.start_dragging()（前端 startDragging 受 ACL 限制）
  const startDrag = (e: React.MouseEvent<HTMLSpanElement>) => {
    e.preventDefault();
    void invoke('overlay_start_drag').catch(() => undefined);
  };

  // 布局方向：横排(row)时两行内容并排单行；竖排(column)时上下两行
  const isRow = cfg?.layout === 'row';

  return (
    <div
      className={cfg?.edit ? 'overlay-box editing' : 'overlay-box'}
      style={{ background: `rgba(7, 11, 20, ${cfg?.opacity ?? 0.7})` }}
    >
      <div className={isRow ? 'overlay-rows-h' : 'overlay-rows'}>
        {cfg?.show_tokens !== false && (
          <div className="overlay-row">
            <span className="overlay-label">{t('overlay.flow')}</span>
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
                  {busy ? '…' : t('overlay.save')}
                </button>
                <span
                  className="overlay-grip"
                  onMouseDown={startDrag}
                  title={t('overlay.drag')}
                  aria-label={t('overlay.drag')}
                />
              </>
            )}
          </div>
        )}
        {cfg?.show_client !== false && (
          <div className="overlay-row">
            <span className="overlay-label">{t('overlay.source')}</span>
            <span className="overlay-client" title={`${client} · ${mode}`}>
              {client}
              <span className="overlay-dim">{mode}</span>
            </span>
          </div>
        )}
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Overlay />);

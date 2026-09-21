// 流量悬浮窗：置顶 + 半透明 + 鼠标穿透。
// 数据与仪表盘「请求统计」同源（get_status.traffic + last_proxy_*），无假数据。
import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import './overlay.css';
import { t } from './i18n';

type OverlayConfigView = {
  enabled: boolean;
  opacity: number;
  edit: boolean;
  show_tokens: boolean;
  show_client: boolean;
  show_channel: boolean;
  show_requests: boolean;
  x: number;
  y: number;
  layout: 'column' | 'row';
  width: number;
  height: number;
};

type TrafficSlice = {
  today_requests: number;
  total_requests: number;
  success_count: number;
  failed_count: number;
  today_input_tokens: number;
  today_output_tokens: number;
};

type Status = {
  running: boolean;
  last_proxy_client?: string | null;
  last_proxy_provider?: string | null;
  last_proxy_path?: string | null;
  traffic: TrafficSlice;
};

const fmt = (n: number) => (n >= 10000 ? `${(n / 10000).toFixed(1)}w` : n.toLocaleString());

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
    const timer = window.setInterval(poll, 2000);
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

  const startDrag = (e: React.MouseEvent<HTMLSpanElement>) => {
    e.preventDefault();
    void invoke('overlay_start_drag').catch(() => undefined);
  };

  const traffic = status?.traffic;
  const running = status?.running ?? false;
  const today = traffic?.today_requests ?? 0;
  const total = traffic?.total_requests ?? 0;
  const okRate =
    total > 0 && traffic
      ? Math.round((traffic.success_count / total) * 100)
      : 100;
  const client = status?.last_proxy_client?.trim() || '';
  const channel = status?.last_proxy_provider?.trim() || '';
  const isRow = cfg?.layout === 'row';
  const showReq = cfg?.show_requests !== false;
  const showTok = cfg?.show_tokens !== false;
  const showClient = cfg?.show_client !== false;
  const showChannel = cfg?.show_channel !== false;
  const empty = today === 0 && !client && !channel;

  return (
    <div
      className={cfg?.edit ? 'overlay-box editing' : 'overlay-box'}
      style={{ background: `rgba(7, 11, 20, ${cfg?.opacity ?? 0.7})` }}
    >
      {cfg?.edit && (
        <div className="overlay-edit-bar">
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
        </div>
      )}

      <div className={isRow ? 'overlay-body row' : 'overlay-body'}>
        {!running ? (
          <div className="overlay-idle">{t('overlay.off')}</div>
        ) : empty ? (
          <div className="overlay-idle">{t('overlay.idle')}</div>
        ) : (
          <>
            {(showReq || showTok) && (
              <div className="overlay-block">
                {showReq && (
                  <div className="overlay-line">
                    <span className="overlay-k">{t('overlay.today')}</span>
                    <span className="overlay-v">{fmt(today)}</span>
                    {total > 0 && (
                      <span className={okRate < 90 ? 'overlay-fail' : 'overlay-ok'}>
                        {okRate}%
                      </span>
                    )}
                  </div>
                )}
                {showTok && traffic && (
                  <div className="overlay-line">
                    <span className="overlay-k">{t('overlay.flow')}</span>
                    <span className="overlay-v tabular">
                      ↑{fmt(traffic.today_input_tokens)}
                      <span className="overlay-dim">/</span>
                      ↓{fmt(traffic.today_output_tokens)}
                    </span>
                  </div>
                )}
              </div>
            )}
            {(showClient || showChannel) && (
              <div className="overlay-block">
                {showClient && (
                  <div className="overlay-line">
                    <span className="overlay-k">{t('overlay.source')}</span>
                    <span className="overlay-v trunc" title={client || '—'}>
                      {client || '—'}
                    </span>
                  </div>
                )}
                {showChannel && (
                  <div className="overlay-line">
                    <span className="overlay-k">{t('overlay.channel')}</span>
                    <span className="overlay-v trunc" title={channel || '—'}>
                      {channel || '—'}
                    </span>
                  </div>
                )}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Overlay />);

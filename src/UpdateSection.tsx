import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Download, ExternalLink, RefreshCw } from 'lucide-react';
import { t } from './i18n';

export type UpdateInfo = {
  current_version: string;
  latest_version: string;
  update_available: boolean;
  release_notes: string;
  release_url: string;
  asset_name: string | null;
  asset_url: string | null;
  asset_sha256: string | null;
  channel_label: string;
  channel_prefix: string;
  platform: string;
};

type Progress = {
  phase: string;
  downloaded: number;
  total: number | null;
  message: string;
};

type Props = {
  busy: boolean;
  setBusy: (v: boolean) => void;
  pushToast: (msg: string, kind?: 'ok' | 'error' | 'info') => void;
  formatInvokeError: (error: unknown) => string;
};

function channelText(info: UpdateInfo): string {
  if (info.channel_label === 'direct' || !info.channel_prefix) {
    return t('update.channelDirect');
  }
  if (info.channel_label === 'user') {
    return t('update.channelUser', { prefix: info.channel_prefix });
  }
  return t('update.channelMirror', { prefix: info.channel_prefix });
}

export function UpdateSection({ busy, setBusy, pushToast, formatInvokeError }: Props) {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [checking, setChecking] = useState(false);
  const [applying, setApplying] = useState(false);

  useEffect(() => {
    let un: (() => void) | undefined;
    listen<Progress>('sugt://update-progress', (ev) => {
      setProgress(ev.payload);
    }).then((fn) => {
      un = fn;
    });
    return () => {
      un?.();
    };
  }, []);

  const check = async () => {
    setChecking(true);
    setBusy(true);
    setProgress(null);
    try {
      const next = await invoke<UpdateInfo>('check_app_update');
      setInfo(next);
      if (next.update_available) {
        pushToast(t('update.toastAvailable', { version: next.latest_version }), 'info');
      } else {
        pushToast(t('update.toastLatest'), 'ok');
      }
    } catch (error) {
      pushToast(formatInvokeError(error), 'error');
    } finally {
      setChecking(false);
      setBusy(false);
    }
  };

  const apply = async () => {
    if (!info?.update_available) return;
    if (!info.asset_url) {
      pushToast(t('update.noAsset'), 'error');
      return;
    }
    setApplying(true);
    setBusy(true);
    try {
      const result = await invoke<{ message: string; will_exit: boolean }>('apply_app_update', {
        info,
      });
      pushToast(result.message, 'ok');
      if (result.will_exit) {
        pushToast(t('update.toastExiting'), 'info');
      }
    } catch (error) {
      pushToast(formatInvokeError(error), 'error');
    } finally {
      setApplying(false);
      setBusy(false);
    }
  };

  const openRelease = async () => {
    try {
      await invoke('open_app_release_page', { url: info?.release_url ?? null });
    } catch (error) {
      pushToast(formatInvokeError(error), 'error');
    }
  };

  const pct =
    progress?.total && progress.total > 0
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : null;

  return (
    <div className="about-update">
      <div className="section-title">
        <h4>{t('update.title')}</h4>
        <div className="title-actions" style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <button
            type="button"
            className="ghost tiny-btn"
            disabled={busy || checking}
            onClick={() => void check()}
          >
            <RefreshCw size={14} />
            {checking ? t('update.checking') : t('update.check')}
          </button>
          <button
            type="button"
            className="ghost tiny-btn"
            disabled={busy}
            onClick={() => void openRelease()}
          >
            <ExternalLink size={14} />
            {t('update.openRelease')}
          </button>
        </div>
      </div>
      <p className="hint compact">{t('update.hint')}</p>

      {info && (
        <div className="about-update-status">
          <div>
            <strong>{t('update.current')}</strong>
            <span>v{info.current_version}</span>
          </div>
          <div>
            <strong>{t('update.latest')}</strong>
            <span>v{info.latest_version}</span>
          </div>
          <div>
            <strong>{t('update.channel')}</strong>
            <span>{channelText(info)}</span>
          </div>
          <div>
            <strong>{t('update.status')}</strong>
            <span>
              {info.update_available
                ? t('update.statusAvailable')
                : t('update.statusLatest')}
            </span>
          </div>
        </div>
      )}

      {progress && (
        <div className="about-update-progress">
          <span>{progress.message}</span>
          {pct != null && (
            <div className="about-update-bar">
              <i style={{ width: `${pct}%` }} />
            </div>
          )}
        </div>
      )}

      {info?.update_available && (
        <div className="about-update-actions">
          <button
            type="button"
            className="primary tiny-btn"
            disabled={busy || applying || !info.asset_url}
            onClick={() => void apply()}
          >
            <Download size={14} />
            {applying
              ? t('update.applying')
              : t('update.apply', { version: info.latest_version })}
          </button>
          {!info.asset_url && <p className="hint compact">{t('update.noAsset')}</p>}
        </div>
      )}
    </div>
  );
}

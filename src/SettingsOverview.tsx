// 作者：sun / 孙文龙
// 作用：设置-概览子页（首个横向 Tab）。聚合服务状态、局域网接入信息、上游、端点与版本构建号。
// 只读：所有数据来自已有 get_status / get_lan_share_info，不新增后端命令、不写配置。
import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Copy, RefreshCw } from 'lucide-react';
import type { LanShareInfo } from './SettingsPanel';
import type { AboutStatusView } from './AboutSection';
import { CURRENT_VERSION } from './release-notes';
import { t } from './i18n';
import type { SettingsTab } from './SettingsPanel';

export type OverviewStatus = AboutStatusView & {
  running?: boolean;
  openai_base_url?: string;
  anthropic_base_url?: string;
  gateway_client_api_key?: string;
  public_model_id?: string | null;
  allow_lan_access?: boolean;
  active_provider?: string | null;
  last_proxy_provider?: string | null;
};

type Props = {
  status: OverviewStatus | null;
  statusListenUrl?: string;
  lanEnabled: boolean;
  pushToast: (message: string, type: 'ok' | 'error' | 'info') => void;
  onSubTab: (tab: SettingsTab) => void;
  onNavigate?: (tab: 'dashboard' | 'models' | 'clients' | 'skills' | 'me' | 'rules' | 'settings') => void;
};

export function SettingsOverview({
  status,
  statusListenUrl,
  lanEnabled,
  pushToast,
  onSubTab,
  onNavigate,
}: Props) {
  const [lanInfo, setLanInfo] = useState<LanShareInfo | null>(null);
  const [loading, setLoading] = useState(false);

  const reloadLan = useCallback(async () => {
    setLoading(true);
    try {
      const info = await invoke<LanShareInfo>('get_lan_share_info');
      setLanInfo(info);
    } catch {
      // 后端不可用时不显示过期数据，也不弹错误打断浏览。
      setLanInfo(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reloadLan();
  }, [reloadLan, lanEnabled]);

  const copy = async (text: string, label: string) => {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      pushToast(t('gateway.copied', { label }), 'ok');
    } catch {
      pushToast(t('gateway.copyFailed'), 'error');
    }
  };

  const openaiBase = status?.openai_base_url ?? `${statusListenUrl ?? 'http://127.0.0.1:8787'}/v1`;
  const anthropicBase = status?.anthropic_base_url ?? statusListenUrl ?? 'http://127.0.0.1:8787';
  const apiKey = status?.gateway_client_api_key || 'sugt-local-key';
  const modelId = status?.public_model_id || 'sutai';
  const upstream = status?.active_provider ?? status?.last_proxy_provider ?? '-';
  const buildId = status?.trial?.build_id ?? '-';

  const fields = [
    { label: t('gateway.openaiBase'), value: openaiBase },
    { label: t('gateway.anthropicBase'), value: anthropicBase },
    { label: t('gateway.apiKey'), value: apiKey },
    { label: t('gateway.modelId'), value: modelId },
  ];

  return (
    <div className="settings-layout settings-layout--stack">
      <div className="card settings-group settings-group-wide">
        <div className="section-title">
          <h4>{t('set.ovTitle')}</h4>
          <button type="button" className="ghost tiny-btn" disabled={loading} onClick={() => void reloadLan()}>
            <RefreshCw size={14} />{t('common.refresh')}
          </button>
        </div>
        <p className="hint compact">{t('set.ovHint')}</p>
        <div className="overview-grid">
          <div>
            <strong>{t('set.ovService')}</strong>
            <span>
              <span className={status?.running ? 'badge ok inline' : 'badge stop inline'}>
                {status?.running ? t('common.running') : t('common.stopped')}
              </span>
              {' '}<code>{statusListenUrl ?? 'http://127.0.0.1:8787'}</code>
            </span>
          </div>
          <div><strong>{t('set.ovUpstream')}</strong><span>{upstream}</span></div>
          <div><strong>{t('about.version')}</strong><span>v{CURRENT_VERSION} · {t('set.ovBuild')} {buildId}</span></div>
          <div><strong>{t('about.configDir')}</strong><span><code>{status?.config_dir ?? '-'}</code></span></div>
        </div>
      </div>

      <div className="card settings-group settings-group-wide">
        <div className="section-title">
          <h4>{t('set.ovLan')}</h4>
          <span className={lanEnabled ? 'badge ok inline' : 'badge neutral inline'}>
            {lanEnabled ? t('set.ovLanOn') : t('set.ovLanOff')}
          </span>
        </div>
        <p className="hint compact">{t('set.ovLanHint')}</p>
        {lanEnabled && lanInfo?.hosts?.length ? (
          <div className="overview-lan-list">
            <strong>{t('set.ovHosts')}</strong>
            <ul className="lan-share-list">
              {lanInfo.hosts.map((host, i) => (
                <li key={host}>
                  <code>{lanInfo.openai_bases[i] ?? `http://${host}:${lanInfo.port}/v1`}</code>
                  <button
                    type="button"
                    className="tiny icon-only"
                    aria-label={t('common.copy')}
                    onClick={() => void copy(lanInfo.openai_bases[i] ?? `http://${host}:${lanInfo.port}/v1`, host)}
                  >
                    <Copy size={13} />
                  </button>
                </li>
              ))}
              <li><code>{lanInfo.api_key}</code></li>
              <li><code>{lanInfo.model_id}</code></li>
            </ul>
          </div>
        ) : (
          <p className="hint compact">{lanEnabled ? t('set.ovLanEmpty') : t('set.ovLanOffHint')}</p>
        )}
      </div>

      <div className="card settings-group settings-group-wide">
        <h4>{t('set.endpoint')}</h4>
        <div className="overview-grid">
          {fields.map((field) => (
            <div key={field.label}>
              <strong>{field.label}</strong>
              <span className="overview-value-row">
                <code>{field.value}</code>
                <button
                  type="button"
                  className="tiny icon-only"
                  aria-label={field.label}
                  onClick={() => void copy(field.value, field.label)}
                >
                  <Copy size={13} />
                </button>
              </span>
            </div>
          ))}
        </div>
      </div>

      <div className="card settings-group settings-group-wide">
        <h4>{t('set.ovQuick')}</h4>
        <p className="hint compact">{t('set.ovUpdateHint')}</p>
        <div className="settings-actions-row">
          <button type="button" className="ghost tiny-btn" onClick={() => onSubTab('logs')}>{t('set.ovGoLogs')}</button>
          <button type="button" className="ghost tiny-btn" onClick={() => onSubTab('about')}>{t('set.ovGoAbout')}</button>
          <button type="button" className="ghost tiny-btn" onClick={() => onSubTab('general')}>{t('set.ovGoGeneral')}</button>
          <button type="button" className="ghost tiny-btn" onClick={() => onSubTab('pet')}>{t('set.ovGoPet')}</button>
          <button type="button" className="ghost tiny-btn" onClick={() => onNavigate?.('clients')}>{t('set.goClients')}</button>
          <button type="button" className="ghost tiny-btn" onClick={() => onNavigate?.('skills')}>{t('set.openSkills')}</button>
        </div>
      </div>
    </div>
  );
}

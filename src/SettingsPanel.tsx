// 作者：sun / 孙文龙
// 作用：设置面板。横向子 Tab（概览/通用/外观/高级/精灵/日志/关于），子页状态由上层受控，
// 保证状态栏「打开日志页」等外部入口可以精确跳转，切换主 Tab 后也不丢子页位置。
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CircleAlert, Copy } from 'lucide-react';
import { ModalPortal } from './ModalPortal';
import { GithubProxyModal } from './store/GithubProxyModal';
import { langSetting, setLang, t, type LangSetting, type TKey } from './i18n';
import { ThemeSettingsSection } from './theme';
import type { CustomBgState, ThemeId, ThemeMeta } from './theme';
import { LogsSection } from './LogsSection';
import { AboutSection } from './AboutSection';
import { SettingsOverview, type OverviewStatus } from './SettingsOverview';
import { PetSettingsSection } from './PetSettingsSection';
import { MaintenanceSection } from './MaintenanceSection';
import { DEFAULT_OVERLAY, type OverlayConfigView } from './pet/config';
export type { OverlayConfigView };

export type LanShareInfo = {
  enabled: boolean;
  port: number;
  api_key: string;
  model_id: string;
  hosts: string[];
  openai_bases: string[];
  anthropic_bases: string[];
  copy_text: string;
};

type QuitBehavior = 'exit_only' | 'stop_gateway' | 'stop_all';

type AppConfigView = {
  autostart?: boolean;
  failover_enabled?: boolean;
  autostart_gateway?: boolean;
  quit_behavior?: QuitBehavior;
  port_fallback_enabled?: boolean;
  gateway_watchdog_enabled?: boolean;
  allow_lan_access?: boolean;
  log_retention_days?: number;
  overlay?: OverlayConfigView;
};

type Tab = 'dashboard' | 'models' | 'clients' | 'skills' | 'me' | 'rules' | 'settings';
export type SettingsTab = 'overview' | 'general' | 'appearance' | 'advanced' | 'pet' | 'logs' | 'about';

const SETTINGS_TABS: { id: SettingsTab; key: TKey }[] = [
  { id: 'overview', key: 'set.tabOverview' },
  { id: 'general', key: 'set.tabGeneral' },
  { id: 'appearance', key: 'set.tabAppearance' },
  { id: 'advanced', key: 'set.tabAdvanced' },
  { id: 'pet', key: 'set.tabPet' },
  { id: 'logs', key: 'set.tabLogs' },
  { id: 'about', key: 'set.tabAbout' },
];

type Props = {
  config: AppConfigView | null;
  statusListenUrl?: string;
  run: (fn: () => Promise<unknown>, ok?: string) => void;
  onNavigate?: (tab: Tab) => void;
  pushToast?: (message: string, type: 'ok' | 'error' | 'info') => void;
  formatInvokeError?: (error: unknown) => string;
  themeId: ThemeId;
  themeCatalog: ThemeMeta[];
  onTheme: (id: ThemeId) => void;
  onThemeImport: (meta: ThemeMeta) => void;
  themeError?: string | null;
  customBg: CustomBgState | null;
  onCustomBgFile: (file: File) => Promise<void>;
  onCustomBgOpacity: (opacity: number) => void;
  onClearCustomBg: () => void;
  statusBarEnabled: boolean;
  onStatusBarEnabled: (enabled: boolean) => void;
  // 子页状态：由 App 受控；未传时退回组件内 state（浏览器预览 / 单组件复用）。
  settingsTab?: SettingsTab;
  onSettingsTabChange?: (tab: SettingsTab) => void;
  status?: OverviewStatus | null;
  logs?: string[];
  busy?: boolean;
  setBusy?: (v: boolean) => void;
  onRefreshLogs?: () => void;
  onVersionCardClick?: () => void;
};

export function SettingsPanel({
  config,
  statusListenUrl,
  run,
  onNavigate,
  pushToast,
  formatInvokeError,
  themeId,
  themeCatalog,
  onTheme,
  onThemeImport,
  themeError,
  customBg,
  onCustomBgFile,
  onCustomBgOpacity,
  onClearCustomBg,
  statusBarEnabled,
  onStatusBarEnabled,
  settingsTab,
  onSettingsTabChange,
  status,
  logs,
  busy,
  setBusy,
  onRefreshLogs,
  onVersionCardClick,
}: Props) {
  const [proxyOpen, setProxyOpen] = useState(false);
  const [proxyPrefix, setProxyPrefix] = useState('');
  const [internalTab, setInternalTab] = useState<SettingsTab>('overview');
  const [lanOpen, setLanOpen] = useState(false);
  const [lanInfo, setLanInfo] = useState<LanShareInfo | null>(null);

  const tab = settingsTab ?? internalTab;
  const selectTab = (next: SettingsTab) => {
    if (onSettingsTabChange) onSettingsTabChange(next);
    else setInternalTab(next);
  };
  // 统一成「kind 可选」签名，既能喂给只传一个参数的组件，也能喂给要求必传的组件。
  const toast = (message: string, type?: 'ok' | 'error' | 'info') =>
    (pushToast ?? (() => undefined))(message, type ?? 'ok');
  const formatError = formatInvokeError ?? ((error: unknown) => String(error));

  useEffect(() => {
    if (!lanOpen) return;
    let cancelled = false;
    void invoke<LanShareInfo>('get_lan_share_info')
      .then((info) => {
        if (!cancelled) setLanInfo(info);
      })
      .catch(() => {
        if (!cancelled) setLanInfo(null);
      });
    return () => {
      cancelled = true;
    };
  }, [lanOpen, config?.allow_lan_access]);

  useEffect(() => {
    if (!lanOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setLanOpen(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [lanOpen]);

  const copyLan = async () => {
    const text = lanInfo?.copy_text?.trim();
    if (!text) {
      toast(t('set.lanShareEmpty'), 'info');
      return;
    }
    try {
      await navigator.clipboard.writeText(text);
      toast(t('set.lanShareCopied'), 'ok');
    } catch {
      toast(t('gateway.copyFailed'), 'error');
    }
  };

  // 精灵配置按域增量写入：合并当前快照 + 增量，持久化并即时应用。
  // 本地草稿：父级 config 由 3s 轮询刷新，快速连点两个开关时不能各自拿旧 base 覆盖。
  // 后端只 merge 补丁字段，菜单窗 / 透明窗的并发写入不会互相抹掉。
  const overlayDraft = useRef<OverlayConfigView | null>(null);
  const pendingWrites = useRef(0);
  useEffect(() => {
    if (pendingWrites.current === 0) overlayDraft.current = config?.overlay ?? null;
  }, [config?.overlay]);

  const setOverlay = (patch: Partial<OverlayConfigView>) => {
    const base = overlayDraft.current ?? config?.overlay ?? DEFAULT_OVERLAY;
    overlayDraft.current = { ...base, ...patch };
    pendingWrites.current += 1;
    run(async () => {
      try {
        await invoke('patch_overlay_config', { patch });
      } finally {
        pendingWrites.current -= 1;
      }
    }, t('common.update'));
  };

  const openProxy = async () => {
    try {
      const settings = await invoke<{ editor_command: string; github_proxy_prefix: string }>('store_get_settings');
      setProxyPrefix(settings.github_proxy_prefix ?? '');
      setProxyOpen(true);
    } catch (error) {
      toast(formatError(error), 'error');
    }
  };

  return (
    <div className="settings-layout-wrap">
      {/* 冻结在滚动容器首行：子页再长也不会把 Tab 行顶出视野 */}
      <div className="settings-tabs" role="tablist" aria-label={t('set.tabsLabel')}>
        {SETTINGS_TABS.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            className={tab === item.id ? 'ghost tiny-btn active' : 'ghost tiny-btn'}
            onClick={() => selectTab(item.id)}
          >
            {t(item.key)}
          </button>
        ))}
      </div>

      {tab === 'overview' && (
        <SettingsOverview
          status={status ?? null}
          statusListenUrl={statusListenUrl}
          lanEnabled={Boolean(config?.allow_lan_access)}
          pushToast={toast}
          onSubTab={selectTab}
          onNavigate={onNavigate}
        />
      )}

      {tab === 'general' && (
        <div className="settings-layout settings-layout--stack">
          <div className="card settings-group settings-group-wide settings-lang-bar">
            <div className="settings-lang-main">
              <div>
                <h4>{t('set.language')}</h4>
                <p className="hint compact">{t('set.langHint')}</p>
              </div>
              <label className="switch-line select-line settings-lang-select">
                <span className="sr-only">{t('set.language')}</span>
                <select
                  value={langSetting()}
                  onChange={(e) => setLang(e.target.value as LangSetting)}
                >
                  <option value="system">{t('set.langSystem')}</option>
                  <option value="zh">{t('set.langZh')}</option>
                  <option value="en">{t('set.langEn')}</option>
                </select>
              </label>
            </div>
          </div>

          <div className="card settings-group switches">
            <h4>{t('set.startup')}</h4>
            <p className="hint compact">{t('set.startupHint')}</p>
            <label className="switch-line">
              <span>{t('set.autostart')}</span>
              <input type="checkbox" checked={Boolean(config?.autostart)} onChange={(e) => run(() => invoke('set_autostart', { enabled: e.target.checked }), t('common.update'))} />
            </label>
            <label className="switch-line">
              <span>{t('set.autostartGateway')}</span>
              <input type="checkbox" checked={Boolean(config?.autostart_gateway)} onChange={(e) => run(() => invoke('set_autostart_gateway', { enabled: e.target.checked }), t('common.update'))} />
            </label>
          </div>

          <div className="card settings-group switches">
            <h4>{t('set.gateway')}</h4>
            <p className="hint compact">{t('set.gatewayHint')}</p>
            <label className="switch-line">
              <span>{t('set.failover')}</span>
              <input type="checkbox" checked={Boolean(config?.failover_enabled)} onChange={(e) => run(() => invoke('set_failover', { enabled: e.target.checked }), t('common.update'))} />
            </label>
            <label className="switch-line">
              <span>{t('set.portFallback')}</span>
              <input type="checkbox" checked={Boolean(config?.port_fallback_enabled ?? true)} onChange={(e) => run(() => invoke('update_gateway_settings', { input: { portFallbackEnabled: e.target.checked } }), t('common.update'))} />
            </label>
            <label className="switch-line">
              <span>{t('set.watchdog')}</span>
              <input type="checkbox" checked={Boolean(config?.gateway_watchdog_enabled ?? true)} onChange={(e) => run(() => invoke('update_gateway_settings', { input: { gatewayWatchdogEnabled: e.target.checked } }), t('common.update'))} />
            </label>
          </div>

          <div className="card settings-group switches">
            <h4>{t('set.network')}</h4>
            <p className="hint compact">{t('set.networkHint')}</p>
            <label className="switch-line">
              <span className="switch-line-label">
                {t('set.lan')}
                {config?.allow_lan_access ? (
                  <span className="lan-share-wrap">
                    <button
                      type="button"
                      className="tiny icon-only lan-share-btn"
                      aria-label={t('set.lanShare')}
                      title={t('set.lanShare')}
                      onClick={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        setLanOpen((v) => !v);
                      }}
                    >
                      <CircleAlert size={14} />
                    </button>
                    {lanOpen && (
                      <ModalPortal>
                        <div
                          className="modal-overlay"
                          onMouseDown={(e) => {
                            if (e.target === e.currentTarget) setLanOpen(false);
                          }}
                        >
                          <div className="modal lan-share-modal" role="dialog" aria-modal="true" aria-label={t('set.lanShare')}>
                            <div className="modal-head">
                              <h3>{t('set.lanShare')}</h3>
                              <button type="button" className="icon-btn" onClick={() => setLanOpen(false)} aria-label={t('common.close')}>×</button>
                            </div>
                            <div className="modal-body">
                              {lanInfo?.hosts?.length ? (
                                <ul className="lan-share-list">
                                  {lanInfo.hosts.map((host, i) => (
                                    <li key={host}>
                                      <code>{lanInfo.openai_bases[i] ?? `http://${host}:${lanInfo.port}/v1`}</code>
                                    </li>
                                  ))}
                                  <li><code>{lanInfo.api_key}</code></li>
                                  <li><code>{lanInfo.model_id}</code></li>
                                </ul>
                              ) : (
                                <p className="hint compact">{t('set.lanShareEmpty')}</p>
                              )}
                            </div>
                            <div className="modal-actions">
                              <button type="button" className="ghost" onClick={() => setLanOpen(false)}>{t('common.close')}</button>
                              <button type="button" className="primary" onClick={() => void copyLan()}>
                                <Copy size={14} />{t('set.lanShareCopy')}
                              </button>
                            </div>
                          </div>
                        </div>
                      </ModalPortal>
                    )}
                  </span>
                ) : null}
              </span>
              <input
                type="checkbox"
                checked={Boolean(config?.allow_lan_access)}
                onChange={(e) => {
                  const enabled = e.target.checked;
                  if (!enabled) setLanOpen(false);
                  run(
                    () => invoke('update_gateway_settings', { input: { allowLanAccess: enabled } }),
                    t('common.update'),
                  );
                }}
              />
            </label>
            <label className="switch-line select-line">
              <span>{t('set.quitBehavior')}</span>
              <select
                value={config?.quit_behavior ?? 'stop_all'}
                onChange={(e) => run(() => invoke('set_quit_behavior', { behavior: e.target.value as QuitBehavior }), t('common.update'))}
              >
                <option value="exit_only">{t('set.quitExit')}</option>
                <option value="stop_gateway">{t('set.quitStop')}</option>
                <option value="stop_all">{t('set.quitStopAll')}</option>
              </select>
            </label>
          </div>

          <MaintenanceSection config={config} pushToast={toast} />
        </div>
      )}

      {tab === 'appearance' && (
        <div className="settings-layout settings-layout--stack">
          <ThemeSettingsSection
            themeId={themeId}
            catalog={themeCatalog}
            onTheme={onTheme}
            onImport={onThemeImport}
            error={themeError}
            pushToast={toast}
            customBg={customBg}
            onCustomBgFile={onCustomBgFile}
            onCustomBgOpacity={onCustomBgOpacity}
            onClearCustomBg={onClearCustomBg}
            statusBarEnabled={statusBarEnabled}
            onStatusBarEnabled={onStatusBarEnabled}
          />

        </div>
      )}

      {tab === 'pet' && (
        <PetSettingsSection
          overlay={config?.overlay}
          defaults={DEFAULT_OVERLAY}
          onPatch={setOverlay}
        />
      )}

      {tab === 'advanced' && (
        <div className="settings-layout">
          <div className="card settings-group">
            <h4>{t('set.tabAdvExp')}</h4>
            <p className="hint compact">{t('set.tabAdvExpHint')}</p>
            <div className="settings-actions-row">
              <button
                type="button"
                className="ghost tiny-btn"
                onClick={() =>
                  run(async () => {
                    await invoke('restore_experimental_free');
                  }, t('models.toastZenRestored'))
                }
              >
                {t('set.restoreZen')}
              </button>
              <button type="button" className="ghost tiny-btn" onClick={() => onNavigate?.('models')}>
                {t('set.goModels')}
              </button>
            </div>
          </div>

          <div className="card settings-group">
            <h4>{t('set.tabAdvTakeover')}</h4>
            <p className="hint compact">{t('set.tabAdvTakeoverHint')}</p>
            <div className="settings-actions-row">
              <button type="button" className="ghost tiny-btn" onClick={() => onNavigate?.('clients')}>
                {t('set.goClients')}
              </button>
            </div>
          </div>

          <div className="card settings-group settings-group-wide">
            <h4>{t('set.tabAdvSkills')}</h4>
            <p className="hint compact">{t('set.tabAdvSkillsHint')}</p>
            <div className="settings-actions-row">
              <button type="button" className="ghost tiny-btn" onClick={() => void openProxy()}>
                {t('set.gitProxy')}
              </button>
              <button type="button" className="ghost tiny-btn" onClick={() => onNavigate?.('skills')}>
                {t('set.openSkills')}
              </button>
            </div>
          </div>
        </div>
      )}

      {tab === 'logs' && (
        <LogsSection lines={logs ?? []} busy={Boolean(busy)} onRefresh={onRefreshLogs ?? (() => undefined)} />
      )}

      {tab === 'about' && (
        <AboutSection
          status={status ?? null}
          busy={Boolean(busy)}
          setBusy={setBusy ?? (() => undefined)}
          pushToast={toast}
          formatInvokeError={formatError}
          onVersionCardClick={onVersionCardClick ?? (() => undefined)}
        />
      )}

      {proxyOpen && (
        <GithubProxyModal
          prefix={proxyPrefix}
          onClose={() => setProxyOpen(false)}
          onSaved={(prefix) => {
            setProxyPrefix(prefix);
            setProxyOpen(false);
          }}
          pushToast={toast}
          formatError={formatError}
        />
      )}
    </div>
  );
}

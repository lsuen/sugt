// 作者：sun / 孙文龙
// 作用：设置面板。六个横向子 Tab（概览/通用/外观/高级/日志/关于），子页状态由上层受控，
// 保证状态栏「打开日志页」等外部入口可以精确跳转，切换主 Tab 后也不丢子页位置。
import React, { useEffect, useState } from 'react';
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

// 流量悬浮窗配置（与后端 model.rs OverlayConfig 对应）
export type OverlayConfigView = {
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

// -1 表示「未定位」，由后端 overlay::apply 创建时按主屏右上角初始化
const DEFAULT_OVERLAY: OverlayConfigView = {
  enabled: false,
  opacity: 0.72,
  edit: false,
  show_tokens: true,
  show_client: true,
  show_channel: true,
  show_requests: true,
  x: -1,
  y: -1,
  layout: 'column',
  width: 280,
  height: 108,
};

// 切换布局方向时同步的推荐尺寸（前端只控制方向，尺寸仍可手动调节）
const LAYOUT_SIZE: Record<'column' | 'row', { width: number; height: number }> = {
  column: { width: 280, height: 108 },
  row: { width: 520, height: 72 },
};

type AppConfigView = {
  autostart?: boolean;
  failover_enabled?: boolean;
  autostart_gateway?: boolean;
  quit_behavior?: QuitBehavior;
  port_fallback_enabled?: boolean;
  gateway_watchdog_enabled?: boolean;
  allow_lan_access?: boolean;
  overlay?: OverlayConfigView;
};

type Tab = 'dashboard' | 'models' | 'clients' | 'skills' | 'me' | 'rules' | 'settings';
export type SettingsTab = 'overview' | 'general' | 'appearance' | 'advanced' | 'logs' | 'about';

const SETTINGS_TABS: { id: SettingsTab; key: TKey }[] = [
  { id: 'overview', key: 'set.tabOverview' },
  { id: 'general', key: 'set.tabGeneral' },
  { id: 'appearance', key: 'set.tabAppearance' },
  { id: 'advanced', key: 'set.tabAdvanced' },
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
  const [opacity, setOpacity] = useState(config?.overlay?.opacity ?? DEFAULT_OVERLAY.opacity);
  const [width, setWidth] = useState(config?.overlay?.width ?? DEFAULT_OVERLAY.width);
  const [height, setHeight] = useState(config?.overlay?.height ?? DEFAULT_OVERLAY.height);
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

  // 悬浮窗配置更新：合并当前配置 + 增量，持久化并即时应用
  const setOverlay = (patch: Partial<OverlayConfigView>) => {
    const base = config?.overlay ?? DEFAULT_OVERLAY;
    run(() => invoke('set_overlay_config', { cfg: { ...base, ...patch } }), t('common.update'));
  };

  // 外部配置刷新后同步滑块显示值
  useEffect(() => {
    setOpacity(config?.overlay?.opacity ?? DEFAULT_OVERLAY.opacity);
    setWidth(config?.overlay?.width ?? DEFAULT_OVERLAY.width);
    setHeight(config?.overlay?.height ?? DEFAULT_OVERLAY.height);
  }, [config?.overlay?.opacity, config?.overlay?.width, config?.overlay?.height]);

  // 切换横/竖排：同时应用对应推荐尺寸，让布局一眼可见
  const switchOverlayLayout = (layout: 'column' | 'row') => {
    const size = LAYOUT_SIZE[layout];
    setWidth(size.width);
    setHeight(size.height);
    setOverlay({ layout, width: size.width, height: size.height });
  };

  // 恢复默认：重置位置、透明度、布局与尺寸（保留启用/显示项开关）
  const resetOverlay = () => {
    setWidth(DEFAULT_OVERLAY.width);
    setHeight(DEFAULT_OVERLAY.height);
    setOpacity(DEFAULT_OVERLAY.opacity);
    setOverlay({
      opacity: DEFAULT_OVERLAY.opacity,
      x: DEFAULT_OVERLAY.x,
      y: DEFAULT_OVERLAY.y,
      layout: DEFAULT_OVERLAY.layout,
      width: DEFAULT_OVERLAY.width,
      height: DEFAULT_OVERLAY.height,
      edit: false,
    });
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

  const posText = (cfg?: OverlayConfigView) =>
    !cfg || cfg.x < 0 || cfg.y < 0
      ? t('set.overlayPos')
      : t('set.overlayPosAt', { x: cfg.x, y: cfg.y });

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

          <div className="card settings-group switches">
            <h4>{t('set.overlay')}</h4>
            <p className="hint compact">{t('set.overlayHint')}</p>
            <label className="switch-line">
              <span>{t('set.overlayEnable')}</span>
              <input type="checkbox" checked={Boolean(config?.overlay?.enabled)} onChange={(e) => setOverlay({ enabled: e.target.checked })} />
            </label>
            {config?.overlay?.enabled && (
              <>
                <label className="switch-line">
                  <span>{t('set.overlayShowRequests')}</span>
                  <input type="checkbox" checked={config.overlay.show_requests !== false} onChange={(e) => setOverlay({ show_requests: e.target.checked })} />
                </label>
                <label className="switch-line">
                  <span>{t('set.overlayShowTokens')}</span>
                  <input type="checkbox" checked={config.overlay.show_tokens} onChange={(e) => setOverlay({ show_tokens: e.target.checked })} />
                </label>
                <label className="switch-line">
                  <span>{t('set.overlayShowClient')}</span>
                  <input type="checkbox" checked={config.overlay.show_client} onChange={(e) => setOverlay({ show_client: e.target.checked })} />
                </label>
                <label className="switch-line">
                  <span>{t('set.overlayShowChannel')}</span>
                  <input type="checkbox" checked={config.overlay.show_channel !== false} onChange={(e) => setOverlay({ show_channel: e.target.checked })} />
                </label>
                <div className="switch-line">
                  <span>{t('set.overlayOpacity', { p: Math.round(config.overlay.opacity * 100) })}</span>
                  <input
                    type="range"
                    min={0.1}
                    max={1}
                    step={0.05}
                    value={opacity}
                    onChange={(e) => setOpacity(Number(e.target.value))}
                    onMouseUp={(e) => setOverlay({ opacity: Number(e.currentTarget.value) })}
                    onTouchEnd={(e) => setOverlay({ opacity: Number(e.currentTarget.value) })}
                    onKeyUp={(e) => {
                      if (e.key.startsWith('Arrow')) setOverlay({ opacity: Number(e.currentTarget.value) });
                    }}
                  />
                </div>
                <div className="switch-line">
                  <span>{t('set.overlayLayout')}</span>
                  <div className="overlay-layout-switch">
                    <button
                      type="button"
                      className={`ghost tiny-btn${config.overlay.layout !== 'row' ? ' active' : ''}`}
                      onClick={() => switchOverlayLayout('column')}
                    >
                      {t('set.overlayColumn')}
                    </button>
                    <button
                      type="button"
                      className={`ghost tiny-btn${config.overlay.layout === 'row' ? ' active' : ''}`}
                      onClick={() => switchOverlayLayout('row')}
                    >
                      {t('set.overlayRow')}
                    </button>
                  </div>
                </div>
                <div className="switch-line">
                  <span>{t('set.overlayWidth', { w: Math.round(width) })}</span>
                  <input
                    type="range"
                    min={200}
                    max={600}
                    step={10}
                    value={width}
                    onChange={(e) => setWidth(Number(e.target.value))}
                    onMouseUp={(e) => setOverlay({ width: Number(e.currentTarget.value) })}
                    onTouchEnd={(e) => setOverlay({ width: Number(e.currentTarget.value) })}
                    onKeyUp={(e) => {
                      if (e.key.startsWith('Arrow')) setOverlay({ width: Number(e.currentTarget.value) });
                    }}
                  />
                </div>
                <div className="switch-line">
                  <span>{t('set.overlayHeight', { h: Math.round(height) })}</span>
                  <input
                    type="range"
                    min={48}
                    max={200}
                    step={4}
                    value={height}
                    onChange={(e) => setHeight(Number(e.target.value))}
                    onMouseUp={(e) => setOverlay({ height: Number(e.currentTarget.value) })}
                    onTouchEnd={(e) => setOverlay({ height: Number(e.currentTarget.value) })}
                    onKeyUp={(e) => {
                      if (e.key.startsWith('Arrow')) setOverlay({ height: Number(e.currentTarget.value) });
                    }}
                  />
                </div>
                <div className="settings-actions-row">
                  <button type="button" className="ghost tiny-btn" onClick={() => setOverlay({ edit: true })}>
                    {t('set.overlayMove')}
                  </button>
                  <button type="button" className="ghost tiny-btn" onClick={resetOverlay}>
                    {t('set.overlayReset')}
                  </button>
                  <span className="hint compact">
                    {posText(config.overlay)} · {t('set.overlayMoveHint')}
                  </span>
                </div>
              </>
            )}
          </div>
        </div>
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

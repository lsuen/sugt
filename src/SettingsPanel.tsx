import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GithubProxyModal } from './store/GithubProxyModal';
import { langSetting, setLang, t, type LangSetting } from './i18n';

type QuitBehavior = 'exit_only' | 'stop_gateway' | 'stop_all';

// 流量悬浮窗配置（与后端 model.rs OverlayConfig 对应）
export type OverlayConfigView = {
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

// -1 表示「未定位」，由后端 overlay::apply 创建时按主屏右上角初始化
const DEFAULT_OVERLAY: OverlayConfigView = {
  enabled: false,
  opacity: 0.7,
  edit: false,
  show_tokens: true,
  show_client: true,
  x: -1,
  y: -1,
  layout: 'column',
  width: 260,
  height: 76,
};

// 切换布局方向时同步的推荐尺寸（前端只控制方向，尺寸仍可手动调节）
const LAYOUT_SIZE: Record<'column' | 'row', { width: number; height: number }> = {
  column: { width: 260, height: 76 },
  row: { width: 420, height: 48 },
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

type Tab = 'dashboard' | 'models' | 'clients' | 'skills' | 'settings' | 'logs' | 'about';
type SettingsTab = 'basic' | 'advanced';

type Props = {
  config: AppConfigView | null;
  statusListenUrl?: string;
  run: (fn: () => Promise<unknown>, ok?: string) => void;
  onNavigate?: (tab: Tab) => void;
  pushToast?: (message: string, type: 'ok' | 'error' | 'info') => void;
  formatInvokeError?: (error: unknown) => string;
};

export function SettingsPanel({
  config,
  statusListenUrl,
  run,
  onNavigate,
  pushToast,
  formatInvokeError,
}: Props) {
  const [proxyOpen, setProxyOpen] = useState(false);
  const [proxyPrefix, setProxyPrefix] = useState('');
  const [opacity, setOpacity] = useState(config?.overlay?.opacity ?? DEFAULT_OVERLAY.opacity);
  const [width, setWidth] = useState(config?.overlay?.width ?? DEFAULT_OVERLAY.width);
  const [height, setHeight] = useState(config?.overlay?.height ?? DEFAULT_OVERLAY.height);
  const [tab, setTab] = useState<SettingsTab>('basic');

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
      pushToast?.(formatInvokeError?.(error) ?? String(error), 'error');
    }
  };

  const posText = (cfg?: OverlayConfigView) =>
    !cfg || cfg.x < 0 || cfg.y < 0
      ? t('set.overlayPos')
      : t('set.overlayPosAt', { x: cfg.x, y: cfg.y });

  return (
    <div className="settings-layout-wrap">
      <div className="settings-tabs">
        <button
          type="button"
          className={tab === 'basic' ? 'ghost tiny-btn active' : 'ghost tiny-btn'}
          onClick={() => setTab('basic')}
        >
          {t('set.tabBasic')}
        </button>
        <button
          type="button"
          className={tab === 'advanced' ? 'ghost tiny-btn active' : 'ghost tiny-btn'}
          onClick={() => setTab('advanced')}
        >
          {t('set.tabAdvanced')}
        </button>
      </div>

      {tab === 'basic' && (
        <div className="settings-layout">
          <div className="card settings-group switches">
            <h4>{t('set.language')}</h4>
            <p className="hint compact">{t('set.langHint')}</p>
            <label className="switch-line select-line">
              <span>{t('set.language')}</span>
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
              <span>{t('set.lan')}</span>
              <input type="checkbox" checked={Boolean(config?.allow_lan_access)} onChange={(e) => run(() => invoke('update_gateway_settings', { input: { allowLanAccess: e.target.checked } }), t('common.update'))} />
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
                  <span>{t('set.overlayShowTokens')}</span>
                  <input type="checkbox" checked={config.overlay.show_tokens} onChange={(e) => setOverlay({ show_tokens: e.target.checked })} />
                </label>
                <label className="switch-line">
                  <span>{t('set.overlayShowClient')}</span>
                  <input type="checkbox" checked={config.overlay.show_client} onChange={(e) => setOverlay({ show_client: e.target.checked })} />
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
                    max={160}
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

          <div className="card settings-group settings-group-wide settings-info">
            <h4>{t('set.endpoint')}</h4>
            <div className="hint compact settings-foot">
              {t('set.endpointOpenai')}<code>{statusListenUrl}/v1</code><br />
              {t('set.endpointAnthropic')}<code>{statusListenUrl}</code>
            </div>
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
                    await invoke('restore_experimental_zen');
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

      {proxyOpen && (
        <GithubProxyModal
          prefix={proxyPrefix}
          onClose={() => setProxyOpen(false)}
          onSaved={(prefix) => {
            setProxyPrefix(prefix);
            setProxyOpen(false);
          }}
          pushToast={pushToast ?? (() => undefined)}
          formatError={formatInvokeError ?? ((e) => String(e))}
        />
      )}
    </div>
  );
}

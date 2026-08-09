import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GithubProxyModal } from './store/GithubProxyModal';

type QuitBehavior = 'exit_only' | 'stop_gateway' | 'stop_all';

type AppConfigView = {
  autostart?: boolean;
  failover_enabled?: boolean;
  autostart_gateway?: boolean;
  quit_behavior?: QuitBehavior;
  port_fallback_enabled?: boolean;
  gateway_watchdog_enabled?: boolean;
  allow_lan_access?: boolean;
};

type Tab = 'dashboard' | 'models' | 'clients' | 'skills' | 'settings' | 'logs' | 'about';

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

  const openProxy = async () => {
    try {
      const settings = await invoke<{ editor_command: string; github_proxy_prefix: string }>('store_get_settings');
      setProxyPrefix(settings.github_proxy_prefix ?? '');
      setProxyOpen(true);
    } catch (error) {
      pushToast?.(formatInvokeError?.(error) ?? String(error), 'error');
    }
  };

  return (
    <div className="settings-layout">
      <div className="card settings-group switches">
        <h4>启动</h4>
        <p className="hint compact">控制应用与网关的开机行为</p>
        <label className="switch-line">
          <span>开机启动 SUGT</span>
          <input type="checkbox" checked={Boolean(config?.autostart)} onChange={(e) => run(() => invoke('set_autostart', { enabled: e.target.checked }), '已更新')} />
        </label>
        <label className="switch-line">
          <span>启动时自动开网关</span>
          <input type="checkbox" checked={Boolean(config?.autostart_gateway)} onChange={(e) => run(() => invoke('set_autostart_gateway', { enabled: e.target.checked }), '已更新')} />
        </label>
      </div>

      <div className="card settings-group switches">
        <h4>网关</h4>
        <p className="hint compact">可用性与故障恢复</p>
        <label className="switch-line">
          <span>故障转移</span>
          <input type="checkbox" checked={Boolean(config?.failover_enabled)} onChange={(e) => run(() => invoke('set_failover', { enabled: e.target.checked }), '已更新')} />
        </label>
        <label className="switch-line">
          <span>端口占用时自动换端口</span>
          <input type="checkbox" checked={Boolean(config?.port_fallback_enabled ?? true)} onChange={(e) => run(() => invoke('update_gateway_settings', { input: { portFallbackEnabled: e.target.checked } }), '已更新')} />
        </label>
        <label className="switch-line">
          <span>网关守护循环</span>
          <input type="checkbox" checked={Boolean(config?.gateway_watchdog_enabled ?? true)} onChange={(e) => run(() => invoke('update_gateway_settings', { input: { gatewayWatchdogEnabled: e.target.checked } }), '已更新')} />
        </label>
      </div>

      <div className="card settings-group settings-group-wide switches">
        <h4>网络与退出</h4>
        <p className="hint compact">局域网访问与托盘退出清理范围</p>
        <label className="switch-line">
          <span>允许局域网连接</span>
          <input type="checkbox" checked={Boolean(config?.allow_lan_access)} onChange={(e) => run(() => invoke('update_gateway_settings', { input: { allowLanAccess: e.target.checked } }), '已更新')} />
        </label>
        <label className="switch-line select-line">
          <span>托盘退出时</span>
          <select
            value={config?.quit_behavior ?? 'stop_all'}
            onChange={(e) => run(() => invoke('set_quit_behavior', { behavior: e.target.value as QuitBehavior }), '已更新')}
          >
            <option value="exit_only">退出并清理接管</option>
            <option value="stop_gateway">退出、停网关并清理接管</option>
            <option value="stop_all">退出、停网关并清理接管（推荐）</option>
          </select>
        </label>
      </div>

      <div className="card settings-group settings-group-wide">
        <h4>实验功能</h4>
        <p className="hint compact">OpenCode Zen 免费通道不稳定，可删可改；失效后可在此恢复默认实验源</p>
        <div className="settings-actions-row">
          <button
            type="button"
            className="ghost tiny-btn"
            onClick={() =>
              run(async () => {
                await invoke('restore_experimental_zen');
              }, '已恢复实验免费源')
            }
          >
            恢复实验·OpenCode Zen
          </button>
          <button type="button" className="ghost tiny-btn" onClick={() => onNavigate?.('models')}>
            前往模型配置
          </button>
        </div>
      </div>

      <div className="card settings-group settings-group-wide">
        <h4>技能商店</h4>
        <p className="hint compact">
          启动后会预热 Gitee 推荐库（swlgitee/sun-skills）。
          Anthropic 官方技能（含 frontend-design）在国内通常需先配置 GitHub 代理。
        </p>
        <div className="settings-actions-row">
          <button type="button" className="ghost tiny-btn" onClick={() => void openProxy()}>
            配置 GitHub 克隆代理
          </button>
          <button type="button" className="ghost tiny-btn" onClick={() => onNavigate?.('skills')}>
            打开技能页
          </button>
        </div>
      </div>

      <div className="card settings-group settings-group-wide">
        <h4>客户端接管</h4>
        <p className="hint compact">
          在「客户端」页可发现本机已安装的工具并一键接管；行内「高级」可管理技能、插件与配置路径。
        </p>
        <div className="settings-actions-row">
          <button type="button" className="ghost tiny-btn" onClick={() => onNavigate?.('clients')}>
            前往客户端
          </button>
        </div>
      </div>

      <div className="card settings-group settings-group-wide settings-info">
        <h4>本地接入点</h4>
        <div className="hint compact settings-foot">
          OpenAI 兼容：<code>{statusListenUrl}/v1</code><br />
          Anthropic：<code>{statusListenUrl}</code>
        </div>
      </div>

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

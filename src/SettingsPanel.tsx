import React, { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { GithubProxyModal } from './store/GithubProxyModal'

type QuitBehavior = 'exit_only' | 'stop_gateway' | 'stop_all'

// 流量悬浮窗配置（与后端 model.rs OverlayConfig 对应）
export type OverlayConfigView = {
  enabled: boolean
  opacity: number
  edit: boolean
  show_tokens: boolean
  show_client: boolean
  x: number
  y: number
  layout: 'column' | 'row'
  width: number
  height: number
}

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
}

// 切换布局方向时同步的推荐尺寸（前端只控制方向，尺寸仍可手动调节）
const LAYOUT_SIZE: Record<'column' | 'row', { width: number; height: number }> =
  {
    column: { width: 260, height: 76 },
    row: { width: 420, height: 48 },
  }

type AppConfigView = {
  autostart?: boolean
  failover_enabled?: boolean
  autostart_gateway?: boolean
  quit_behavior?: QuitBehavior
  port_fallback_enabled?: boolean
  gateway_watchdog_enabled?: boolean
  allow_lan_access?: boolean
  overlay?: OverlayConfigView
}

type Tab =
  | 'dashboard'
  | 'models'
  | 'clients'
  | 'skills'
  | 'settings'
  | 'logs'
  | 'about'
type SettingsTab = 'basic' | 'advanced'

type Props = {
  config: AppConfigView | null
  statusListenUrl?: string
  run: (fn: () => Promise<unknown>, ok?: string) => void
  onNavigate?: (tab: Tab) => void
  pushToast?: (message: string, type: 'ok' | 'error' | 'info') => void
  formatInvokeError?: (error: unknown) => string
}

export function SettingsPanel({
  config,
  statusListenUrl,
  run,
  onNavigate,
  pushToast,
  formatInvokeError,
}: Props) {
  const [proxyOpen, setProxyOpen] = useState(false)
  const [proxyPrefix, setProxyPrefix] = useState('')
  const [opacity, setOpacity] = useState(
    config?.overlay?.opacity ?? DEFAULT_OVERLAY.opacity,
  )
  const [width, setWidth] = useState(
    config?.overlay?.width ?? DEFAULT_OVERLAY.width,
  )
  const [height, setHeight] = useState(
    config?.overlay?.height ?? DEFAULT_OVERLAY.height,
  )
  const [tab, setTab] = useState<SettingsTab>('basic')

  // 悬浮窗配置更新：合并当前配置 + 增量，持久化并即时应用
  const setOverlay = (patch: Partial<OverlayConfigView>) => {
    const base = config?.overlay ?? DEFAULT_OVERLAY
    run(
      () => invoke('set_overlay_config', { cfg: { ...base, ...patch } }),
      '已更新',
    )
  }

  // 外部配置刷新后同步滑块显示值
  useEffect(() => {
    setOpacity(config?.overlay?.opacity ?? DEFAULT_OVERLAY.opacity)
    setWidth(config?.overlay?.width ?? DEFAULT_OVERLAY.width)
    setHeight(config?.overlay?.height ?? DEFAULT_OVERLAY.height)
  }, [
    config?.overlay?.opacity,
    config?.overlay?.width,
    config?.overlay?.height,
  ])

  // 切换横/竖排：同时应用对应推荐尺寸，让布局一眼可见
  const switchOverlayLayout = (layout: 'column' | 'row') => {
    const size = LAYOUT_SIZE[layout]
    setWidth(size.width)
    setHeight(size.height)
    setOverlay({ layout, width: size.width, height: size.height })
  }

  // 恢复默认：重置位置、透明度、布局与尺寸（保留启用/显示项开关）
  const resetOverlay = () => {
    setWidth(DEFAULT_OVERLAY.width)
    setHeight(DEFAULT_OVERLAY.height)
    setOpacity(DEFAULT_OVERLAY.opacity)
    setOverlay({
      opacity: DEFAULT_OVERLAY.opacity,
      x: DEFAULT_OVERLAY.x,
      y: DEFAULT_OVERLAY.y,
      layout: DEFAULT_OVERLAY.layout,
      width: DEFAULT_OVERLAY.width,
      height: DEFAULT_OVERLAY.height,
      edit: false,
    })
  }

  const openProxy = async () => {
    try {
      const settings = await invoke<{
        editor_command: string
        github_proxy_prefix: string
      }>('store_get_settings')
      setProxyPrefix(settings.github_proxy_prefix ?? '')
      setProxyOpen(true)
    } catch (error) {
      pushToast?.(formatInvokeError?.(error) ?? String(error), 'error')
    }
  }

  const posText = (cfg?: OverlayConfigView) =>
    !cfg || cfg.x < 0 || cfg.y < 0
      ? '未定位（自动右上）'
      : `当前位置 (${cfg.x}, ${cfg.y})`

  return (
    <div className="settings-layout-wrap">
      <div className="settings-tabs">
        <button
          type="button"
          className={
            tab === 'basic' ? 'ghost tiny-btn active' : 'ghost tiny-btn'
          }
          onClick={() => setTab('basic')}
        >
          基础
        </button>
        <button
          type="button"
          className={
            tab === 'advanced' ? 'ghost tiny-btn active' : 'ghost tiny-btn'
          }
          onClick={() => setTab('advanced')}
        >
          高级
        </button>
      </div>

      {tab === 'basic' && (
        <div className="settings-layout">
          <div className="card settings-group switches">
            <h4>启动</h4>
            <p className="hint compact">控制应用与网关的开机行为</p>
            <label className="switch-line">
              <span>开机启动 SUGT</span>
              <input
                type="checkbox"
                checked={Boolean(config?.autostart)}
                onChange={(e) =>
                  run(
                    () =>
                      invoke('set_autostart', { enabled: e.target.checked }),
                    '已更新',
                  )
                }
              />
            </label>
            <label className="switch-line">
              <span>启动时自动开网关</span>
              <input
                type="checkbox"
                checked={Boolean(config?.autostart_gateway)}
                onChange={(e) =>
                  run(
                    () =>
                      invoke('set_autostart_gateway', {
                        enabled: e.target.checked,
                      }),
                    '已更新',
                  )
                }
              />
            </label>
          </div>

          <div className="card settings-group switches">
            <h4>网关</h4>
            <p className="hint compact">可用性与故障恢复</p>
            <label className="switch-line">
              <span>故障转移</span>
              <input
                type="checkbox"
                checked={Boolean(config?.failover_enabled)}
                onChange={(e) =>
                  run(
                    () => invoke('set_failover', { enabled: e.target.checked }),
                    '已更新',
                  )
                }
              />
            </label>
            <label className="switch-line">
              <span>端口占用时自动换端口</span>
              <input
                type="checkbox"
                checked={Boolean(config?.port_fallback_enabled ?? true)}
                onChange={(e) =>
                  run(
                    () =>
                      invoke('update_gateway_settings', {
                        input: { portFallbackEnabled: e.target.checked },
                      }),
                    '已更新',
                  )
                }
              />
            </label>
            <label className="switch-line">
              <span>网关守护循环</span>
              <input
                type="checkbox"
                checked={Boolean(config?.gateway_watchdog_enabled ?? true)}
                onChange={(e) =>
                  run(
                    () =>
                      invoke('update_gateway_settings', {
                        input: { gatewayWatchdogEnabled: e.target.checked },
                      }),
                    '已更新',
                  )
                }
              />
            </label>
          </div>

          <div className="card settings-group switches">
            <h4>网络与退出</h4>
            <p className="hint compact">局域网访问与托盘退出清理范围</p>
            <label className="switch-line">
              <span>允许局域网连接</span>
              <input
                type="checkbox"
                checked={Boolean(config?.allow_lan_access)}
                onChange={(e) =>
                  run(
                    () =>
                      invoke('update_gateway_settings', {
                        input: { allowLanAccess: e.target.checked },
                      }),
                    '已更新',
                  )
                }
              />
            </label>
            <label className="switch-line select-line">
              <span>托盘退出时</span>
              <select
                value={config?.quit_behavior ?? 'stop_all'}
                onChange={(e) =>
                  run(
                    () =>
                      invoke('set_quit_behavior', {
                        behavior: e.target.value as QuitBehavior,
                      }),
                    '已更新',
                  )
                }
              >
                <option value="exit_only">退出并清理接管</option>
                <option value="stop_gateway">退出、停网关并清理接管</option>
                <option value="stop_all">退出、停网关并清理接管（推荐）</option>
              </select>
            </label>
          </div>

          <div className="card settings-group switches">
            <h4>流量悬浮窗</h4>
            {/* <p className="hint compact">置顶 + 半透明 + 鼠标穿透，实时显示 Token 交互</p> */}
            <label className="switch-line">
              <span>启用流量悬浮窗</span>
              <input
                type="checkbox"
                checked={Boolean(config?.overlay?.enabled)}
                onChange={(e) => setOverlay({ enabled: e.target.checked })}
              />
            </label>
            {config?.overlay?.enabled && (
              <>
                <label className="switch-line">
                  <span>显示 Token</span>
                  <input
                    type="checkbox"
                    checked={config.overlay.show_tokens}
                    onChange={(e) =>
                      setOverlay({ show_tokens: e.target.checked })
                    }
                  />
                </label>
                <label className="switch-line">
                  <span>显示调用来源</span>
                  <input
                    type="checkbox"
                    checked={config.overlay.show_client}
                    onChange={(e) =>
                      setOverlay({ show_client: e.target.checked })
                    }
                  />
                </label>
                <div className="switch-line">
                  <span>
                    透明度 {Math.round(config.overlay.opacity * 100)}%
                  </span>
                  <input
                    type="range"
                    min={0.1}
                    max={1}
                    step={0.05}
                    value={opacity}
                    onChange={(e) => setOpacity(Number(e.target.value))}
                    onMouseUp={(e) =>
                      setOverlay({ opacity: Number(e.currentTarget.value) })
                    }
                    onTouchEnd={(e) =>
                      setOverlay({ opacity: Number(e.currentTarget.value) })
                    }
                    onKeyUp={(e) => {
                      if (e.key.startsWith('Arrow'))
                        setOverlay({ opacity: Number(e.currentTarget.value) })
                    }}
                  />
                </div>
                <div className="switch-line">
                  <span>布局方向</span>
                  <div className="overlay-layout-switch">
                    <button
                      type="button"
                      className={`ghost tiny-btn${config.overlay.layout !== 'row' ? ' active' : ''}`}
                      onClick={() => switchOverlayLayout('column')}
                    >
                      竖排
                    </button>
                    <button
                      type="button"
                      className={`ghost tiny-btn${config.overlay.layout === 'row' ? ' active' : ''}`}
                      onClick={() => switchOverlayLayout('row')}
                    >
                      横排
                    </button>
                  </div>
                </div>
                <div className="switch-line">
                  <span>窗口宽度 {Math.round(width)}px</span>
                  <input
                    type="range"
                    min={200}
                    max={600}
                    step={10}
                    value={width}
                    onChange={(e) => setWidth(Number(e.target.value))}
                    onMouseUp={(e) =>
                      setOverlay({ width: Number(e.currentTarget.value) })
                    }
                    onTouchEnd={(e) =>
                      setOverlay({ width: Number(e.currentTarget.value) })
                    }
                    onKeyUp={(e) => {
                      if (e.key.startsWith('Arrow'))
                        setOverlay({ width: Number(e.currentTarget.value) })
                    }}
                  />
                </div>
                <div className="switch-line">
                  <span>窗口高度 {Math.round(height)}px</span>
                  <input
                    type="range"
                    min={48}
                    max={160}
                    step={4}
                    value={height}
                    onChange={(e) => setHeight(Number(e.target.value))}
                    onMouseUp={(e) =>
                      setOverlay({ height: Number(e.currentTarget.value) })
                    }
                    onTouchEnd={(e) =>
                      setOverlay({ height: Number(e.currentTarget.value) })
                    }
                    onKeyUp={(e) => {
                      if (e.key.startsWith('Arrow'))
                        setOverlay({ height: Number(e.currentTarget.value) })
                    }}
                  />
                </div>
                <div className="settings-actions-row">
                  <button
                    type="button"
                    className="ghost tiny-btn"
                    onClick={() => setOverlay({ edit: true })}
                  >
                    移动悬浮窗
                  </button>
                  <button
                    type="button"
                    className="ghost tiny-btn"
                    onClick={resetOverlay}
                  >
                    恢复默认
                  </button>
                  <span className="hint compact">
                    {posText(config.overlay)}
                    {/* ；进入后窗口可拖动，拖到目标位置再点悬浮窗上的「保存」。 */}
                  </span>
                </div>
              </>
            )}
          </div>

          <div className="card settings-group settings-group-wide settings-info">
            <h4>本地接入点</h4>
            <div className="hint compact settings-foot">
              OpenAI 兼容：<code>{statusListenUrl}/v1</code>
              <br />
              Anthropic：<code>{statusListenUrl}</code>
            </div>
          </div>
        </div>
      )}

      {tab === 'advanced' && (
        <div className="settings-layout">
          <div className="card settings-group">
            <h4>实验功能</h4>
            <p className="hint compact">
              OpenCode Zen 免费通道不稳定，可删可改；失效后可在此恢复默认实验源
            </p>
            <div className="settings-actions-row">
              <button
                type="button"
                className="ghost tiny-btn"
                onClick={() =>
                  run(async () => {
                    await invoke('restore_experimental_zen')
                  }, '已恢复实验免费源')
                }
              >
                恢复实验·OpenCode Zen
              </button>
              <button
                type="button"
                className="ghost tiny-btn"
                onClick={() => onNavigate?.('models')}
              >
                前往模型配置
              </button>
            </div>
          </div>

          <div className="card settings-group">
            <h4>客户端接管</h4>
            <p className="hint compact">
              在「客户端」页可发现本机已安装的工具并一键接管；行内「高级」可管理技能、插件与配置路径。
            </p>
            <div className="settings-actions-row">
              <button
                type="button"
                className="ghost tiny-btn"
                onClick={() => onNavigate?.('clients')}
              >
                前往客户端
              </button>
            </div>
          </div>

          <div className="card settings-group settings-group-wide">
            <h4>技能商店</h4>
            <p className="hint compact">
              启动后会预热 Gitee 推荐库（swlgitee/sun-skills）。 Anthropic
              官方技能（含 frontend-design）在国内通常需先配置 GitHub 代理。
            </p>
            <div className="settings-actions-row">
              <button
                type="button"
                className="ghost tiny-btn"
                onClick={() => void openProxy()}
              >
                配置 GitHub 克隆代理
              </button>
              <button
                type="button"
                className="ghost tiny-btn"
                onClick={() => onNavigate?.('skills')}
              >
                打开技能页
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
            setProxyPrefix(prefix)
            setProxyOpen(false)
          }}
          pushToast={pushToast ?? (() => undefined)}
          formatError={formatInvokeError ?? ((e) => String(e))}
        />
      )}
    </div>
  )
}

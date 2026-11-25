import { lazy } from 'react';

// 面板级代码分割：首屏只加载控制台真正需要的组件（TrafficPanel / GatewayAccessBanner
// / StatusBar），其余面板各自独立 chunk，首屏 paint 后在空闲时静默预取。
// 点击切换时模块已在内存里，不出现白屏或骨架闪烁。
//
// 注意：这里的 loader 必须与 lazy 共用同一个函数，浏览器模块缓存保证预取和
// 实际挂载只下载一次。

const loadSettingsPanel = () => import('./SettingsPanel').then((m) => ({ default: m.SettingsPanel }));
const loadSkillsPanel = () => import('./SkillsPanel').then((m) => ({ default: m.SkillsPanel }));
const loadModelListPanel = () => import('./ModelListPanel').then((m) => ({ default: m.ModelListPanel }));
const loadStrategyBoxPanel = () => import('./StrategyBoxPanel').then((m) => ({ default: m.StrategyBoxPanel }));
const loadApiKeyPoolPanel = () => import('./ApiKeyPoolPanel').then((m) => ({ default: m.ApiKeyPoolPanel }));
const loadClientKeyPanel = () => import('./ClientKeyPanel').then((m) => ({ default: m.ClientKeyPanel }));
const loadWorkspaceRulesPanel = () => import('./WorkspaceRulesPanel').then((m) => ({ default: m.WorkspaceRulesPanel }));
const loadRequestRulesSection = () => import('./RequestRulesSection').then((m) => ({ default: m.RequestRulesSection }));
const loadAccessInfoCard = () => import('./AccessInfoCard').then((m) => ({ default: m.AccessInfoCard }));
const loadMePanel = () => import('./MePanel').then((m) => ({ default: m.MePanel }));
const loadTakeoverProfilesSection = () =>
  import('./TakeoverProfilesSection').then((m) => ({ default: m.TakeoverProfilesSection }));

export const SettingsPanel = lazy(loadSettingsPanel);
export const SkillsPanel = lazy(loadSkillsPanel);
export const ModelListPanel = lazy(loadModelListPanel);
export const StrategyBoxPanel = lazy(loadStrategyBoxPanel);
export const ApiKeyPoolPanel = lazy(loadApiKeyPoolPanel);
export const ClientKeyPanel = lazy(loadClientKeyPanel);
export const WorkspaceRulesPanel = lazy(loadWorkspaceRulesPanel);
export const RequestRulesSection = lazy(loadRequestRulesSection);
export const AccessInfoCard = lazy(loadAccessInfoCard);
export const MePanel = lazy(loadMePanel);
export const TakeoverProfilesSection = lazy(loadTakeoverProfilesSection);

/** 预取顺序：按日常使用频率排，先 warm 最常去的页面。 */
const PRELOAD_ORDER: Array<() => Promise<unknown>> = [
  loadSettingsPanel,
  loadModelListPanel,
  loadMePanel,
  loadApiKeyPoolPanel,
  loadStrategyBoxPanel,
  loadClientKeyPanel,
  loadWorkspaceRulesPanel,
  loadRequestRulesSection,
  loadSkillsPanel,
  loadTakeoverProfilesSection,
  loadAccessInfoCard,
];

let preloadStarted = false;

/**
 * 首屏 paint 之后在空闲时段静默预取全部面板 chunk。
 * 单个失败不影响功能——真正挂载时 lazy 会再取一次。
 */
export function preloadPanels(): void {
  if (preloadStarted) return;
  preloadStarted = true;

  const run = () => {
    void (async () => {
      for (const load of PRELOAD_ORDER) {
        try {
          await load();
        } catch {
          // 预取失败静默忽略，用户点开时仍会正常加载或给出真实错误。
        }
      }
    })();
  };

  const ric = (window as unknown as { requestIdleCallback?: (cb: () => void, opts?: { timeout: number }) => number })
    .requestIdleCallback;
  if (typeof ric === 'function') {
    ric(run, { timeout: 1500 });
  } else {
    window.setTimeout(run, 240);
  }
}

/** 悬停/聚焦导航时提前拉取对应面板，进一步降低首次点击延迟。 */
export function preloadPanelPreview(
  key: 'overview' | 'settings' | 'models' | 'soul' | 'gateway' | 'rules' | 'access' | 'skills',
): void {
  const map: Record<string, Array<() => Promise<unknown>>> = {
    overview: [],
    settings: [loadSettingsPanel],
    models: [loadModelListPanel, loadStrategyBoxPanel, loadApiKeyPoolPanel],
    soul: [loadMePanel],
    gateway: [loadApiKeyPoolPanel, loadStrategyBoxPanel],
    rules: [loadWorkspaceRulesPanel, loadRequestRulesSection],
    access: [loadClientKeyPanel, loadTakeoverProfilesSection, loadAccessInfoCard],
    skills: [loadSkillsPanel],
  };
  for (const load of map[key] ?? []) {
    void load().catch(() => undefined);
  }
}

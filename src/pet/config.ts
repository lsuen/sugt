// 桌面精灵（小酥）前端共享类型、默认值与工具函数。
// SettingsPanel / overlay / PetStage 统一从这里取，避免与后端 model.rs 的 OverlayConfig 三处重复。
import type { TKey } from '../i18n';

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
  // 桌面精灵模式：与后端 OverlayConfig 对应
  pet_enabled: boolean;
  pet_interactive: boolean;
  pet_snap: boolean;
  pet_sleep_after_secs: number;
  pet_bubble_ms: number;
  pet_gaze: boolean;
  pet_blink: boolean;
  pet_quiet: boolean;
  pet_always_interactive: boolean;
  pet_roam: boolean;
};

// -1 表示「未定位」，由后端 overlay::apply 创建时按模式落点（精灵右下角）
// 产品默认：开悬浮窗 + 开小酥 + 全功能互动
export const DEFAULT_OVERLAY: OverlayConfigView = {
  enabled: true,
  opacity: 0.72,
  edit: false,
  show_tokens: true,
  show_client: true,
  show_channel: true,
  show_requests: true,
  x: -1,
  y: -1,
  layout: 'column',
  width: 148,
  height: 168,
  pet_enabled: true,
  pet_interactive: true,
  pet_snap: true,
  pet_sleep_after_secs: 300,
  pet_bubble_ms: 4000,
  pet_gaze: true,
  pet_blink: true,
  pet_quiet: false,
  pet_always_interactive: true,
  pet_roam: true,
};

export type TrafficSlice = {
  today_requests: number;
  total_requests: number;
  success_count: number;
  failed_count: number;
  today_input_tokens: number;
  today_output_tokens: number;
  // 真实在途 LLM 请求数；健康检查 / 看门狗心跳 / 免费通道探测不计入
  active_requests?: number;
};

export type OverlayStatus = {
  running: boolean;
  last_proxy_client?: string | null;
  last_proxy_provider?: string | null;
  last_proxy_path?: string | null;
  last_proxy_failover?: boolean;
  traffic: TrafficSlice;
};

export type Bubble = { id: number; text: string; kind: 'ok' | 'error' | 'info' };

// 紧凑三档：贴合立绘，减少透明空框；宽高比约 7:8（立绘 2:3 + 窄侧栏）。
export const PET_SIZES = [
  { key: 'small', label: 'set.petSizeSmall' as TKey, width: 148, height: 168 },
  { key: 'medium', label: 'set.petSizeMedium' as TKey, width: 180, height: 204 },
  { key: 'large', label: 'set.petSizeLarge' as TKey, width: 220, height: 250 },
] as const;

export const PET_SIZE_STORAGE_KEY = 'sugt.pet.size.v2';

export const readPetSize = (): (typeof PET_SIZES)[number] => {
  try {
    const saved = window.localStorage.getItem(PET_SIZE_STORAGE_KEY);
    return PET_SIZES.find((size) => size.key === saved) ?? PET_SIZES[0];
  } catch {
    return PET_SIZES[0];
  }
};

export const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

// 走动缓动：首尾慢、中段快，避免窗口位置瞬移。
export const easeInOut = (p: number) =>
  p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2;

/** 判断光标（窗口内 CSS 像素）是否落在立绘命中框内；用于元素级穿透切换。 */
export function pointInsideFigure(
  clientX: number,
  clientY: number,
  rect: DOMRect,
): boolean {
  return (
    clientX >= rect.left
    && clientX <= rect.right
    && clientY >= rect.top
    && clientY <= rect.bottom
  );
}

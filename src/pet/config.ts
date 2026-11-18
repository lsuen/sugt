// 桌面精灵（小酥）前端共享类型、默认值与工具函数。
// 流量 HUD 悬浮窗已废弃：本文件只描述精灵一个功能域，不再有模式切换与回退尺寸。
// 物理标识（窗口 label / overlay.html / get|set_overlay_config）保持不动，
// 让拖动、右键、视线、光标等既有 IPC 链路零改动。
import type { TKey } from '../i18n';
import { PET_IMAGE_SIZE } from './petManifest.generated';

export type OverlayConfigView = {
  enabled: boolean;
  opacity: number;
  edit: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  // 桌面精灵行为开关：与后端 model.rs::OverlayConfig 对应
  pet_interactive: boolean;
  pet_snap: boolean;
  pet_sleep_after_secs: number;
  pet_bubble_ms: number;
  pet_gaze: boolean;
  pet_blink: boolean;
  pet_quiet: boolean;
  pet_always_interactive: boolean;
  pet_roam: boolean;
  // 鼠标穿透：默认关闭（整窗可交互，拖拽/右键可用）；开启后退回元素级动态穿透
  pet_click_through: boolean;
};

// -1 表示「未定位」，由后端 overlay::apply 创建时落到主屏右下角
// 产品默认：开小酥 + 全功能互动 + 不穿透
export const DEFAULT_OVERLAY: OverlayConfigView = {
  enabled: true,
  // 精灵默认不透明；该字段只作用于立绘，不再有 HUD 底板
  opacity: 1,
  edit: false,
  x: -1,
  y: -1,
  width: 198,
  height: 168,
  pet_interactive: true,
  pet_snap: true,
  pet_sleep_after_secs: 300,
  pet_bubble_ms: 4000,
  pet_gaze: true,
  pet_blink: true,
  pet_quiet: false,
  pet_always_interactive: true,
  pet_roam: true,
  pet_click_through: false,
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

// v1.1 三档：旧「大」(220×250) 废弃，旧中降为新大、旧小降为新中，新增更小的小档。
// v1.4 画布 240×297（左右各补 30px 全透明）；三档窗口按 4/3 加宽保持 contain 高度主导，
// 人物屏幕像素高不变，宽出的部分全是透明边、不挡点击。
export const PET_SIZES = [
  { key: 'small', label: 'set.petSizeSmall' as TKey, width: 160, height: 136 },
  { key: 'medium', label: 'set.petSizeMedium' as TKey, width: 198, height: 168 },
  { key: 'large', label: 'set.petSizeLarge' as TKey, width: 240, height: 204 },
] as const;

// 沿用 v2 key：老存档 medium/large 自然落到「新中/新大」，正好是档位整体下移。
export const PET_SIZE_STORAGE_KEY = 'sugt.pet.size.v2';

/** 无存档时的默认档：中号。 */
export const DEFAULT_PET_SIZE = PET_SIZES[1];

export const readPetSize = (): (typeof PET_SIZES)[number] => {
  try {
    const saved = window.localStorage.getItem(PET_SIZE_STORAGE_KEY);
    return PET_SIZES.find((size) => size.key === saved) ?? DEFAULT_PET_SIZE;
  } catch {
    return DEFAULT_PET_SIZE;
  }
};

export type PetSizePreset = (typeof PET_SIZES)[number];

export function writePetSize(size: PetSizePreset): void {
  try {
    window.localStorage.setItem(PET_SIZE_STORAGE_KEY, size.key);
  } catch {
    // 隐私模式无 localStorage 时仍应用本次尺寸
  }
}

/** 只改立绘档位。停靠才动坐标。 */
export function petSizePatch(size: PetSizePreset): Pick<OverlayConfigView, 'width' | 'height'> {
  writePetSize(size);
  return { width: size.width, height: size.height };
}

/**
 * 精灵启用开关只写 `{ enabled }`：窗口尺寸、落点、不透明度与行为开关
 * 全部留给用户在各自设置项里独立维护，切换启用态不再连带改写。
 * HUD 废弃前的模式补丁会连带强改这些字段，已随之删除。
 */
export const petEnabledPatch = (enabled: boolean): Pick<OverlayConfigView, 'enabled'> => ({
  enabled,
});

export const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

// 走动缓动：首尾慢、中段快，避免窗口位置瞬移。
export const easeInOut = (p: number) =>
  p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2;

export type FigureRect = {
  left: number;
  right: number;
  top: number;
  bottom: number;
};

/** 判断光标（窗口内 CSS 像素）是否落在立绘命中框内；用于元素级穿透切换。 */
export function pointInsideFigure(
  clientX: number,
  clientY: number,
  rect: FigureRect,
  pad = 0,
): boolean {
  return (
    clientX >= rect.left - pad
    && clientX <= rect.right + pad
    && clientY >= rect.top - pad
    && clientY <= rect.bottom + pad
  );
}

/** 全图兜底：姿势未识别时退回整张立绘，避免误把整个人物判为穿透。 */
export const FULL_HITBOX = [0, 0, 1, 1] as const;

/**
 * 把素材内归一化的 alpha 包围盒映射到实际绘制区。
 * 立绘是 background-size: contain + center/bottom 对齐（v1.3.1 起水平居中，
 * 让 scaleX 镜像后左右对称、四周都有安全边），所以先算等比缩放后的绘制矩形，
 * 再叠加 bbox 与外扩 pad；命中区只覆盖有像素的部分，透明边距不挡点击。
 * 传入的 rect 必须是 .pet-figure 的实际盒子（已含 --pet-inset-* 内缩）。
 * 默认尺寸与 manifest.baseSize（裁剪后画布）保持一致。
 */
export function figureHitRect(
  rect: FigureRect,
  hitBox: readonly [number, number, number, number],
  // 画布尺寸唯一真相源：manifest.json → petManifest.generated.ts，禁止再写第二份。
  image: { w: number; h: number } = PET_IMAGE_SIZE,
  pad = 0,
): FigureRect {
  const width = rect.right - rect.left;
  const height = rect.bottom - rect.top;
  const factor = Math.min(width / image.w, height / image.h);
  const drawW = image.w * factor;
  const drawH = image.h * factor;
  // center bottom：绘制矩形水平居中、垂直贴底，与 pet.css 的
  // background-position 保持一致；写成 rect.right - drawW 会让命中框整体偏左。
  const originX = rect.left + (width - drawW) / 2;
  const originY = rect.bottom - drawH;
  const left = originX + hitBox[0] * drawW - pad;
  const top = originY + hitBox[1] * drawH - pad;
  return {
    left,
    top,
    right: originX + hitBox[2] * drawW + pad,
    bottom: originY + hitBox[3] * drawH + pad,
  };
}

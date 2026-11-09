// 桌面精灵（小酥）状态机纯函数：与 React / Tauri 解耦，便于单测与复用。
// 网关态 + 交互叠加姿分开，避免轻碰抢掉真实 typing。

export type PetState = 'offline' | 'idle' | 'typing' | 'done' | 'error' | 'sleep';

/** 立绘可用的动画层（含交互/表情差分，不计入状态胶囊文案） */
export type PetPose =
  | PetState
  | 'wave'
  | 'peek'
  | 'shy'
  | 'celebrate'
  | 'walk';

export type PetHold = { state: PetState; until: number };

/** 休眠阈值（毫秒）；配置缺省或非法时回退 5 分钟 */
export function petSleepAfterMs(secs?: number | null): number {
  const n = Number(secs);
  return Number.isFinite(n) && n > 0 ? n * 1000 : 300_000;
}

/** 气泡展示时长（毫秒）；缺省 4s */
export function petBubbleMs(ms?: number | null): number {
  const n = Number(ms);
  return Number.isFinite(n) && n > 0 ? n : 4000;
}

/**
 * 由网关轮询样本推导当前立绘态。
 * 优先级：offline → typing（含余晖）→ hold(error/done) → sleep → idle。
 */
export function derivePetState(input: {
  running: boolean;
  active: number;
  hold: PetHold;
  typingUntil: number;
  lastActivity: number;
  sleepAfterMs: number;
  now: number;
}): PetState {
  const { running, active, hold, typingUntil, lastActivity, sleepAfterMs, now } = input;
  if (!running) return 'offline';
  if (active > 0 || typingUntil > now) return 'typing';
  if (hold.state === 'error' && hold.until > now) return 'error';
  if (hold.state === 'done' && hold.until > now) return 'done';
  if (now - lastActivity > sleepAfterMs) return 'sleep';
  return 'idle';
}

/**
 * 立绘实际播放哪一层：表情/交互只叠在合适基态上，忙碌 typing 不被打扰。
 * 优先级：celebrate(done) → 轻碰/摸头 → 开场招呼 → 闲置互动 → 自主走动
 *        → peek(靠近) → walk(靠近未贴脸) → 基态。
 * v1.0 新增 greeting / idlePose / roaming 三个可选差分源，缺省时行为与旧版完全一致。
 */
export function resolvePetPose(input: {
  state: PetState;
  poked: boolean;
  shy: boolean;
  listening: boolean;
  walking: boolean;
  celebrating: boolean;
  greeting?: boolean;
  idlePose?: PetPose | null;
  roaming?: boolean;
}): PetPose {
  const {
    state, poked, shy, listening, walking, celebrating,
    greeting, idlePose, roaming,
  } = input;
  if (celebrating && (state === 'done' || state === 'idle')) return 'celebrate';
  if (state === 'idle' || state === 'offline') {
    if (shy) return 'shy';
    if (poked) return 'wave';
    if (greeting && state === 'idle') return 'wave';
    if (idlePose && state === 'idle') return idlePose;
    if (roaming && state === 'idle') return 'walk';
    if (listening && state === 'idle') return 'peek';
    if (walking && state === 'idle') return 'walk';
  }
  if (state === 'offline') return 'idle';
  return state;
}

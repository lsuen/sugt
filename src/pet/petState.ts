// 桌面精灵（小酥）状态机纯函数：与 React / Tauri 解耦，便于单测与复用。
// 声明式优先级表（行序即优先级，首个命中即返回）+ 纯 reducer，取代原先的 if 链。
// 网关态 + 自动交互叠加姿分开，避免轻碰抢掉真实 typing。
// v1.1：参考 whale-girl 的「零打扰自动交互」——随机眨眼/转身/闲置差分/游走、
// 睡醒过渡、请求完成庆祝、失败失落、拖拽态；节奏参数由 manifest 单一真相源注入。
import { PET_AUTO_ACTIONS } from './petManifest.generated';

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

// ---- 自动交互节奏器（纯函数：now / random 显式注入，可确定性单测）----

export const AUTO = PET_AUTO_ACTIONS;

export type RhythmKind = 'blink' | 'facing' | 'idleVariant' | 'wander';

/** 按 manifest 的 [minMs, maxMs] 取随机间隔；注入随机源即可单测。 */
export function rhythmDelay(kind: RhythmKind, random: () => number = Math.random): number {
  const cfg = AUTO[kind];
  return cfg.minMs + random() * (cfg.maxMs - cfg.minMs);
}

export function nextBlinkAt(
  { now, random = Math.random }: { now: number; random?: () => number },
): number {
  return now + rhythmDelay('blink', random);
}

export function nextFacingAt(
  { now, random = Math.random }: { now: number; random?: () => number },
): number {
  return now + rhythmDelay('facing', random);
}

export function nextIdleVariantAt(
  { now, random = Math.random }: { now: number; random?: () => number },
): number {
  return now + rhythmDelay('idleVariant', random);
}

export function nextWanderAt(
  { now, random = Math.random }: { now: number; random?: () => number },
): number {
  return now + rhythmDelay('wander', random);
}

/** 从 manifest 允许的闲置差分姿里随机挑一个；空列表回退 peek。 */
export function pickIdleVariant(
  { random = Math.random }: { random?: () => number } = {},
): PetPose {
  const list = AUTO.idleVariant.poses as readonly PetPose[];
  return list.length ? list[Math.floor(random() * list.length)] : 'peek';
}

/** 睡醒边沿：上一帧是 sleep、本帧离开 sleep，且不是拖拽/已有醒觉动作打断。 */
export function shouldWake(
  prevState: PetState,
  nextState: PetState,
  ctx: { dragging?: boolean; transient?: boolean } = {},
): boolean {
  return prevState === 'sleep' && nextState !== 'sleep'
    && !ctx.dragging && !ctx.transient;
}

/** 用户交互醒觉：重置空闲计时；视觉上正在睡才播 wake 过渡。 */
export function wakeFromInteraction(
  { visuallySleeping }: { visuallySleeping: boolean },
): { sleeping: boolean; wake: boolean } {
  return { sleeping: false, wake: visuallySleeping };
}

/** 拖拽横向位移决定朝向；纯纵向移动保留当前朝向。 */
export function facingFromDrag(
  deltaX: number,
  current: 'left' | 'right',
): 'left' | 'right' {
  return deltaX === 0 ? current : deltaX < 0 ? 'left' : 'right';
}

// ---- 网关态 ----

export type PetStateInput = {
  running: boolean;
  active: number;
  hold: PetHold;
  typingUntil: number;
  lastActivity: number;
  sleepAfterMs: number;
  now: number;
};

export type PetStateRule = {
  id: string;
  state: PetState;
  when: (input: PetStateInput) => boolean;
};

/**
 * 网关态优先级表：offline → typing（含余晖）→ error → done → sleep → idle。
 * 与 src/assets/pet/manifest.json 的 statePriority 同序；新增状态只插一行。
 */
export const PET_STATE_TABLE: readonly PetStateRule[] = [
  { id: 'offline', state: 'offline', when: (i) => !i.running },
  { id: 'typing', state: 'typing', when: (i) => i.active > 0 || i.typingUntil > i.now },
  {
    id: 'error', state: 'error',
    when: (i) => i.hold.state === 'error' && i.hold.until > i.now,
  },
  {
    id: 'done', state: 'done',
    when: (i) => i.hold.state === 'done' && i.hold.until > i.now,
  },
  {
    id: 'sleep', state: 'sleep',
    when: (i) => i.now - i.lastActivity > i.sleepAfterMs,
  },
  { id: 'idle', state: 'idle', when: () => true },
];

/** 查表：首个命中即返回，最后一行 idle 保证永远有结果。 */
export function selectPetState(input: PetStateInput): PetState {
  const row = PET_STATE_TABLE.find((rule) => rule.when(input));
  return row ? row.state : 'idle';
}

// ---- 立绘 / 自动交互动作 ----

export type PetPoseInput = {
  state: PetState;
  poked: boolean;
  shy: boolean;
  listening: boolean;
  walking: boolean;
  celebrating: boolean;
  greeting?: boolean;
  idlePose?: PetPose | null;
  roaming?: boolean;
  /** v1.1 自动交互：拖拽 / 睡醒 / 欢迎 / 等待批准 / 失败失落尾段 */
  dragging?: boolean;
  waking?: boolean;
  welcoming?: boolean;
  waiting?: boolean;
  disappointed?: boolean;
};

export type PetPoseRule = {
  id: string;
  pose: PetPose | ((input: PetPoseInput) => PetPose);
  when: (input: PetPoseInput) => boolean;
};

/**
 * 立绘层优先级表（行序即优先级，与 manifest.posePriority 同序）：
 * 事件反馈 > 用户互动 > 自动陪伴 > 基态。
 * 差分只在 idle/offline/sleep 生效；typing/error/done 等真实任务态不抢戏。
 * 素材复用说明：drag 复用 walk、wake/wait 复用 peek、disappointed 复用 shy，
 * 待 1000 帧真动作素材落包后逐项换成专属姿势，触发逻辑不变。
 */
export const PET_POSE_TABLE: readonly PetPoseRule[] = [
  { id: 'drag', pose: 'walk', when: (i) => Boolean(i.dragging) },
  {
    id: 'wake', pose: 'peek',
    when: (i) => Boolean(i.waking) && (i.state === 'idle' || i.state === 'offline'),
  },
  {
    id: 'welcome', pose: 'wave',
    when: (i) => Boolean(i.welcoming) && (i.state === 'idle' || i.state === 'offline'),
  },
  {
    id: 'celebrate', pose: 'celebrate',
    when: (i) => i.celebrating && (i.state === 'done' || i.state === 'idle'),
  },
  {
    id: 'disappointed', pose: 'shy',
    when: (i) => Boolean(i.disappointed) && i.state === 'error',
  },
  {
    id: 'shy', pose: 'shy',
    when: (i) => (i.state === 'idle' || i.state === 'offline') && i.shy,
  },
  {
    id: 'wave', pose: 'wave',
    when: (i) => (i.state === 'idle' || i.state === 'offline') && i.poked,
  },
  { id: 'greeting', pose: 'wave', when: (i) => i.state === 'idle' && Boolean(i.greeting) },
  {
    id: 'wait', pose: 'peek',
    when: (i) => Boolean(i.waiting) && (i.state === 'idle' || i.state === 'typing'),
  },
  {
    id: 'idleVariant', pose: (i) => i.idlePose as PetPose,
    when: (i) => i.state === 'idle' && Boolean(i.idlePose),
  },
  { id: 'roaming', pose: 'walk', when: (i) => i.state === 'idle' && Boolean(i.roaming) },
  { id: 'peek', pose: 'peek', when: (i) => i.state === 'idle' && i.listening },
  { id: 'walk', pose: 'walk', when: (i) => i.state === 'idle' && i.walking },
];

/** 兼容包装：由表推导立绘实际播放哪一层。offline 视觉复用 idle。 */
export function resolvePetPose(input: PetPoseInput): PetPose {
  const row = PET_POSE_TABLE.find((rule) => rule.when(input));
  if (row) {
    return typeof row.pose === 'function' ? row.pose(input) : row.pose;
  }
  return input.state === 'offline' ? 'idle' : input.state;
}

/** 兼容包装：由表推导当前立绘态。 */
export function derivePetState(input: PetStateInput): PetState {
  return selectPetState(input);
}

const DEFAULT_POSE: Omit<PetPoseInput, 'state'> = {
  poked: false,
  shy: false,
  listening: false,
  walking: false,
  celebrating: false,
};

export type PetReducerInput = {
  gateway: PetStateInput;
  pose?: Partial<PetPoseInput>;
  now?: number;
};

export type PetReducerResult = {
  state: PetState;
  pose: PetPose;
  /** 事件签名：重复同类事件靠它去重，不重播动作起点 */
  signature: string;
  /** 结果态保持时长；非结果态为 0 */
  holdMs: number;
  bubble: 'ok' | 'error' | null;
};

/**
 * 纯 reducer：网关事实 + 交互意图 → 状态 / 立绘 / 签名 / 气泡类别。
 * 不读 Date.now、不设定时器，时间由调用方显式注入，便于确定性单测与将来事件化。
 */
export function reducePetState(input: PetReducerInput): PetReducerResult {
  const now = input.now ?? input.gateway.now;
  const gateway = { ...input.gateway, now };
  const state = selectPetState(gateway);
  const pose = resolvePetPose({ ...DEFAULT_POSE, ...input.pose, state });
  const holdMs = state === 'error' ? 6000 : state === 'done' ? 3500 : 0;
  return {
    state,
    pose,
    signature: `${state}|${pose}|${gateway.hold.state}:${gateway.hold.until}`,
    holdMs,
    bubble: state === 'done' ? 'ok' : state === 'error' ? 'error' : null,
  };
}

// 桌面精灵（小酥）状态机纯函数：与 React / Tauri 解耦，便于单测与复用。
// v1.3：立绘动作改为「注册表驱动」——PET_ACTIONS 由 manifest.json 的 poses[]
// 生成，按 priority 降序查表；新增动作只在 manifest 追加一条并放两张 WebP，
// 本文件、PetStage.tsx、pet.css 一律零改动。
// 网关态 + 交互/自动动作分开，避免轻碰抢掉真实 typing；纯 reducer 不读 Date.now。
import {
  eventActionFor,
  IDLE_VARIANT_WEIGHTS,
  weightedIdleVariant,
  type PetEvent,
} from '../notify/messageModel';
import {
  PET_ACTIONS,
  PET_AUTO_ACTIONS,
  PET_FRAME_MS,
  type PetPoseName,
} from './petManifest.generated';

// 动作注册表同样由状态机对外暴露：调用方/单测无需再直接依赖生成物。
export { PET_ACTIONS } from './petManifest.generated';

export type PetState = 'offline' | 'idle' | 'typing' | 'done' | 'error' | 'sleep';

/** 立绘动作名唯一真相源：src/assets/pet/manifest.json → petManifest.generated.ts */
export type PetPose = PetPoseName;

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

// ---- 动作时长派生（v1.5：不再手写毫秒，避免「动画没播完就切」）----
//
// 素材是自带逐帧时长的 WebP 动图，浏览器按文件内时长播放；只有把 PET_FRAME_MS
// 求和，才知道一个动作真正播完需要多久。历史上 autoActions 里写死 2200ms，
// 而闲置动作实际 3924~5018ms，于是每次只播一半就被 setIdlePose(null) 切回。
// 这里改为「完整遍数 × 单遍时长 + 尾部停留」，换素材后自动跟随。
// 作者：孙文龙 sun

/** 单个姿势完整播放一遍的时长（毫秒）。 */
export function poseDurationMs(pose: PetPose): number {
  const frames = PET_FRAME_MS[pose as PetPoseName];
  if (!frames || !frames.length) return 0;
  return frames.reduce((sum, ms) => sum + ms, 0);
}

export type PoseHoldOptions = {
  /** 完整播放遍数，默认 1；允许小数（0.5 = 只取前半段，非必要不用）。 */
  cycles?: number;
  /** 动作播完后的定格停留，默认 0。 */
  tailMs?: number;
  /** 显式覆盖（毫秒）：仅供确有「刻意短播」理由的调用点，需在调用处写明原因。 */
  overrideMs?: number;
};

/** manifest.autoActions 里「播几遍 + 尾部停留」的槽位，毫秒由素材时长派生。 */
export type AutoPoseSlot = {
  pose: PetPose;
  holdCycles?: number;
  tailMs?: number;
};

export function autoPoseHold(slot: AutoPoseSlot): number {
  return poseHoldMs(slot.pose, {
    cycles: slot.holdCycles ?? 1,
    tailMs: slot.tailMs ?? 0,
  });
}

/**
 * 派生一次动作的视觉停留时长。
 * 期望：任何动作都至少完整播完 cycles 遍，再由调用方加尾部停留；
 * 因为「播到一半切走」比停留久一点更伤观感。
 */
export function poseHoldMs(pose: PetPose, opts: PoseHoldOptions = {}): number {
  const { cycles = 1, tailMs = 0, overrideMs } = opts;
  if (typeof overrideMs === 'number' && Number.isFinite(overrideMs) && overrideMs > 0) {
    return Math.round(overrideMs);
  }
  const single = poseDurationMs(pose);
  if (!single) return 0;
  const safeCycles = Math.max(0.1, cycles);
  const safeTail = Math.max(0, tailMs);
  return Math.round(single * safeCycles + safeTail);
}

/**
 * 结果态（done / error）的完整停留窗口。
 * 期望窗口覆盖「主结果动作 + 尾段动作」各自完整一遍，否则状态会先于动画结束，
 * 表现为庆祝只播一半就回到闲置（历史上 done 窗口 3500ms < celebrate 4795ms）。
 */
export function resultHoldMs(state: PetState): number {
  if (state === 'error') {
    // error（抖动）完整一遍，接 disappointed（垂头）完整一遍。
    return poseHoldMs('error') + poseHoldMs('disappointed');
  }
  if (state === 'done') {
    return poseHoldMs('celebrate', { tailMs: AUTO.celebrate.tailMs });
  }
  return 0;
}

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

/** 一段自动表演的占用。指针交互可以打断；下一段闲置或漂移必须等它结束。 */
export type PerformanceLease = { until: number };

export function canBeginAuto(lease: PerformanceLease | null, now: number): boolean {
  return !lease || now >= lease.until;
}

export function leaseFor(now: number, ms: number): PerformanceLease {
  return { until: now + Math.max(0, ms) };
}

/** 自动表演还占着时，下一次尝试要等到租约结束再留一点空隙。 */
export function autoWaitMs(lease: PerformanceLease | null, now: number, gapMs = 0): number {
  if (!lease || lease.until <= now) return Math.max(0, gapMs);
  return lease.until - now + Math.max(0, gapMs);
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

/** 从 manifest 允许的闲置差分姿里随机挑一个；空列表回退 stretch。 */
export function pickIdleVariant(
  { random = Math.random }: { random?: () => number } = {},
): PetPose {
  const list = AUTO.idleVariant.poses as readonly PetPose[];
  return list.length ? list[Math.floor(random() * list.length)] : 'stretch';
}

/**
 * 加权版闲置姿势选择：内部委托统一通知模型的 weightedIdleVariant，
 * 支持注入权重 / 最近姿势历史 / 随机源；全部参数缺省时等价旧的行为契约。
 * 原 pickIdleVariant 保持原签名与行为，供冒烟脚本继续依赖。
 * 作者：孙文龙 sun
 */
export function pickIdleVariantWeighted(
  poses: readonly string[] = AUTO.idleVariant.poses as readonly string[],
  weights: Record<string, number> = IDLE_VARIANT_WEIGHTS,
  history: readonly string[] = [],
  rand: () => number = Math.random,
): PetPose {
  return weightedIdleVariant(poses, weights, history, rand) as PetPose;
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

/** 网关态优先级表，与 manifest.statePriority 同序；新增状态只插一行。 */
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

// ---- 动作注册表（表驱动，扩展零改代码）----

/** flags 的 key 就是 manifest.poses[].trigger；true 表示该事件正在发生。 */
export type PetActionInput = {
  state: PetState;
  flags?: Readonly<Record<string, boolean>>;
};

export type PetActionRule = {
  id: string;
  pose: PetPose;
  priority: number;
  trigger: string;
  states: readonly string[] | null;
  group: string;
  when: (input: PetActionInput) => boolean;
};

/**
 * 由生成常量直接构建优先级表：PET_ACTIONS 已按 priority 降序排列。
 * `trigger: 'always'` 用于 state 类动作（states 约束生效），其余靠 flags 命中。
 * 每个动作有专属素材，不存在「拖拽复用走路」这类跨语义复用。
 */
export const PET_ACTION_TABLE: readonly PetActionRule[] = PET_ACTIONS.map((action) => ({
  id: action.name,
  pose: action.name,
  priority: action.priority,
  trigger: action.trigger,
  states: action.states,
  group: action.group,
  when: (input) =>
    (action.states === null || action.states.includes(input.state))
    && (action.trigger === 'always' || Boolean(input.flags?.[action.trigger])),
}));

/** 查表：首个命中即返回；idle 是 priority 0 的兜底行，永远有结果。 */
export function selectPetAction(input: PetActionInput): PetPose {
  const row = PET_ACTION_TABLE.find((rule) => rule.when(input));
  return row ? row.pose : 'idle';
}

/** 动作名 → 是否属于某语义分组；扩展动作后自动纳入，无需改调用方。 */
export function actionsInGroup(group: string): readonly PetPose[] {
  return PET_ACTION_TABLE.filter((rule) => rule.group === group)
    .map((rule) => rule.pose);
}

/** state 类动作（trigger === 'always'）由状态机独占，不能当事件 flag 置位，避免虚报状态。 */
const STATE_ACTION_FLAGS = new Set(
  PET_ACTION_TABLE.filter((rule) => rule.trigger === 'always').map((rule) => rule.id),
);

/**
 * 业务事件 → 可直接置位的 flag（auto 动作 trigger 等于动作名，可当 flag 用）。
 * 若事件动作属于 state 类（typing/error/done/sleep/offline）则返回 null，交回状态机。
 * 作者：孙文龙 sun
 */
export function eventActionFlag(event: PetEvent): string | null {
  const name = eventActionFor(event);
  return STATE_ACTION_FLAGS.has(name) ? null : name;
}

// ---- v1.2 兼容包装：旧字段映射到新 flags ----

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
  dragging?: boolean;
  /** v1.3 放下落地：松手后的短暂专属姿 */
  landing?: boolean;
  waking?: boolean;
  welcoming?: boolean;
  waiting?: boolean;
  disappointed?: boolean;
  /** 业务事件动作名（auto 类），由 petActionFlags 展开成对应 flag。作者：孙文龙 sun */
  eventActions?: string[];
};

/** 旧式布尔输入 → 注册表 flags；保留给渐进迁移与既有单测。 */
export function petActionFlags(input: PetPoseInput): Record<string, boolean> {
  const flags: Record<string, boolean> = {
    // v1.3.3：自主漫游改成「被提起来飞过去」，不再复用 walk 走路序列帧
    // （走路姿与窗口位移对齐困难，脚步与落点永远对不上）。
    // 只在 idle 生效：漫游本就只在 idle 启动，且被 typing/error 打断时
    // 不能用 lifted 盖掉真实请求态（红线：状态反馈不得虚报）。
    lifted: Boolean(input.dragging)
      || (Boolean(input.roaming) && input.state === 'idle'),
    landing: Boolean(input.landing),
    wake: Boolean(input.waking),
    welcome: Boolean(input.welcoming),
    greeting: Boolean(input.greeting),
    celebrate: Boolean(input.celebrating),
    disappointed: Boolean(input.disappointed),
    petted: Boolean(input.shy),
    poked: Boolean(input.poked),
    wait: Boolean(input.waiting),
    peek: Boolean(input.listening),
    // walk 只剩「鼠标靠近时的预备走」视线反馈；整屏漂移不再用它。
    walk: Boolean(input.walking),
  };
  // 事件动作名只在 auto 类范围内置位；state 类动作由状态机独占，绝不虚报。
  // 作者：孙文龙 sun
  for (const name of input.eventActions ?? []) {
    if (!STATE_ACTION_FLAGS.has(name)) flags[name] = true;
  }
  if (input.idlePose) flags[input.idlePose] = true;
  return flags;
}

/** 兼容包装：由注册表推导立绘实际播放哪一层。 */
export function resolvePetPose(input: PetPoseInput): PetPose {
  return selectPetAction({ state: input.state, flags: petActionFlags(input) });
}

/** 兼容包装：由表推导当前立绘态。 */
export function derivePetState(input: PetStateInput): PetState {
  return selectPetState(input);
}

/** 兼容旧引用名；语义上是同一张动作注册表。 */
export const PET_POSE_TABLE = PET_ACTION_TABLE;

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
 * 不读 Date.now、不设定时器，时间由调用方显式注入，便于确定性单测。
 */
export function reducePetState(input: PetReducerInput): PetReducerResult {
  const now = input.now ?? input.gateway.now;
  const gateway = { ...input.gateway, now };
  const state = selectPetState(gateway);
  const pose = resolvePetPose({ ...DEFAULT_POSE, ...input.pose, state });
  // 结果态窗口由素材时长派生：error 抖动一遍 + 失落一遍，done 庆祝一遍 + 尾部。
  // 历史上写死 6000 / 3500，比动作本身短，会导致状态先切、动画跟着被截断。
  const holdMs = resultHoldMs(state);
  return {
    state,
    pose,
    signature: `${state}|${pose}|${gateway.hold.state}:${gateway.hold.until}`,
    holdMs,
    bubble: state === 'done' ? 'ok' : state === 'error' ? 'error' : null,
  };
}

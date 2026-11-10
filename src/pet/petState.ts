// 桌面精灵（小酥）状态机纯函数：与 React / Tauri 解耦，便于单测与复用。
// v1.3：立绘动作改为「注册表驱动」——PET_ACTIONS 由 manifest.json 的 poses[]
// 生成，按 priority 降序查表；新增动作只在 manifest 追加一条并放两张 WebP，
// 本文件、PetStage.tsx、pet.css 一律零改动。
// 网关态 + 交互/自动动作分开，避免轻碰抢掉真实 typing；纯 reducer 不读 Date.now。
import {
  PET_ACTIONS,
  PET_AUTO_ACTIONS,
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

/** 从 manifest 允许的闲置差分姿里随机挑一个；空列表回退 stretch。 */
export function pickIdleVariant(
  { random = Math.random }: { random?: () => number } = {},
): PetPose {
  const list = AUTO.idleVariant.poses as readonly PetPose[];
  return list.length ? list[Math.floor(random() * list.length)] : 'stretch';
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
  const holdMs = state === 'error' ? 6000 : state === 'done' ? 3500 : 0;
  return {
    state,
    pose,
    signature: `${state}|${pose}|${gateway.hold.state}:${gateway.hold.until}`,
    holdMs,
    bubble: state === 'done' ? 'ok' : state === 'error' ? 'error' : null,
  };
}

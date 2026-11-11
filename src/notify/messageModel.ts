/**
 * 统一通知模型（1.2.0）
 * 作者：孙文龙 sun
 * 作用：把主窗口 Toast 与桌面精灵气泡收敛到同一套分类、定级、去重、生命周期与可见性规则。
 * 说明：本文件是纯函数集合，不依赖 React / DOM，便于 Node 冒烟脚本直接打包测试。
 */

export type NoticeCategory = 'state' | 'result' | 'idle' | 'interaction';
export type NoticeSeverity = 'ok' | 'error' | 'info' | 'warn';
export type NoticeSurface = 'toast' | 'bubble';
export type NoticeActionKind = 'navigate' | 'retry' | 'settings';

export interface NoticeAction {
  label: string;
  kind: NoticeActionKind;
  target?: string;
}

export interface Notice {
  id: number;
  text: string;
  category: NoticeCategory;
  severity: NoticeSeverity;
  surface: NoticeSurface;
  title?: string;
  source?: string;
  action?: NoticeAction;
  dedupeKey: string;
  count: number;
  ttlMs: number;
  createdAt: number;
  updatedAt: number;
}

export interface NoticeInput {
  text: string;
  category?: NoticeCategory;
  severity?: NoticeSeverity;
  surface?: NoticeSurface;
  title?: string;
  source?: string;
  action?: NoticeAction;
  dedupeKey?: string;
  ttlMs?: number;
}

/** 分类优先级：互动与结果优先于闲置，避免闲聊把错误消息挤出可视区。 */
export const CATEGORY_PRIORITY: Record<NoticeCategory, number> = {
  interaction: 40,
  result: 30,
  state: 20,
  idle: 10,
};

/** 分类基础停留时长（毫秒）：错误类会再叠加 SEVERITY_TTL_BONUS。 */
export const CATEGORY_TTL_MS: Record<NoticeCategory, number> = {
  state: 3200,
  result: 5200,
  idle: 2600,
  interaction: 3600,
};

export const SEVERITY_TTL_BONUS: Record<NoticeSeverity, number> = {
  error: 2600,
  warn: 1200,
  ok: 0,
  info: 0,
};

export const SEVERITY_PRIORITY: Record<NoticeSeverity, number> = {
  error: 40,
  warn: 20,
  ok: 10,
  info: 0,
};

export const DEFAULT_CAPACITY = 3;
/** 同一 dedupeKey 在该窗口内重复出现时合并计数，而不是新增一条。 */
export const DEDUPE_WINDOW_MS = 1500;
/** 真实在途请求持续超过该时长才允许发一条状态消息，避免短请求刷屏。 */
export const LONG_REQUEST_MS = 8000;
export const STATE_COOLDOWN_MS = 15000;

/** 从文案推断级别；调用方显式传入时以调用方为准。 */
export function guessSeverity(text: string): NoticeSeverity {
  if (/error|fail|invalid|timeout|失败|错误|不可用|超时/i.test(text)) return 'error';
  if (/warn|retry|警告|注意|重试/i.test(text)) return 'warn';
  return 'info';
}

export function noticeTtlMs(
  category: NoticeCategory,
  severity: NoticeSeverity,
  override?: number,
): number {
  if (typeof override === 'number' && override > 0) return override;
  return CATEGORY_TTL_MS[category] + SEVERITY_TTL_BONUS[severity];
}

export function dedupeKeyFor(input: NoticeInput): string {
  return input.dedupeKey || `${input.category || 'result'}|${input.severity || ''}|${input.text.trim()}`;
}

export function createNotice(
  id: number,
  input: NoticeInput | string,
  nowMs: number,
  kind?: NoticeSeverity,
  defaultSurface: NoticeSurface = 'toast',
): Notice {
  const normalized: NoticeInput = typeof input === 'string'
    ? { text: input, severity: kind }
    : input;
  const text = normalized.text.trim();
  const category = normalized.category || 'result';
  const severity = normalized.severity || kind || guessSeverity(text);
  return {
    id,
    text,
    category,
    severity,
    surface: normalized.surface || defaultSurface,
    title: normalized.title,
    source: normalized.source,
    action: normalized.action,
    dedupeKey: dedupeKeyFor(normalized),
    count: 1,
    ttlMs: noticeTtlMs(category, severity, normalized.ttlMs),
    createdAt: nowMs,
    updatedAt: nowMs,
  };
}

export interface MergeResult {
  list: Notice[];
  mergedId: number | null;
}

/** 合并同键消息：刷新停留时间并把 count 累加，返回合并到的消息 id。 */
export function mergeNotices(
  list: Notice[],
  incoming: Notice,
  nowMs: number,
  windowMs: number = DEDUPE_WINDOW_MS,
): MergeResult {
  const target = list.find(
    (item) => item.dedupeKey === incoming.dedupeKey
      && nowMs - item.updatedAt <= windowMs
      && nowMs - item.updatedAt < item.ttlMs,
  );
  if (!target) return { list: [...list, incoming], mergedId: null };
  return {
    list: list.map((item) => (item.id === target.id
      ? {
        ...item,
        count: item.count + 1,
        updatedAt: nowMs,
        ttlMs: incoming.ttlMs,
        text: incoming.text,
        severity: incoming.severity,
        action: incoming.action || item.action,
      }
      : item)),
    mergedId: target.id,
  };
}

/** 按停留时长清理过期消息；暂停期间由 shiftNoticeDeadlines 顺延时间基准。 */
export function pruneNotices(list: Notice[], nowMs: number): Notice[] {
  return list.filter((item) => nowMs - item.updatedAt < item.ttlMs);
}

/** 暂停恢复后把所有消息的更新时间顺延，避免一恢复就整批消失。 */
export function shiftNoticeDeadlines(list: Notice[], deltaMs: number): Notice[] {
  if (deltaMs <= 0) return list;
  return list.map((item) => ({
    ...item,
    createdAt: item.createdAt + deltaMs,
    updatedAt: item.updatedAt + deltaMs,
  }));
}

export interface VisibleResult {
  visible: Notice[];
  hidden: number;
}

/** 优先级 + 新鲜度排序后截取可视条数；被挤掉的数量用于「还有 N 条」提示。 */
export function selectVisible(list: Notice[], capacity: number = DEFAULT_CAPACITY): VisibleResult {
  const ranked = [...list].sort((a, b) => {
    const pa = CATEGORY_PRIORITY[a.category] + SEVERITY_PRIORITY[a.severity];
    const pb = CATEGORY_PRIORITY[b.category] + SEVERITY_PRIORITY[b.severity];
    if (pa !== pb) return pb - pa;
    return b.updatedAt - a.updatedAt;
  });
  const limit = Math.max(0, capacity);
  return { visible: ranked.slice(0, limit), hidden: Math.max(0, ranked.length - limit) };
}

export interface StateNoticeGate {
  activeRequests: number;
  requestSinceMs: number;
  nowMs: number;
  lastEmitAt: number;
  thresholdMs?: number;
  cooldownMs?: number;
}

/** 状态消息只由真实在途统计产生：在途归零、短请求、冷却期内一律不发，避免虚报状态。 */
export function canEmitStateNotice(gate: StateNoticeGate): boolean {
  if (gate.activeRequests <= 0) return false;
  if (gate.requestSinceMs < (gate.thresholdMs ?? LONG_REQUEST_MS)) return false;
  if (gate.lastEmitAt > 0
    && gate.nowMs - gate.lastEmitAt < (gate.cooldownMs ?? STATE_COOLDOWN_MS)) return false;
  return true;
}

/** 闲置姿势 → 台词键：让台词跟着当前动作走，而不是 3 条随机文案。 */
export const IDLE_MESSAGE_KEYS: Record<string, string> = {
  squatDraw: 'pet.bubble.idleSquatDraw',
  wiggleDance: 'pet.bubble.idleWiggleDance',
  kickflip: 'pet.bubble.idleKickflip',
  miniGame: 'pet.bubble.idleMiniGame',
  stretch: 'pet.bubble.idleStretch',
  lookAround: 'pet.bubble.idleLookAround',
  peek: 'pet.bubble.idlePeek',
  sleep: 'pet.bubble.idle1',
  idle: 'pet.bubble.idle2',
};

export function idleMessageKeyFor(pose: string): string {
  return IDLE_MESSAGE_KEYS[pose] || 'pet.bubble.idle1';
}

/** 交互意图 → 台词键：摸头、连点、落地、靠近各有自己的话。 */
export const INTERACTION_MESSAGE_KEYS: Record<string, string> = {
  headPat: 'pet.bubble.headPat',
  poke: 'pet.bubble.poke',
  tripleTap: 'pet.bubble.tripleTap',
  wake: 'pet.bubble.wake',
  near: 'pet.bubble.near',
  dragLand: 'pet.bubble.dragLand',
  hello: 'pet.bubble.hello',
};

export function interactionMessageKeyFor(kind: string): string {
  return INTERACTION_MESSAGE_KEYS[kind] || 'pet.bubble.poke';
}

export type PetEvent =
  | 'longRequest'
  | 'retry'
  | 'modelSwitch'
  | 'failure'
  | 'gatewayUp'
  | 'gatewayDown'
  | 'failover';

/**
 * 业务事件 → 复用已有姿势（全部是 auto 类动作，trigger 等于动作名，可直接置 flag）；
 * 刻意不映射到 typing / error / offline 这类 state 动作，避免和状态优先级打架。零素材、不新增序列帧。
 */
export const PET_EVENT_ACTIONS: Record<PetEvent, string> = {
  longRequest: 'wait',
  retry: 'lookAround',
  modelSwitch: 'peek',
  failure: 'disappointed',
  gatewayUp: 'greeting',
  gatewayDown: 'wait',
  failover: 'lookAround',
};

export function eventActionFor(event: PetEvent): string {
  return PET_EVENT_ACTIONS[event];
}

/** 闲置姿势权重：基础动作常见、新增动作也能被看见，kickflip 只有一版进包所以少播。 */
export const IDLE_VARIANT_WEIGHTS: Record<string, number> = {
  stretch: 3,
  lookAround: 3,
  peek: 2,
  squatDraw: 2,
  wiggleDance: 2,
  miniGame: 2,
  kickflip: 1,
  sleep: 1,
  idle: 1,
};

/**
 * 加权挑选闲置姿势：不与上一次重复，最近两次降权而非禁止。
 * rand 可注入以便测试；权重缺省时按 1 处理。
 */
export function weightedIdleVariant(
  poses: readonly string[],
  weights: Record<string, number> = IDLE_VARIANT_WEIGHTS,
  history: readonly string[] = [],
  rand: () => number = Math.random,
): string {
  if (poses.length === 0) return 'stretch';
  const last = history[0];
  let candidates = poses.filter((pose) => pose !== last);
  if (candidates.length === 0) candidates = [...poses];
  const recent = new Set(history.slice(0, 2));
  const scored = candidates.map((pose) => {
    const base = Math.max(0, weights[pose] ?? 1);
    return { pose, weight: recent.has(pose) ? base * 0.35 : base };
  });
  const total = scored.reduce((sum, item) => sum + item.weight, 0);
  if (total <= 0) return candidates[Math.floor(rand() * candidates.length) % candidates.length];
  let roll = rand() * total;
  for (const item of scored) {
    roll -= item.weight;
    if (roll <= 0) return item.pose;
  }
  return scored[scored.length - 1].pose;
}

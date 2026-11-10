// 由 docs/孙测测/scripts/gen_pet_from_manifest.py 从 manifest.json 生成，勿手工编辑。
// 单一真相源：姿势清单、素材文件名、逐帧时长、播放模式、motion 与 alpha 命中框。
export const PET_POSE_NAMES = [
  'lifted',
  'landing',
  'typing',
  'error',
  'done',
  'sleep',
  'offline',
  'celebrate',
  'disappointed',
  'petted',
  'poked',
  'wake',
  'welcome',
  'greeting',
  'wait',
  'stretch',
  'lookAround',
  'peek',
  'walk',
  'idle',
] as const;
export type PetPoseName = (typeof PET_POSE_NAMES)[number];

export const PET_IMAGE_SIZE = { w: 180, h: 297 } as const;

export const PET_HITBOXES: Record<PetPoseName, readonly [number, number, number, number]> = {
  lifted: [0.2056, 0.1886, 0.7944, 0.8182],
  landing: [0.1278, 0.1717, 0.8444, 0.798],
  typing: [0.15, 0.1987, 0.8444, 0.8013],
  error: [0.1889, 0.2054, 0.7944, 0.8013],
  done: [0.1833, 0.2054, 0.7833, 0.8013],
  sleep: [0.2, 0.1987, 0.8167, 0.798],
  offline: [0.2167, 0.2054, 0.7944, 0.8013],
  celebrate: [0.1056, 0.1919, 0.8944, 0.798],
  disappointed: [0.2111, 0.1987, 0.8, 0.8013],
  petted: [0.2056, 0.1852, 0.8111, 0.798],
  poked: [0.2056, 0.202, 0.7944, 0.798],
  wake: [0.0889, 0.1785, 0.9278, 0.798],
  welcome: [0.1611, 0.202, 0.8111, 0.798],
  greeting: [0.2056, 0.101, 0.8333, 0.8114],
  wait: [0.1611, 0.2054, 0.7778, 0.798],
  stretch: [0.1667, 0.1684, 0.8833, 0.8047],
  lookAround: [0.1667, 0.2054, 0.8222, 0.798],
  peek: [0.1556, 0.1987, 0.8389, 0.798],
  walk: [0.2056, 0.1953, 0.7778, 0.8081],
  idle: [0.2333, 0.2054, 0.7833, 0.8013],
};

export const PET_ASSET_FILES: Record<PetPoseName, { poster: string; loop: string }> = {
  lifted: { poster: 'su-lifted.webp', loop: 'su-lifted-loop.webp' },
  landing: { poster: 'su-landing.webp', loop: 'su-landing-loop.webp' },
  typing: { poster: 'su-typing.webp', loop: 'su-typing-loop.webp' },
  error: { poster: 'su-error.webp', loop: 'su-error-loop.webp' },
  done: { poster: 'su-done.webp', loop: 'su-done-loop.webp' },
  sleep: { poster: 'su-sleep.webp', loop: 'su-sleep-loop.webp' },
  offline: { poster: 'su-offline.webp', loop: 'su-offline-loop.webp' },
  celebrate: { poster: 'su-celebrate.webp', loop: 'su-celebrate-loop.webp' },
  disappointed: { poster: 'su-disappointed.webp', loop: 'su-disappointed-loop.webp' },
  petted: { poster: 'su-petted.webp', loop: 'su-petted-loop.webp' },
  poked: { poster: 'su-poked.webp', loop: 'su-poked-loop.webp' },
  wake: { poster: 'su-wake.webp', loop: 'su-wake-loop.webp' },
  welcome: { poster: 'su-welcome.webp', loop: 'su-welcome-loop.webp' },
  greeting: { poster: 'su-greeting.webp', loop: 'su-greeting-loop.webp' },
  wait: { poster: 'su-wait.webp', loop: 'su-wait-loop.webp' },
  stretch: { poster: 'su-stretch.webp', loop: 'su-stretch-loop.webp' },
  lookAround: { poster: 'su-lookAround.webp', loop: 'su-lookAround-loop.webp' },
  peek: { poster: 'su-peek.webp', loop: 'su-peek-loop.webp' },
  walk: { poster: 'su-walk.webp', loop: 'su-walk-loop.webp' },
  idle: { poster: 'su-idle.webp', loop: 'su-idle-loop.webp' },
};

// 逐帧停留时长（ms）：支持非匀速节奏，长度与动画 WebP 的 ANMF 帧一一对应。
export const PET_FRAME_MS: Record<PetPoseName, readonly number[]> = {
  lifted: [125, 125, 120, 120, 115, 115, 110, 105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130],
  landing: [115, 115, 115, 110, 110, 105, 105, 100, 95, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 60, 60, 60, 60, 60, 60, 60, 60, 60, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 95, 100, 105, 105, 110, 110, 115, 115, 115, 120, 120, 120],
  typing: [125, 120, 120, 115, 115, 110, 105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125],
  error: [105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125, 125, 120, 120, 115, 115, 110],
  done: [90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125, 125, 120, 120, 115, 115, 110, 105, 100, 100, 95],
  sleep: [105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125, 125, 120, 120, 115, 115, 110],
  offline: [115, 110, 105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125, 125, 120, 120, 115],
  celebrate: [130, 130, 130, 125, 125, 120, 120, 115, 115, 110, 105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130],
  disappointed: [110, 105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125, 125, 120, 120, 115, 115],
  petted: [130, 130, 130, 125, 125, 120, 120, 115, 115, 110, 105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130],
  poked: [125, 120, 120, 115, 115, 110, 105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125],
  wake: [75, 70, 70, 65, 65, 60, 60, 60, 60, 60, 60, 60, 60, 60, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 95, 100, 105, 105, 110, 110, 115, 115, 115, 120, 120, 120, 115, 115, 115, 110, 110, 105, 105, 100, 95, 95, 90, 85, 80, 80],
  welcome: [120, 120, 115, 115, 115, 110, 110, 105, 105, 100, 95, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 60, 60, 60, 60, 60, 60, 60, 60, 60, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 95, 100, 105, 105, 110, 110, 115, 115, 115, 120],
  greeting: [115, 115, 110, 110, 105, 105, 100, 95, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 60, 60, 60, 60, 60, 60, 60, 60, 60, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 95, 100, 105, 105, 110, 110, 115, 115, 115, 120, 120, 120, 115],
  wait: [100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125, 125, 120, 120, 115, 115, 110, 105, 100],
  stretch: [120, 120, 115, 115, 115, 110, 110, 105, 105, 100, 95, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 60, 60, 60, 60, 60, 60, 60, 60, 60, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 95, 100, 105, 105, 110, 110, 115, 115, 115, 120],
  lookAround: [120, 120, 115, 115, 115, 110, 110, 105, 105, 100, 95, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 60, 60, 60, 60, 60, 60, 60, 60, 60, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 95, 100, 105, 105, 110, 110, 115, 115, 115, 120],
  peek: [95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125, 125, 120, 120, 115, 115, 110, 105, 100, 100],
  walk: [120, 115, 115, 110, 105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125, 125, 120],
  idle: [120, 115, 115, 110, 105, 100, 100, 95, 90, 85, 80, 80, 75, 70, 70, 65, 65, 65, 65, 65, 65, 65, 65, 65, 70, 70, 75, 80, 80, 85, 90, 95, 100, 100, 105, 110, 115, 115, 120, 120, 125, 125, 130, 130, 130, 130, 130, 125, 125, 120],
};

export const PET_PLAYBACK: Record<PetPoseName, string> = {
  lifted: 'loop',
  landing: 'once',
  typing: 'loop',
  error: 'loop',
  done: 'loop',
  sleep: 'loop',
  offline: 'loop',
  celebrate: 'loop',
  disappointed: 'loop',
  petted: 'loop',
  poked: 'loop',
  wake: 'once',
  welcome: 'once',
  greeting: 'once',
  wait: 'loop',
  stretch: 'once',
  lookAround: 'once',
  peek: 'loop',
  walk: 'loop',
  idle: 'loop',
};

export const PET_MOTION: Record<PetPoseName, string> = {
  lifted: 'dangle',
  landing: 'settle',
  typing: 'bob',
  error: 'shake',
  done: 'hop',
  sleep: 'float',
  offline: 'breathe',
  celebrate: 'hop',
  disappointed: 'droop',
  petted: 'wiggle',
  poked: 'tilt',
  wake: 'stretch',
  welcome: 'wave',
  greeting: 'nod',
  wait: 'tilt',
  stretch: 'stretch',
  lookAround: 'turn',
  peek: 'tilt',
  walk: 'bob',
  idle: 'breathe',
};

// 动作注册表：前端按 priority 降序查表。新增动作只在 manifest.json 追加一条，
// 本文件由生成器重算，petState.ts / PetStage.tsx 无需改动。
export type PetAction = {
  name: PetPoseName;
  semantic: string;
  trigger: string;
  group: string;
  priority: number;
  states: readonly string[] | null;
  motion: string;
  playback: string;
  frames: number;
};

export const PET_ACTIONS: readonly PetAction[] = [
  { name: 'lifted', semantic: "鼠标按住换位置时小酥被提起来：双脚离地、手臂上提、头发与双腿自然下垂并轻晃", trigger: 'lifted', group: 'interaction', priority: 100, states: null, motion: 'dangle', playback: 'loop', frames: 50 },
  { name: 'landing', semantic: "松手放下：落地缓冲、膝盖微屈、头发回弹后站定", trigger: 'landing', group: 'interaction', priority: 99, states: null, motion: 'settle', playback: 'once', frames: 50 },
  { name: 'celebrate', semantic: "连点三下或请求完成时的跳起庆祝", trigger: 'celebrate', group: 'interaction', priority: 80, states: ["done", "idle"], motion: 'hop', playback: 'loop', frames: 50 },
  { name: 'disappointed', semantic: "失败尾段的低头失落，短暂且不惩罚", trigger: 'disappointed', group: 'auto', priority: 78, states: ["error"], motion: 'droop', playback: 'loop', frames: 50 },
  { name: 'petted', semantic: "点头部摸头，缩肩害羞", trigger: 'petted', group: 'interaction', priority: 76, states: ["idle", "offline"], motion: 'wiggle', playback: 'loop', frames: 50 },
  { name: 'poked', semantic: "点身体轻碰，抬手回应", trigger: 'poked', group: 'interaction', priority: 75, states: ["idle", "offline"], motion: 'tilt', playback: 'loop', frames: 50 },
  { name: 'wake', semantic: "睡醒，伸懒腰揉眼", trigger: 'wake', group: 'auto', priority: 70, states: ["idle", "offline"], motion: 'stretch', playback: 'once', frames: 50 },
  { name: 'welcome', semantic: "开场或久别回来，挥手欢迎", trigger: 'welcome', group: 'auto', priority: 69, states: ["idle", "offline"], motion: 'wave', playback: 'once', frames: 50 },
  { name: 'greeting', semantic: "光标靠近，点头打招呼", trigger: 'greeting', group: 'auto', priority: 68, states: ["idle"], motion: 'nod', playback: 'once', frames: 50 },
  { name: 'wait', semantic: "等待批准，左右张望", trigger: 'wait', group: 'auto', priority: 66, states: ["idle", "typing"], motion: 'tilt', playback: 'loop', frames: 50 },
  { name: 'stretch', semantic: "闲置差分：伸懒腰", trigger: 'stretch', group: 'idleVariant', priority: 50, states: ["idle"], motion: 'stretch', playback: 'once', frames: 50 },
  { name: 'lookAround', semantic: "闲置差分：转头张望", trigger: 'lookAround', group: 'idleVariant', priority: 49, states: ["idle"], motion: 'turn', playback: 'once', frames: 50 },
  { name: 'peek', semantic: "聆听时探头", trigger: 'peek', group: 'auto', priority: 48, states: ["idle"], motion: 'tilt', playback: 'loop', frames: 50 },
  { name: 'walk', semantic: "自主游走 / 走到光标附近", trigger: 'walk', group: 'auto', priority: 47, states: ["idle"], motion: 'bob', playback: 'loop', frames: 50 },
  { name: 'typing', semantic: "真实请求在途，低头敲键盘；请求级只播动作不吐气泡", trigger: 'always', group: 'state', priority: 30, states: ["typing"], motion: 'bob', playback: 'loop', frames: 50 },
  { name: 'error', semantic: "请求失败，短促一顿后垂眼", trigger: 'always', group: 'state', priority: 29, states: ["error"], motion: 'shake', playback: 'loop', frames: 50 },
  { name: 'done', semantic: "请求完成，抬头挺胸的结果态", trigger: 'always', group: 'state', priority: 28, states: ["done"], motion: 'hop', playback: 'loop', frames: 50 },
  { name: 'sleep', semantic: "久无操作，闭眼打盹", trigger: 'always', group: 'state', priority: 27, states: ["sleep"], motion: 'float', playback: 'loop', frames: 50 },
  { name: 'offline', semantic: "网关离线，垂手静立待机", trigger: 'always', group: 'state', priority: 26, states: ["offline"], motion: 'breathe', playback: 'loop', frames: 50 },
  { name: 'idle', semantic: "待机呼吸，动作注册表的兜底基态", trigger: 'always', group: 'state', priority: 0, states: ["idle", "offline", "sleep", "typing", "done", "error"], motion: 'breathe', playback: 'loop', frames: 50 },
] as const;

// 自动交互节奏：语义层常量，代码级固定，不进设置页（参考 whale-girl 零打扰原则）。
export const PET_AUTO_ACTIONS = {"blink": {"minMs": 3000, "maxMs": 9000}, "facing": {"minMs": 10000, "maxMs": 25000}, "idleVariant": {"minMs": 12000, "maxMs": 30000, "holdMs": 2200, "poses": ["stretch", "lookAround", "peek", "walk"]}, "wander": {"minMs": 18000, "maxMs": 40000}, "welcome": {"holdMs": 6000, "pose": "welcome"}, "celebrate": {"holdMs": 3500, "pose": "celebrate"}, "disappointed": {"holdMs": 3000, "pose": "disappointed"}, "wake": {"holdMs": 1600, "pose": "wake"}, "wait": {"pose": "wait"}, "drag": {"pose": "lifted", "releasePose": "landing", "holdMs": 420}, "zeroNegativeFeedback": true, "liveliness": {"cycles": {"breathMs": 3100, "secondaryMs": 5300, "weightMs": 7900}, "gaze": {"eyeLerp": 0.32, "headLerp": 0.13, "bodyLerp": 0.055}, "secondary": {"maxDeg": 1.5, "maxShiftPx": 1.2}, "note": "三层周期按 0.1s 为单位两两互质（31/53/79），合成周期约 3.6 小时，人眼读不出循环点；眼/头/身用递减 lerp 形成跟随延迟。纯 CSS 层，零素材。"}, "note": "节奏参数属语义层常量，代码级固定；动作清单与优先级由 actions 注册表驱动，新增动作只在 manifest 追加一条，前端零改动。"} as const;

export const PET_STATE_PRIORITY = ["offline", "typing", "error", "done", "sleep", "idle"] as const;

export const PET_POSE_PRIORITY = ["lifted", "landing", "celebrate", "disappointed", "petted", "poked", "wake", "welcome", "greeting", "wait", "stretch", "lookAround", "peek", "walk", "typing", "error", "done", "sleep", "offline", "idle"] as const;

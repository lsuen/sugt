// 由 docs/孙测测/scripts/gen_pet_from_manifest.py 从 manifest.json 生成，勿手工编辑。
// 单一真相源：姿势清单、素材文件名、逐帧时长、播放模式、motion 与 alpha 命中框。
export const PET_POSE_NAMES = [
  'idle',
  'typing',
  'done',
  'error',
  'sleep',
  'wave',
  'peek',
  'shy',
  'celebrate',
  'walk',
] as const;
export type PetPoseName = (typeof PET_POSE_NAMES)[number];

export const PET_IMAGE_SIZE = { w: 105, h: 297 } as const;

export const PET_HITBOXES: Record<PetPoseName, readonly [number, number, number, number]> = {
  idle: [0.0667, 0.0202, 0.9333, 0.9697],
  typing: [0.0667, 0.0236, 0.9333, 0.9697],
  done: [0.1333, 0.0471, 0.9333, 0.9697],
  error: [0.0667, 0.0269, 0.9429, 0.9731],
  sleep: [0.0667, 0.0236, 0.9333, 0.9663],
  wave: [0.0762, 0.0236, 0.9333, 0.9764],
  peek: [0.0762, 0.0236, 0.9333, 0.9764],
  shy: [0.0667, 0.0236, 0.9429, 0.9798],
  celebrate: [0.0667, 0.0236, 0.9333, 0.9798],
  walk: [0.0762, 0.0236, 0.9333, 0.9798],
};

export const PET_ASSET_FILES: Record<PetPoseName, { poster: string; loop: string }> = {
  idle: { poster: 'su-idle.webp', loop: 'su-idle-loop.webp' },
  typing: { poster: 'su-typing.webp', loop: 'su-typing-loop.webp' },
  done: { poster: 'su-done.webp', loop: 'su-done-loop.webp' },
  error: { poster: 'su-error.webp', loop: 'su-error-loop.webp' },
  sleep: { poster: 'su-sleep.webp', loop: 'su-sleep-loop.webp' },
  wave: { poster: 'su-wave.webp', loop: 'su-wave-loop.webp' },
  peek: { poster: 'su-peek.webp', loop: 'su-peek-loop.webp' },
  shy: { poster: 'su-shy.webp', loop: 'su-shy-loop.webp' },
  celebrate: { poster: 'su-celebrate.webp', loop: 'su-celebrate-loop.webp' },
  walk: { poster: 'su-walk.webp', loop: 'su-walk-loop.webp' },
};

// 逐帧停留时长（ms）：支持非匀速节奏，长度与动画 WebP 的 ANMF 帧一一对应。
export const PET_FRAME_MS: Record<PetPoseName, readonly number[]> = {
  idle: [150, 120, 100, 100, 120, 150, 120, 100, 100, 120],
  typing: [150, 120, 100, 100, 120, 150, 120, 100, 100, 120],
  done: [150, 120, 100, 100, 120, 150, 120, 100, 100, 120],
  error: [150, 120, 100, 100, 120, 150, 120, 100, 100, 120],
  sleep: [150, 120, 100, 100, 120, 150, 120, 100, 100, 120],
  wave: [150, 120, 100, 100, 120, 150, 120, 100, 100, 120],
  peek: [150, 120, 100, 100, 120, 150, 120, 100, 100, 120],
  shy: [150, 120, 100, 100, 120, 150, 120, 100, 100, 120],
  celebrate: [150, 120, 100, 100, 120, 150, 120, 100, 100, 120],
  walk: [150, 120, 100, 100, 120, 150, 120, 100, 100, 120],
};

export const PET_PLAYBACK: Record<PetPoseName, string> = {
  idle: 'loop',
  typing: 'loop',
  done: 'loop',
  error: 'loop',
  sleep: 'loop',
  wave: 'loop',
  peek: 'loop',
  shy: 'loop',
  celebrate: 'loop',
  walk: 'loop',
};

export const PET_MOTION: Record<PetPoseName, string> = {
  idle: 'breathe',
  typing: 'bob',
  done: 'hop',
  error: 'shake',
  sleep: 'float',
  wave: 'wave',
  peek: 'tilt',
  shy: 'wiggle',
  celebrate: 'hop',
  walk: 'bob',
};

// 自动交互节奏：语义层常量，代码级固定，不进设置页（参考 whale-girl 零打扰原则）。
export const PET_AUTO_ACTIONS = {"blink": {"minMs": 3000, "maxMs": 9000}, "facing": {"minMs": 10000, "maxMs": 25000}, "idleVariant": {"minMs": 12000, "maxMs": 30000, "holdMs": 2200, "poses": ["peek", "wave", "shy", "walk"]}, "wander": {"minMs": 18000, "maxMs": 40000}, "welcome": {"holdMs": 6000, "pose": "wave"}, "celebrate": {"holdMs": 3500, "pose": "celebrate"}, "disappointed": {"holdMs": 3000, "pose": "shy"}, "wake": {"holdMs": 1600, "pose": "peek"}, "wait": {"pose": "peek"}, "drag": {"pose": "walk"}, "zeroNegativeFeedback": true, "liveliness": {"cycles": {"breathMs": 3100, "secondaryMs": 5300, "weightMs": 7900}, "gaze": {"eyeLerp": 0.32, "headLerp": 0.13, "bodyLerp": 0.055}, "secondary": {"maxDeg": 1.5, "maxShiftPx": 1.2}, "note": "三层周期按 0.1s 为单位两两互质（31/53/79），合成周期约 3.6 小时，人眼读不出循环点；眼/头/身用递减 lerp 形成跟随延迟。纯 CSS 层，零素材。"}, "note": "参考 whale-girl 的自动交互节奏；间隔与时长属语义层常量，代码级固定，不进设置页。"} as const;

export const PET_STATE_PRIORITY = ["offline", "typing", "error", "done", "sleep", "idle"] as const;

export const PET_POSE_PRIORITY = ["drag", "wake", "welcome", "celebrate", "disappointed", "shy", "wave", "greeting", "wait", "idleVariant", "roaming", "peek", "walk", "base"] as const;

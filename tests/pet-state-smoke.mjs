#!/usr/bin/env node
/**
 * 桌面精灵状态机冒烟：真实执行 derivePetState / resolvePetPose。
 * 运行：npm run test:pet-state
 */
import { build } from 'esbuild';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = process.cwd();
const workDir = mkdtempSync(join(tmpdir(), 'sugt-pet-state-'));
const bundlePath = join(workDir, 'pet-state.mjs');
const configPath = join(workDir, 'pet-config.mjs');

const bundleOptions = {
  bundle: true,
  platform: 'node',
  format: 'esm',
  logLevel: 'silent',
};
await build({
  ...bundleOptions,
  entryPoints: [join(root, 'src/pet/petState.ts')],
  outfile: bundlePath,
});
await build({
  ...bundleOptions,
  entryPoints: [join(root, 'src/pet/config.ts')],
  outfile: configPath,
});
const {
  derivePetState,
  resolvePetPose,
  reducePetState,
  PET_STATE_TABLE,
  PET_POSE_TABLE,
  petSleepAfterMs,
  petBubbleMs,
  AUTO,
  rhythmDelay,
  nextBlinkAt,
  nextFacingAt,
  nextIdleVariantAt,
  nextWanderAt,
  pickIdleVariant,
  shouldWake,
  wakeFromInteraction,
  facingFromDrag,
} = await import(pathToFileURL(bundlePath).href);
const { figureHitRect, pointInsideFigure } = await import(pathToFileURL(configPath).href);

let failed = 0;
function check(name, condition, detail) {
  if (condition) {
    console.log(`[ok] ${name}`);
  } else {
    console.error(`[fail] ${name}${detail ? ` — ${detail}` : ''}`);
    failed += 1;
  }
}

const now = 1_000_000;
check(
  'offline when gateway down',
  derivePetState({
    running: false, active: 0, hold: { state: 'idle', until: 0 },
    typingUntil: 0, lastActivity: now, sleepAfterMs: 300_000, now,
  }) === 'offline',
);
check(
  'typing wins over hold',
  derivePetState({
    running: true, active: 1, hold: { state: 'done', until: now + 9999 },
    typingUntil: 0, lastActivity: now, sleepAfterMs: 300_000, now,
  }) === 'typing',
);
check(
  'error hold after idle',
  derivePetState({
    running: true, active: 0, hold: { state: 'error', until: now + 1000 },
    typingUntil: 0, lastActivity: now, sleepAfterMs: 300_000, now,
  }) === 'error',
);
check(
  'sleep after inactivity',
  derivePetState({
    running: true, active: 0, hold: { state: 'idle', until: 0 },
    typingUntil: 0, lastActivity: now - 400_000, sleepAfterMs: 300_000, now,
  }) === 'sleep',
);
check(
  'typing not interrupted by poke',
  resolvePetPose({
    state: 'typing', poked: true, shy: true, listening: true,
    walking: true, celebrating: true, greeting: true, idlePose: 'wave', roaming: true,
  }) === 'typing',
);
check(
  'idle poke → wave',
  resolvePetPose({
    state: 'idle', poked: true, shy: false, listening: false,
    walking: false, celebrating: false,
  }) === 'wave',
);
check(
  'idle shy beats poke',
  resolvePetPose({
    state: 'idle', poked: true, shy: true, listening: false,
    walking: false, celebrating: false,
  }) === 'shy',
);
check(
  'done celebrate',
  resolvePetPose({
    state: 'done', poked: false, shy: false, listening: false,
    walking: false, celebrating: true,
  }) === 'celebrate',
);
check(
  'idle roam → walk',
  resolvePetPose({
    state: 'idle', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, roaming: true,
  }) === 'walk',
);
check('sleep default 300s', petSleepAfterMs(null) === 300_000);
check('bubble default 4s', petBubbleMs(undefined) === 4000);

// 声明式表 + 纯 reducer：注入 now，全部可确定性复跑
const gateway = (patch = {}) => ({
  running: true, active: 0, hold: { state: 'idle', until: 0 },
  typingUntil: 0, lastActivity: now, sleepAfterMs: 300_000, now,
  ...patch,
});
check('状态表行序即优先级',
  PET_STATE_TABLE.map((rule) => rule.id).join(',')
  === 'offline,typing,error,done,sleep,idle');
check('姿势表行序即优先级',
  PET_POSE_TABLE.map((rule) => rule.id).join(',')
  === 'drag,wake,welcome,celebrate,disappointed,shy,wave,greeting,wait,idleVariant,roaming,peek,walk');
check('reducer 与旧版 derivePetState 等价',
  ['offline', 'typing', 'error', 'done', 'sleep', 'idle'].every((_, i) => {
    const samples = [
      gateway({ running: false }),
      gateway({ active: 1 }),
      gateway({ hold: { state: 'error', until: now + 1000 } }),
      gateway({ hold: { state: 'done', until: now + 1000 } }),
      gateway({ lastActivity: now - 400_000 }),
      gateway(),
    ];
    return reducePetState({ gateway: samples[i], now }).state
      === derivePetState(samples[i]);
  }));
check('typing 不被交互差分姿抢',
  reducePetState({
    gateway: gateway({ active: 1 }),
    pose: {
      poked: true, shy: true, listening: true, walking: true,
      celebrating: true, greeting: true, idlePose: 'wave', roaming: true,
    },
    now,
  }).pose === 'typing');
check('重复同类事件签名一致（不重播动作起点）',
  reducePetState({ gateway: gateway({ active: 1 }), now }).signature
  === reducePetState({ gateway: gateway({ active: 1 }), now }).signature);
check('done hold 过期后重算底层 idle',
  reducePetState({
    gateway: gateway({ hold: { state: 'done', until: now - 1 } }), now,
  }).state === 'idle');
check('失败必切 error 且带 error 气泡与 6000ms 保持',
  (() => {
    const result = reducePetState({
      gateway: gateway({ hold: { state: 'error', until: now + 1000 } }), now,
    });
    return result.state === 'error' && result.bubble === 'error'
      && result.holdMs === 6000;
  })());
check('burst 结束重算底层：真实请求在走则回 typing',
  (() => {
    const result = reducePetState({
      gateway: gateway({ active: 1, hold: { state: 'error', until: now - 1 } }),
      now,
    });
    return result.state === 'typing' && result.bubble === null;
  })());

// 离线 alpha bbox 命中区：透明边距不再吞点击
const rect = { left: 0, top: 0, right: 106.6, bottom: 168 };
const hit = figureHitRect(rect, [0.3125, 0.1111, 0.6917, 0.8944],
  { w: 240, h: 360 }, 14);
check('命中区显著小于整块角色层',
  (hit.right - hit.left) < (rect.right - rect.left) * 0.8
  && (hit.bottom - hit.top) < (rect.bottom - rect.top) * 0.95,
  `${(hit.right - hit.left).toFixed(1)}×${(hit.bottom - hit.top).toFixed(1)}`);
check('角色中心仍可点', pointInsideFigure(53, 88, hit));
check('透明左上角保持穿透', !pointInsideFigure(2, 2, hit));

// v1.1 自动交互节奏器：now / random 注入，全部可确定性复跑
const fixed = () => 0.5;
check('节奏间隔由 manifest 与随机源确定性推导',
  rhythmDelay('blink', fixed) === (AUTO.blink.minMs + AUTO.blink.maxMs) / 2
  && nextBlinkAt({ now: 1000, random: fixed })
    === 1000 + (AUTO.blink.minMs + AUTO.blink.maxMs) / 2
  && nextFacingAt({ now: 0, random: fixed }) === (AUTO.facing.minMs + AUTO.facing.maxMs) / 2
  && nextIdleVariantAt({ now: 0, random: fixed })
    === (AUTO.idleVariant.minMs + AUTO.idleVariant.maxMs) / 2
  && nextWanderAt({ now: 0, random: fixed }) === (AUTO.wander.minMs + AUTO.wander.maxMs) / 2);
check('闲置差分姿只从 manifest 姿势池取',
  AUTO.idleVariant.poses.includes(pickIdleVariant({ random: () => 0 }))
  && AUTO.idleVariant.poses.includes(pickIdleVariant({ random: () => 0.999 })));
check('睡醒只在 sleep→非 sleep 边沿触发，拖拽打断不播',
  shouldWake('sleep', 'idle') && !shouldWake('idle', 'sleep')
  && !shouldWake('sleep', 'idle', { dragging: true })
  && !shouldWake('sleep', 'sleep'));
check('交互醒觉：视觉睡着才播 wake，且总是重置空闲',
  wakeFromInteraction({ visuallySleeping: true }).wake
  && !wakeFromInteraction({ visuallySleeping: false }).wake
  && !wakeFromInteraction({ visuallySleeping: true }).sleeping);
check('拖拽朝向：纯纵向保留、横向按位移翻转',
  facingFromDrag(-10, 'right') === 'left'
  && facingFromDrag(10, 'left') === 'right'
  && facingFromDrag(0, 'left') === 'left');
check('自动动作优先级：拖拽 > 欢迎 > 庆祝 > 闲置差分 > 基态',
  (() => {
    const base = {
      poked: false, shy: false, listening: false, walking: false, celebrating: false,
    };
    return resolvePetPose({
      state: 'idle', ...base, idlePose: 'peek', welcoming: true, dragging: true,
    }) === 'walk'
      && resolvePetPose({ state: 'idle', ...base, idlePose: 'peek', welcoming: true }) === 'wave'
      && resolvePetPose({ state: 'done', ...base, celebrating: true, dragging: true }) === 'walk'
      && resolvePetPose({ state: 'idle', ...base, idlePose: 'peek' }) === 'peek'
      && resolvePetPose({ state: 'idle', ...base }) === 'idle';
  })());
check('失败失落尾段只在 error 生效，不打断 typing',
  resolvePetPose({
    state: 'error', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, disappointed: true,
  }) === 'shy'
  && resolvePetPose({
    state: 'typing', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, disappointed: true,
  }) === 'typing');
check('等待批准动作只在 idle/typing 生效且不抢真实结果态',
  resolvePetPose({
    state: 'idle', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, waiting: true,
  }) === 'peek'
  && resolvePetPose({
    state: 'done', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, waiting: true,
  }) === 'done');

// 裁剪后画布：命中区贴合人物，不再有 38% 宽的透明点击墙
const petManifest = JSON.parse(
  readFileSync(join(root, 'src/assets/pet/manifest.json'), 'utf8'),
);
const tightHit = figureHitRect(
  { left: 0, top: 0, right: 120, bottom: 136 },
  petManifest.poses[0].hitBox,
);
check('裁剪后默认画布 105×297 且命中区贴合人物',
  petManifest.baseSize.w === 105 && petManifest.baseSize.h === 297
  && (petManifest.poses[0].hitBox[2] - petManifest.poses[0].hitBox[0]) > 0.7
  && (tightHit.right - tightHit.left) > 0
  && (tightHit.bottom - tightHit.top) > 0);

rmSync(workDir, { recursive: true, force: true });
if (failed) {
  console.error(`pet-state smoke: ${failed} failed`);
  process.exit(1);
}
console.log('pet-state smoke: all passed');

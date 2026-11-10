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
  PET_ACTIONS,
  selectPetAction,
  actionsInGroup,
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
  'idle poke → poked（专属抬手回应，语义不复用）',
  resolvePetPose({
    state: 'idle', poked: true, shy: false, listening: false,
    walking: false, celebrating: false,
  }) === 'poked',
);
check(
  '摸头胜过轻碰（76 > 75）且用专属 petted',
  resolvePetPose({
    state: 'idle', poked: true, shy: true, listening: false,
    walking: false, celebrating: false,
  }) === 'petted',
);
check(
  'done celebrate',
  resolvePetPose({
    state: 'done', poked: false, shy: false, listening: false,
    walking: false, celebrating: true,
  }) === 'celebrate',
);
check(
  'v1.3.3 idle 自主漂移 → lifted（被提起来飞过去，不再复用走路帧）',
  resolvePetPose({
    state: 'idle', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, roaming: true,
  }) === 'lifted'
  && resolvePetPose({
    state: 'typing', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, roaming: true,
  }) === 'typing',
);
check(
  'v1.3.3 漂移落地 → landing，且被提起压过落地',
  resolvePetPose({
    state: 'idle', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, landing: true,
  }) === 'landing'
  && resolvePetPose({
    state: 'idle', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, roaming: true, landing: true,
  }) === 'lifted',
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
check('动作注册表行序即优先级（由 manifest 生成，降序且 idle 兜底在末位）',
  PET_POSE_TABLE.map((rule) => rule.id).join(',')
    === PET_ACTIONS.map((action) => action.name).join(',')
  && PET_POSE_TABLE.length === 20
  && PET_POSE_TABLE[PET_POSE_TABLE.length - 1].id === 'idle'
  && PET_POSE_TABLE.every((rule, i) => i === 0
    || PET_POSE_TABLE[i - 1].priority >= rule.priority));
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
check('自动动作优先级：被提起 > 欢迎 > 闲置差分 > 基态',
  (() => {
    const base = {
      poked: false, shy: false, listening: false, walking: false, celebrating: false,
    };
    return resolvePetPose({
      state: 'idle', ...base, idlePose: 'peek', welcoming: true, dragging: true,
    }) === 'lifted'
      && resolvePetPose({ state: 'idle', ...base, idlePose: 'peek', welcoming: true }) === 'welcome'
      && resolvePetPose({ state: 'done', ...base, celebrating: true, dragging: true }) === 'lifted'
      && resolvePetPose({ state: 'idle', ...base, idlePose: 'peek' }) === 'peek'
      && resolvePetPose({ state: 'idle', ...base }) === 'idle';
  })());
check('失败失落尾段只在 error 生效，不打断 typing',
  resolvePetPose({
    state: 'error', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, disappointed: true,
  }) === 'disappointed'
  && resolvePetPose({
    state: 'typing', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, disappointed: true,
  }) === 'typing');
check('等待批准动作只在 idle/typing 生效且不抢真实结果态',
  resolvePetPose({
    state: 'idle', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, waiting: true,
  }) === 'wait'
  && resolvePetPose({
    state: 'done', poked: false, shy: false, listening: false,
    walking: false, celebrating: false, waiting: true,
  }) === 'done');

// v1.3 拖拽三段：按住被提起来、松手落地缓冲、随后回基态，三段各有专属序列帧。
check('拖拽三段 lifted → landing → idle，且被提起压过真实请求态',
  (() => {
    const base = {
      poked: false, shy: false, listening: false, walking: false, celebrating: false,
    };
    return resolvePetPose({ state: 'typing', ...base, dragging: true }) === 'lifted'
      && resolvePetPose({ state: 'idle', ...base, landing: true }) === 'landing'
      && resolvePetPose({ state: 'idle', ...base, dragging: true, landing: true }) === 'lifted'
      && resolvePetPose({ state: 'idle', ...base }) === 'idle';
  })());
check('动作注册表可扩展：未知 flag 不破坏查表且永远有兜底',
  selectPetAction({ state: 'typing', flags: { futureAction: true } }) === 'typing'
  && selectPetAction({ state: 'idle', flags: { futureAction: true } }) === 'idle'
  && actionsInGroup('interaction').includes('lifted')
  && actionsInGroup('interaction').includes('landing')
  && actionsInGroup('state').includes('idle'));
check('每段动作有专属语义、专属素材与唯一帧序（无跨语义复用、无重复帧）',
  new Set(PET_ACTIONS.map((action) => action.semantic)).size === PET_ACTIONS.length
  && PET_ACTIONS.every((action) => action.playback === 'loop'
    || action.playback === 'once'));

// 裁剪后画布：命中区贴合人物，不再有 38% 宽的透明点击墙
const petManifest = JSON.parse(
  readFileSync(join(root, 'src/assets/pet/manifest.json'), 'utf8'),
);
const tightHit = figureHitRect(
  { left: 0, top: 0, right: 120, bottom: 136 },
  petManifest.poses[0].hitBox,
);
// v1.3.2：人物改为「纯缩放、不裁肢体」后画布内留白更多，命中框宽度会比
// v1.3 的 0.7 更窄；再用写死的 >0.7 会误判。判据改为「四边内缩、面积不退化」，
// 与「命中区贴合人物、不留整幅透明点击墙」的原意一致。逐条与素材 50 帧
// alpha 并集的精确比对交给 SCR-020（Python + Pillow），此处只做量级断言。
const [hitX0, hitY0, hitX1, hitY1] = petManifest.poses[0].hitBox;
check('v1.3.2 默认画布 180×297 且命中区四边内缩、贴合人物（不退化）',
  petManifest.baseSize.w === 180 && petManifest.baseSize.h === 297
  && hitX0 > 0.05 && hitY0 > 0.05 && hitX1 < 0.95 && hitY1 < 0.95
  && (hitX1 - hitX0) > 0.45 && (hitY1 - hitY0) > 0.45
  && (tightHit.right - tightHit.left) > 0
  && (tightHit.bottom - tightHit.top) > 0);

// v1.3 帧预算与扩展契约：20 动作 × 50 帧 = 1000 张独立位图。
const actions = petManifest.poses;
check('动作注册表 20 项 / 精确 1000 帧 / 语义与素材均唯一',
  actions.length === 20
  && actions.reduce((sum, action) => sum + action.frames, 0) === 1000
  && new Set(actions.map((action) => action.semantic)).size === 20
  && new Set(actions.map((action) => action.loop)).size === 20
  && new Set(actions.map((action) => action.poster)).size === 20
  && actions.every((action) => Array.isArray(action.frameOrder)
    && action.frameOrder.length === action.frames
    && new Set(action.frameOrder).size === action.frames),
  `${actions.length} 动作 / ${actions.reduce((s, a) => s + a.frames, 0)} 帧`);
check('动作注册表声明了 trigger / priority / group / motion / playback 扩展字段',
  actions.every((action) => typeof action.trigger === 'string'
    && Number.isFinite(action.priority)
    && typeof action.group === 'string'
    && typeof action.motion === 'string'
    && ['loop', 'once'].includes(action.playback))
  && petManifest.actionRegistry.frameBudget.total === 1000
  && petManifest.actionRegistry.frameBudget.uniqueBitmapFrames === true
  && petManifest.actionRegistry.semanticUniqueness === true);

rmSync(workDir, { recursive: true, force: true });
if (failed) {
  console.error(`pet-state smoke: ${failed} failed`);
  process.exit(1);
}
console.log('pet-state smoke: all passed');

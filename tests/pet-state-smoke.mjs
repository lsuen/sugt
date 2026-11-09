#!/usr/bin/env node
/**
 * 桌面精灵状态机冒烟：真实执行 derivePetState / resolvePetPose。
 * 运行：npm run test:pet-state
 */
import { build } from 'esbuild';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = process.cwd();
const workDir = mkdtempSync(join(tmpdir(), 'sugt-pet-state-'));
const bundlePath = join(workDir, 'pet-state.mjs');

await build({
  entryPoints: [join(root, 'src/pet/petState.ts')],
  bundle: true,
  platform: 'node',
  format: 'esm',
  outfile: bundlePath,
  logLevel: 'silent',
});
const {
  derivePetState,
  resolvePetPose,
  petSleepAfterMs,
  petBubbleMs,
} = await import(pathToFileURL(bundlePath).href);

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

rmSync(workDir, { recursive: true, force: true });
if (failed) {
  console.error(`pet-state smoke: ${failed} failed`);
  process.exit(1);
}
console.log('pet-state smoke: all passed');

#!/usr/bin/env node
/**
 * 通知队列 Hook 冒烟：用极简 hook 运行时真实执行 useNoticeQueue，覆盖暂停/恢复/卸载路径。
 * 运行：npm run test:notice-queue
 * 编写人员：孙测测；作者：孙文龙 sun
 * 作用：把 QD-01「恢复暂停把新通知顺延到未来」与 QD-02「关闭最后一条后暂停态泄漏」
 *       两条缺陷做成可判定的回归用例，避免只靠代码走查放行。
 * 退出码：0 全部通过 / 1 有断言失败 / 2 打包或读取失败。
 */
import { build } from 'esbuild';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = process.cwd();
const workDir = mkdtempSync(join(tmpdir(), 'sugt-notice-queue-'));
const stubPath = join(workDir, 'react-stub.mjs');
const bundlePath = join(workDir, 'use-notice-queue.mjs');

writeFileSync(stubPath, [
  'export const useState = (...args) => globalThis.__miniReact.useState(...args);',
  'export const useRef = (...args) => globalThis.__miniReact.useRef(...args);',
  'export const useEffect = (...args) => globalThis.__miniReact.useEffect(...args);',
  'export const useCallback = (...args) => globalThis.__miniReact.useCallback(...args);',
  'export const useMemo = (...args) => globalThis.__miniReact.useMemo(...args);',
].join('\n'), 'utf8');

let useNoticeQueue;
try {
  await build({
    bundle: true,
    platform: 'node',
    format: 'esm',
    logLevel: 'silent',
    entryPoints: [join(root, 'src/notify/useNoticeQueue.ts')],
    outfile: bundlePath,
    plugins: [{
      name: 'mini-react-stub',
      setup(builder) {
        builder.onResolve({ filter: /^react$/ }, () => ({ path: stubPath, namespace: 'file' }));
      },
    }],
  });
  ({ useNoticeQueue } = await import(pathToFileURL(bundlePath).href));
} catch (error) {
  console.error(`[error] 打包通知队列 Hook 失败：${error?.message || error}`);
  rmSync(workDir, { recursive: true, force: true });
  process.exit(2);
}

let failed = 0;
function check(name, condition, detail) {
  if (condition) {
    console.log(`[ok] ${name}`);
  } else {
    console.error(`[fail] ${name}${detail ? ` — ${detail}` : ''}`);
    failed += 1;
  }
}

/**
 * 极简 hook 运行时：只实现本 Hook 用到的 5 个 API，setState 同步重渲染。
 * effect 按依赖数组变化执行并先跑 cleanup；window.setInterval 被捕获，由用例手动推进。
 */
function createHarness(hookFactory) {
  let hooks = [];
  let cursor = 0;
  let pendingEffects = [];
  let result;
  let tick = null;

  function slot() {
    if (cursor >= hooks.length) hooks.push({});
    const found = hooks[cursor];
    cursor += 1;
    return found;
  }

  function render() {
    cursor = 0;
    const effects = pendingEffects;
    pendingEffects = [];
    result = hookFactory();
    for (const item of effects) {
      if (typeof item.slot.cleanup === 'function') item.slot.cleanup();
      const cleanup = item.fn();
      item.slot.cleanup = typeof cleanup === 'function' ? cleanup : undefined;
    }
  }

  const runtime = {
    useState(initial) {
      const item = slot();
      if (!item.init) {
        item.init = true;
        item.value = typeof initial === 'function' ? initial() : initial;
      }
      return [item.value, (next) => {
        const value = typeof next === 'function' ? next(item.value) : next;
        if (Object.is(value, item.value)) return;
        item.value = value;
        render();
      }];
    },
    useRef(initial) {
      const item = slot();
      if (!item.init) {
        item.init = true;
        item.value = { current: initial };
      }
      return item.value;
    },
    useEffect(fn, deps) {
      const item = slot();
      const changed = !item.init || !deps || !item.deps
        || deps.length !== item.deps.length
        || deps.some((value, index) => !Object.is(value, item.deps[index]));
      item.init = true;
      item.deps = deps;
      if (changed) pendingEffects.push({ slot: item, fn });
    },
    useCallback(fn) { return fn; },
    useMemo(fn) { return fn(); },
  };

  globalThis.__miniReact = runtime;
  globalThis.window = {
    setInterval: (fn) => { tick = fn; return 1; },
    clearInterval: () => { tick = null; },
  };
  render();

  return {
    get value() { return result; },
    prune() { if (tick) tick(); },
  };
}

const NOW = 1_000_000;

// 一、QD-01：恢复暂停时只顺延暂停前已存在的消息
{
  let clock = NOW;
  const harness = createHarness(() => useNoticeQueue({ now: () => clock }));
  const oldId = harness.value.push('旧通知');
  harness.value.setPaused(true);
  clock = NOW + 4000;
  harness.value.push('新通知');
  clock = NOW + 10000;
  harness.value.setPaused(false);
  const list = harness.value.notices;
  check('暂停恢复后旧通知时间基准顺延', (() => {
    const old = list.find((item) => item.id === oldId);
    return old && old.updatedAt === NOW + 10000;
  })(), JSON.stringify(list.map((item) => ({ id: item.id, updatedAt: item.updatedAt }))));
  check('暂停中新建的通知不顺延、不落到未来时间', (() => {
    const fresh = list.find((item) => item.text === '新通知');
    return fresh && fresh.updatedAt === NOW + 4000 && fresh.updatedAt <= clock;
  })());
  let laterId = -1;
  check('恢复后同文案不会异常合并进未顺延的新通知', (() => {
    laterId = harness.value.push('新通知');
    const sameText = harness.value.notices.filter((item) => item.text === '新通知');
    return sameText.length === 2 && sameText.every((item) => item.count === 1);
  })());
  check('恢复后暂停中新建的通知按原 TTL 过期', (() => {
    // 先移走上一条断言新入队的消息，避免它的存在干扰「原通知是否过期」的判定。
    harness.value.dismiss(laterId);
    harness.prune();
    return harness.value.notices.filter((item) => item.text === '新通知').length === 0;
  })());
  check('暂停过的旧通知恢复后仍按剩余 TTL 保留', (() => {
    return harness.value.notices.some((item) => item.id === oldId);
  })());
}

// 二、QD-02：关闭/清空最后一条后暂停态必须复位
{
  let clock = NOW;
  const harness = createHarness(() => useNoticeQueue({ now: () => clock }));
  const id = harness.value.push('唯一一条');
  harness.value.setPaused(true);
  check('悬停后确实进入暂停', harness.value.paused === true);
  harness.value.dismiss(id);
  check('关闭最后一条后退出暂停（宿主卸载无 mouseleave）', harness.value.paused === false);
  clock = NOW + 1000;
  harness.value.push('之后的通知');
  clock = NOW + 7000;
  harness.prune();
  check('暂停态不泄漏：后续通知仍会过期', harness.value.notices.length === 0);
}

// 三、clear 后同一批次再入队，以及空队列误触暂停
{
  let clock = NOW;
  const harness = createHarness(() => useNoticeQueue({ now: () => clock }));
  harness.value.push('甲');
  harness.value.setPaused(true);
  harness.value.clear();
  check('clear 复位暂停态', harness.value.paused === false);
  clock = NOW + 1000;
  harness.value.push('乙');
  clock = NOW + 7000;
  harness.prune();
  check('清空后立即入队的新通知不会被冻结', harness.value.notices.length === 0);

  const empty = createHarness(() => useNoticeQueue({ now: () => clock }));
  empty.value.setPaused(true);
  check('空队列不进入暂停', empty.value.paused === false);
  clock = NOW + 8000;
  empty.value.push('丙');
  clock = NOW + 14000;
  empty.prune();
  check('空队列误触暂停后通知仍按 TTL 过期', empty.value.notices.length === 0);
}

delete globalThis.__miniReact;
delete globalThis.window;
rmSync(workDir, { recursive: true, force: true });
if (failed) {
  console.error(`notice-queue smoke: ${failed} failed`);
  process.exit(1);
}
console.log('notice-queue smoke: all passed');

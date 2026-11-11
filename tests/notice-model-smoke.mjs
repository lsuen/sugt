#!/usr/bin/env node
/**
 * 统一通知模型冒烟：真实执行 messageModel 纯函数与 petState 新增导出。
 * 运行：npm run test:notice-model
 * 编写人员：孙测测；作者：孙文龙 sun
 * 作用：把「分类 / 定级 / 合并 / 过期 / 优先级 / 状态真实性 / 加权轮换 / 事件动作」
 *       这些可判定规则交给脚本，避免只靠人工观感验收。
 * 退出码：0 全部通过 / 1 有断言失败 / 2 打包或读取失败。
 */
import { build } from 'esbuild';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = process.cwd();
const workDir = mkdtempSync(join(tmpdir(), 'sugt-notice-model-'));
const bundleOptions = { bundle: true, platform: 'node', format: 'esm', logLevel: 'silent' };

let model;
let petState;
try {
  const modelPath = join(workDir, 'message-model.mjs');
  const petStatePath = join(workDir, 'pet-state.mjs');
  await build({ ...bundleOptions, entryPoints: [join(root, 'src/notify/messageModel.ts')], outfile: modelPath });
  await build({ ...bundleOptions, entryPoints: [join(root, 'src/pet/petState.ts')], outfile: petStatePath });
  model = await import(pathToFileURL(modelPath).href);
  petState = await import(pathToFileURL(petStatePath).href);
} catch (error) {
  console.error(`[error] 打包通知模型失败：${error?.message || error}`);
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

const NOW = 1_000_000;

// 一、定级与停留时长
check('错误文案自动定级', model.guessSeverity('请求失败：invalid token') === 'error');
check('警告文案自动定级', model.guessSeverity('正在重试第 2 次') === 'warn');
check('中性文案自动定级', model.guessSeverity('已保存') === 'info');
check('错误类叠加停留时长',
  model.noticeTtlMs('result', 'error') > model.noticeTtlMs('result', 'ok'));
check('显式 ttl 覆盖分类默认值', model.noticeTtlMs('idle', 'info', 777) === 777);
check('创建消息带分类/级别/去重键', (() => {
  const notice = model.createNotice(1, { text: '  保存成功  ', category: 'result', severity: 'ok' }, NOW);
  return notice.text === '保存成功'
    && notice.category === 'result'
    && notice.severity === 'ok'
    && notice.surface === 'toast'
    && notice.count === 1
    && notice.dedupeKey.includes('保存成功');
})());
check('旧式字符串 + kind 仍可创建', (() => {
  const notice = model.createNotice(2, '网关启动失败', NOW, 'error');
  return notice.severity === 'error' && notice.category === 'result';
})());

// 二、合并去重与生命周期
check('同键窗口内合并计数', (() => {
  const first = model.createNotice(3, { text: '刷新完成', category: 'result' }, NOW);
  const second = model.createNotice(4, { text: '刷新完成', category: 'result' }, NOW + 500);
  const result = model.mergeNotices([first], second, NOW + 500);
  return result.mergedId === 3 && result.list.length === 1 && result.list[0].count === 2;
})());
check('超出合并窗口不合并', (() => {
  const first = model.createNotice(5, { text: '刷新完成', category: 'result' }, NOW);
  const second = model.createNotice(6, { text: '刷新完成', category: 'result' }, NOW + 3000);
  const result = model.mergeNotices([first], second, NOW + 3000);
  return result.mergedId === null && result.list.length === 2;
})());
check('不同文案不合并', (() => {
  const first = model.createNotice(7, { text: 'A', category: 'result' }, NOW);
  const second = model.createNotice(8, { text: 'B', category: 'result' }, NOW + 100);
  return model.mergeNotices([first], second, NOW + 100).list.length === 2;
})());
check('过期消息被清理', (() => {
  const idle = model.createNotice(9, { text: '画个圈圈', category: 'idle' }, NOW);
  return model.pruneNotices([idle], NOW + idle.ttlMs + 1).length === 0;
})());
check('暂停后时间基准顺延', (() => {
  const idle = model.createNotice(10, { text: '画个圈圈', category: 'idle' }, NOW);
  const shifted = model.shiftNoticeDeadlines([idle], 5000)[0];
  return shifted.updatedAt === NOW + 5000
    && model.pruneNotices([shifted], NOW + 3000).length === 1;
})());

// 三、优先级与可视条数
check('错误结果优先于闲置消息', (() => {
  const idle = model.createNotice(11, { text: '闲置', category: 'idle' }, NOW + 10);
  const error = model.createNotice(12, { text: '失败', category: 'result', severity: 'error' }, NOW);
  const { visible } = model.selectVisible([idle, error], 1);
  return visible.length === 1 && visible[0].id === 12;
})());
check('超出容量给出被挤出条数', (() => {
  const list = [13, 14, 15, 16].map((id) => model.createNotice(id, { text: `消息${id}`, category: 'result' }, NOW + id));
  const { visible, hidden } = model.selectVisible(list, 2);
  return visible.length === 2 && hidden === 2;
})());

// 四、状态消息只由真实在途产生
check('在途归零不发状态消息', !model.canEmitStateNotice({
  activeRequests: 0, requestSinceMs: 20000, nowMs: NOW, lastEmitAt: 0,
}));
check('短请求不发状态消息', !model.canEmitStateNotice({
  activeRequests: 1, requestSinceMs: 800, nowMs: NOW, lastEmitAt: 0,
}));
check('冷却期内不重复发状态消息', !model.canEmitStateNotice({
  activeRequests: 2, requestSinceMs: 20000, nowMs: NOW, lastEmitAt: NOW - 3000,
}));
check('真实长请求允许发状态消息', model.canEmitStateNotice({
  activeRequests: 2, requestSinceMs: 20000, nowMs: NOW, lastEmitAt: 0,
}));

// 五、闲置 / 交互 / 事件映射
check('新闲置动作有对应台词',
  model.idleMessageKeyFor('kickflip') === 'pet.bubble.idleKickflip'
  && model.idleMessageKeyFor('squatDraw') === 'pet.bubble.idleSquatDraw');
check('未知闲置姿势有兜底台词', model.idleMessageKeyFor('unknownPose') === 'pet.bubble.idle1');
check('交互意图细分台词',
  model.interactionMessageKeyFor('headPat') === 'pet.bubble.headPat'
  && model.interactionMessageKeyFor('tripleTap') === 'pet.bubble.tripleTap');
check('事件动作全部是 auto 类姿势', (() => {
  const stateActions = new Set(['typing', 'error', 'done', 'sleep', 'offline', 'idle', 'waitState']);
  return Object.values(model.PET_EVENT_ACTIONS).every((pose) => !stateActions.has(pose));
})());
check('长请求映射到 wait 姿势', model.eventActionFor('longRequest') === 'wait');

// 六、加权轮换
const poses = ['stretch', 'squatDraw', 'kickflip', 'miniGame'];
check('加权轮换不与上一次重复', (() => {
  for (let i = 0; i < 40; i += 1) {
    const picked = model.weightedIdleVariant(poses, undefined, ['stretch'], () => (i + 0.5) / 40);
    if (picked === 'stretch') return false;
  }
  return true;
})());
check('加权轮换结果始终在姿势池内', (() => {
  for (let i = 0; i < 40; i += 1) {
    const picked = model.weightedIdleVariant(poses, undefined, [], () => (i + 0.5) / 40);
    if (!poses.includes(picked)) return false;
  }
  return true;
})());
check('空姿势池有兜底', model.weightedIdleVariant([], undefined, [], () => 0) === 'stretch');
check('petState 新导出可用且不连播重复', (() => {
  if (typeof petState.pickIdleVariantWeighted !== 'function') return false;
  if (typeof petState.eventActionFlag !== 'function') return false;
  const picked = petState.pickIdleVariantWeighted(poses, undefined, ['kickflip'], () => 0.5);
  return poses.includes(picked) && picked !== 'kickflip'
    && petState.eventActionFlag('longRequest') === 'wait';
})());

rmSync(workDir, { recursive: true, force: true });
if (failed) {
  console.error(`notice-model smoke: ${failed} failed`);
  process.exit(1);
}
console.log('notice-model smoke: all passed');

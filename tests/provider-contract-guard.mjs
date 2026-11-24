#!/usr/bin/env node
/**
 * ProviderView 映射字段完整性守卫（WB-001 / WB-006 回归防护）。
 *
 * 目的：控制台切换模型会把 provider 原样回写 save_provider。只要下面任一环
 * 漏字段，model_alias 会被后端回落成 CLIENT_MODEL_ID、文本重试配置被重置为
 * 默认值，而这在 UI 上无提示。报告 WB-001 第 7 项要求补「前端映射字段完整性
 * 断言」，本脚本即该断言：静态核对数据链每一环的字段契约。
 *
 * 检查项（数据链：后端 ProviderView → main.tsx ProviderView 类型 → 映射 →
 *          GatewayAccessBanner.ProviderOption → UpstreamSelector.ProviderOption）：
 *  1. src/main.tsx 中 providers={providers.map(...)} 的映射包含后端 ProviderView
 *     下发的全部 18 个字段；
 *  2. main.tsx ProviderView 类型、GatewayAccessBanner.tsx ProviderOption 类型、
 *     UpstreamSelector.tsx ProviderOption 类型均包含 8 个关键契约字段
 *     （model_alias / public_model_id / tool_text_retry_enabled /
 *     tool_text_retry_extra / auto_adapt_base_url / status / last_checked_at /
 *     last_error）。
 *
 * 用法：node tests/provider-contract-guard.mjs [--root <项目根>]
 *      --root 用于自证：指向临时副本目录，避免改动仓库源文件。
 * 退出码：0 通过 / 1 契约缺字段 / 2 文件读取失败
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const rootArg = process.argv.indexOf('--root');
const root = rootArg > -1 ? path.resolve(process.argv[rootArg + 1]) : projectRoot;

const FILES = {
  main: path.join(root, 'src/main.tsx'),
  banner: path.join(root, 'src/GatewayAccessBanner.tsx'),
  selector: path.join(root, 'src/UpstreamSelector.tsx'),
};

/** 后端 src-tauri/src/model.rs ProviderView 下发的全部字段（18 个）。 */
const ALL_FIELDS = [
  'id',
  'name',
  'provider',
  'base_url',
  'api_key',
  'api_key_masked',
  'model_name',
  'model_alias',
  'public_model_id',
  'protocol',
  'enabled',
  'status',
  'last_checked_at',
  'last_error',
  'auto_adapt_base_url',
  'tool_text_retry_enabled',
  'tool_text_retry_extra',
  'experimental',
];

/** WB-001 / WB-006 关键契约字段：任一环缺失都会静默清空用户配置。 */
const FOCUS_FIELDS = [
  'model_alias',
  'public_model_id',
  'tool_text_retry_enabled',
  'tool_text_retry_extra',
  'auto_adapt_base_url',
  'status',
  'last_checked_at',
  'last_error',
];

const sources = {};
for (const [key, file] of Object.entries(FILES)) {
  try {
    sources[key] = readFileSync(file, 'utf8');
  } catch (error) {
    console.error(`[provider-contract] 无法读取 ${file}: ${error.message}`);
    process.exit(2);
  }
}

/** 剥掉注释，避免注释里提到的字段名造成误判。 */
const stripComments = (text) =>
  text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/[^\n]*/g, '');

/** 取 providers={providers.map(...)} 的对象字面量块（到首个 }))} 为止）。 */
function extractMapping(src) {
  const start = src.indexOf('providers={providers.map');
  if (start < 0) return null;
  const end = src.indexOf('}))}', start);
  if (end < 0) return null;
  return src.slice(start, end);
}

/** 取 type <name> = { ... } 的类型体（这些类型无嵌套大括号）。 */
function extractType(src, name) {
  const re = new RegExp('type\\s+' + name + '\\s*=\\s*\\{([\\s\\S]*?)\\n\\}');
  const match = re.exec(src);
  return match ? match[1] : null;
}

const failures = [];
function assertFields(label, block, fields) {
  if (block == null) {
    failures.push(`${label}：未找到目标代码块`);
    return;
  }
  const clean = stripComments(block);
  const missing = fields.filter(
    (field) => !new RegExp('\\b' + field + '\\s*\\??\\s*:').test(clean),
  );
  if (missing.length > 0) {
    failures.push(`${label}：缺少字段 ${missing.join(', ')}`);
  }
}

assertFields(
  'src/main.tsx providers 映射',
  extractMapping(sources.main),
  ALL_FIELDS,
);
assertFields(
  'src/main.tsx ProviderView 类型',
  extractType(sources.main, 'ProviderView'),
  FOCUS_FIELDS,
);
assertFields(
  'src/GatewayAccessBanner.tsx ProviderOption 类型',
  extractType(sources.banner, 'ProviderOption'),
  FOCUS_FIELDS,
);
assertFields(
  'src/UpstreamSelector.tsx ProviderOption 类型',
  extractType(sources.selector, 'ProviderOption'),
  FOCUS_FIELDS,
);

if (failures.length > 0) {
  for (const item of failures) console.error(`[provider-contract] ${item}`);
  console.error(
    `[provider-contract] 失败：${failures.length} 处契约不完整，` +
      '切模型时模型别名/重试配置可能被静默清空（WB-001/006）',
  );
  process.exit(1);
}

console.log(
  `[provider-contract] 通过：providers 映射含后端全部 ${ALL_FIELDS.length} 个字段，` +
    `数据链三处类型均覆盖 ${FOCUS_FIELDS.length} 个关键契约字段（root=${root}）`,
);

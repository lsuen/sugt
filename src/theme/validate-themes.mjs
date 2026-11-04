#!/usr/bin/env node
/**
 * 主题契约校验：内置 CSS 皮肤、内置 JSON 主题包、Agent 模板包。
 * 作者：sun / 孙文龙
 * 运行：npm run test:themes
 */
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const THEME_DIR = dirname(fileURLToPath(import.meta.url));
const ROOT = join(THEME_DIR, '../..');
const SCHEMA_PATH = join(THEME_DIR, 'schema.ts');
const CSS_THEMES_DIR = join(THEME_DIR, 'themes');
const PACKS_DIR = join(THEME_DIR, 'packs');
const REGISTRY_PATH = join(THEME_DIR, 'registry.ts');
const EXAMPLE_JSON = join(ROOT, 'theme-packs/example/theme.json');

const schemaSrc = readFileSync(SCHEMA_PATH, 'utf8');
const keysMatch = schemaSrc.match(/export const THEME_TOKEN_KEYS = \[([\s\S]*?)\] as const/);
if (!keysMatch) {
  console.error('无法从 schema.ts 解析 THEME_TOKEN_KEYS');
  process.exit(1);
}
const TOKEN_KEYS = [...keysMatch[1].matchAll(/'([^']+)'/g)].map((m) => m[1]);
if (TOKEN_KEYS.length < 20) {
  console.error(`TOKEN_KEYS 数量异常：${TOKEN_KEYS.length}`);
  process.exit(1);
}

let failed = 0;
function fail(message) {
  console.error(`[fail] ${message}`);
  failed += 1;
}
function ok(message) {
  console.log(`[ok] ${message}`);
}

const RELATIVE_URL = /url\(\s*["']?\.\/([^"')]+)["']?\s*\)/;

/** 检查相对背景资源是否真实存在，防止注入失效的 URL。 */
function checkRelativeAsset(sourceDir, cssValue, label) {
  const match = RELATIVE_URL.exec(String(cssValue ?? ''));
  if (match && !existsSync(join(sourceDir, match[1]))) {
    fail(`${label}: 声明了背景 ${match[1]}，但文件不存在`);
  }
}

/** 校验一份 CSS 皮肤：选择器、完整契约、无契约外变体令牌、资源可用。 */
function checkCssTheme(cssPath, id, label) {
  const src = readFileSync(cssPath, 'utf8');
  const selector = `:root[data-theme="${id}"]`;
  if (!src.includes(selector)) {
    fail(`${label}: 缺少选择器 ${selector}（必须带 :root，否则会被默认令牌覆盖）`);
  }
  if (/(^|[\s,}])\[data-theme="[^"]+"\]/.test(src)) {
    fail(`${label}: 使用了裸 [data-theme] 选择器`);
  }
  if (/\[data-theme="[^"]+"\]\s+\.[A-Za-z]/.test(src)) {
    fail(`${label}: 存在针对业务类名的主题覆盖块`);
  }
  const block = src.match(/:root\[data-theme="[^"]+"\]\s*\{([\s\S]*?)\}/);
  if (!block) {
    fail(`${label}: 找不到主题声明块`);
    return;
  }
  const declared = [...new Set([...block[1].matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]))];
  const missing = TOKEN_KEYS.filter((key) => !declared.includes(key));
  // `--_xxx` 是主题文件内部允许的临时变量（见 THEME_AUTHORING 文档）。
  const unknown = declared.filter(
    (key) => !TOKEN_KEYS.includes(key) && !key.startsWith('--_'),
  );
  if (missing.length) fail(`${label}: 缺失令牌 ${missing.join(', ')}`);
  if (unknown.length) fail(`${label}: 契约外令牌 ${unknown.join(', ')}`);
  const bg = /--bg-image\s*:\s*([^;]+);/.exec(block[1]);
  checkRelativeAsset(dirname(cssPath), bg?.[1], label);
  if (!missing.length && !unknown.length) ok(`${label} (${TOKEN_KEYS.length} tokens)`);
}

/** 校验一份 JSON 主题包：版本、id、双语标签、令牌、背景资源。 */
function checkPackJson(jsonPath, expectedId, label) {
  let pack;
  try {
    pack = JSON.parse(readFileSync(jsonPath, 'utf8'));
  } catch (err) {
    fail(`${label}: JSON 解析失败 ${err.message}`);
    return;
  }
  if (pack.version !== 1) fail(`${label}: version 必须为 1，当前 ${pack.version}`);
  if (!/^[a-z0-9][a-z0-9_-]{0,47}$/i.test(pack.id ?? '')) {
    fail(`${label}: id 不合法 ${pack.id}`);
  }
  if (expectedId && pack.id !== expectedId) {
    fail(`${label}: id 与目录名不一致（${pack.id} != ${expectedId}）`);
  }
  if (!pack.label?.zh || !pack.label?.en) fail(`${label}: 缺少 label.zh / label.en`);
  const tokens = pack.tokens ?? {};
  const missing = TOKEN_KEYS.filter(
    (key) => !tokens[key] || String(tokens[key]).trim() === '',
  );
  const unknown = Object.keys(tokens).filter((key) => !TOKEN_KEYS.includes(key));
  if (missing.length) fail(`${label}: 缺失令牌 ${missing.join(', ')}`);
  if (unknown.length) fail(`${label}: 契约外令牌 ${unknown.join(', ')}`);
  const dir = dirname(jsonPath);
  if (pack.assets?.bgImage && !existsSync(join(dir, pack.assets.bgImage))) {
    fail(`${label}: assets.bgImage 声明了 ${pack.assets.bgImage}，但文件不存在`);
  }
  checkRelativeAsset(dir, tokens['--bg-image'], label);
  if (!missing.length && !unknown.length) ok(`${label} (${TOKEN_KEYS.length} tokens)`);
}

// 1) 内置 CSS 皮肤
const cssFiles = readdirSync(CSS_THEMES_DIR)
  .filter((file) => file.endsWith('.css'))
  .sort();
for (const file of cssFiles) {
  checkCssTheme(join(CSS_THEMES_DIR, file), basename(file, '.css'), `themes/${file}`);
}

// 2) 内置 JSON 主题包（含配套的 theme.css）
const packIds = readdirSync(PACKS_DIR, { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name)
  .sort();
for (const id of packIds) {
  const jsonPath = join(PACKS_DIR, id, 'theme.json');
  if (!existsSync(jsonPath)) fail(`packs/${id}: 缺少 theme.json`);
  else checkPackJson(jsonPath, id, `packs/${id}/theme.json`);
  const cssPath = join(PACKS_DIR, id, 'theme.css');
  if (!existsSync(cssPath)) fail(`packs/${id}: 缺少 theme.css`);
  else checkCssTheme(cssPath, id, `packs/${id}/theme.css`);
}

// 3) 给 Agent / 用户的模板包（id 不要求等于目录名）
if (!existsSync(EXAMPLE_JSON)) fail('theme-packs/example/theme.json 缺失');
else checkPackJson(EXAMPLE_JSON, null, 'theme-packs/example/theme.json');

// 4) registry 与主题源文件双向一致
const registrySrc = readFileSync(REGISTRY_PATH, 'utf8');
const registered = [...new Set([...registrySrc.matchAll(/id: '([^']+)'/g)].map((m) => m[1]))];
console.log(`registry themes: ${registered.join(', ')}`);
for (const id of registered) {
  if (
    !existsSync(join(CSS_THEMES_DIR, `${id}.css`)) &&
    !existsSync(join(PACKS_DIR, id, 'theme.json'))
  ) {
    fail(`registry 注册了 ${id}，但没有对应主题源文件`);
  }
}
const sourceIds = [...cssFiles.map((file) => basename(file, '.css')), ...packIds];
for (const id of sourceIds) {
  if (!registered.includes(id)) fail(`主题 ${id} 未在 registry.ts 注册`);
}
for (const file of cssFiles) {
  if (!registrySrc.includes(`./themes/${file}`)) fail(`registry.ts 未引入 ./themes/${file}`);
}
for (const id of packIds) {
  if (!registrySrc.includes(`./packs/${id}/theme.css`)) {
    fail(`registry.ts 未引入 ./packs/${id}/theme.css`);
  }
}

if (failed) {
  console.error(`\nFAILED: ${failed} issue(s)`);
  process.exit(1);
}
console.log(`\nAll theme sources valid (${TOKEN_KEYS.length} tokens each).`);

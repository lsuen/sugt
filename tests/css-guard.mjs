#!/usr/bin/env node
/**
 * 控件样式守卫（B4）。
 *
 * 目的：输入框 / 下拉 / 多行文本的圆角与内边距必须来自控件原语层，
 * 不允许再在业务选择器里写死 `border-radius: 10px` 之类。否则同一个产品里
 * 会出现「有的输入框圆角 8px、有的 10px、有的是直角」的老问题。
 *
 * 检查项：
 *  1. :root 定义了 --control-radius / --control-pad-y / --control-pad-x；
 *  2. 控件原语选择器（input / select / textarea / .field-input 等）存在；
 *  3. 任何命中 input / select / textarea 的选择器块里，border-radius 必须是 var(...)。
 *
 * 用法：node tests/css-guard.mjs [--file src/styles.css]
 * 退出码：0 通过 / 1 发现违规 / 2 文件读取失败
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const argFile = process.argv.indexOf('--file');
const cssPath = argFile > -1 ? path.resolve(process.argv[argFile + 1]) : path.join(root, 'src/styles.css');

let css;
try {
  css = readFileSync(cssPath, 'utf8');
} catch (error) {
  console.error(`[css-guard] 无法读取 ${cssPath}: ${error.message}`);
  process.exit(2);
}

/** 去掉注释，避免被注释里的示例规则误判。 */
const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');

/**
 * 需要统一外观的控件：裸元素，或以 input / select / textarea 结尾的类名
 * （`.app-select`、`.me-input`、`.quick-chat-input`）。`.preset-select-label`
 * 这类「词中间恰好含 select」的类名不算。
 */
const CONTROL_ELEMENT =
  /(?<![\w.#-])(input|select|textarea)(?![\w-])|\.[\w-]*(?:input|select|textarea)(?![\w-])/;
/** 这些控件有独立的开关/滑块形态，允许自定义圆角。 */
const EXEMPT = /checkbox|radio|range|color|file|switch/i;

const violations = [];
const lineOf = (index) => stripped.slice(0, index).split('\n').length;

// 逐块扫描 `selector { body }`：@media 外层也会被匹配到，
// 但它的选择器以 @ 开头且不含控件元素名，自然被跳过。
const rulePattern = /([^{}]+)\{([^{}]*)\}/g;
let match;
while ((match = rulePattern.exec(stripped)) !== null) {
  const selector = match[1].trim();
  const body = match[2];
  if (!selector || selector.startsWith('@')) continue;
  // `:not([type="checkbox"])` 是对控件的排除，不代表这个规则本身在管开关，
  // 先剥掉再判断，否则 `.modal-body input:not([type=checkbox])` 会被整体误放行。
  const controls = selector
    .split(',')
    .map((part) => part.trim())
    .filter((part) => CONTROL_ELEMENT.test(part))
    .filter((part) => !EXEMPT.test(part.replace(/:not\([^)]*\)/g, '')));
  if (controls.length === 0) continue;

  for (const declaration of body.split(';')) {
    const [rawProperty, ...rest] = declaration.split(':');
    if (!rawProperty || rest.length === 0) continue;
    if (rawProperty.trim() !== 'border-radius') continue;
    const value = rest.join(':').trim();
    if (value.startsWith('var(')) continue;
    violations.push({
      line: lineOf(match.index),
      selector: controls.map((item) => item.trim()).join(', '),
      value,
    });
  }
}

// 原语层本身必须存在，否则上面的规则形同虚设。
const required = ['--control-radius', '--control-pad-y', '--control-pad-x'];
const missingTokens = required.filter((token) => !stripped.includes(`${token}:`));
const primitiveSelectors = ['.field-input', '.field-select', '.field-textarea'];
const missingPrimitives = primitiveSelectors.filter((selector) => !stripped.includes(selector));

if (missingTokens.length > 0) {
  console.error(`[css-guard] :root 缺少控件 token：${missingTokens.join(', ')}`);
}
if (missingPrimitives.length > 0) {
  console.error(`[css-guard] 缺少控件原语选择器：${missingPrimitives.join(', ')}`);
}
for (const item of violations) {
  console.error(
    `[css-guard] ${cssPath}:${item.line} 选择器「${item.selector}」写死 border-radius: ${item.value}，请改用 var(--control-radius) 或删除该声明`,
  );
}

const failed = violations.length > 0 || missingTokens.length > 0 || missingPrimitives.length > 0;
if (failed) {
  console.error(`[css-guard] 失败：${violations.length} 处写死圆角，${missingTokens.length} 个 token 缺失，${missingPrimitives.length} 个原语缺失`);
  process.exit(1);
}
console.log(`[css-guard] 通过：控件圆角与内边距统一走原语层（${cssPath}）`);

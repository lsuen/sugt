#!/usr/bin/env node
/**
 * 主题包导入功能冒烟测试：真实执行 themePackToCss / parseThemePackJson。
 * 作者：sun / 孙文龙
 * 运行：npm run test:theme-pack
 */
import { build } from 'esbuild';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = process.cwd();
const workDir = mkdtempSync(join(tmpdir(), 'sugt-theme-smoke-'));
const bundlePath = join(workDir, 'theme-bundle.mjs');

await build({
  entryPoints: [join(root, 'src/theme/loadThemePack.ts')],
  bundle: true,
  platform: 'node',
  format: 'esm',
  outfile: bundlePath,
  logLevel: 'silent',
});
const { themePackToCss, parseThemePackJson } = await import(pathToFileURL(bundlePath).href);

let failed = 0;
function check(name, condition, detail) {
  if (condition) {
    console.log(`[ok] ${name}`);
  } else {
    console.error(`[fail] ${name}${detail ? ` — ${detail}` : ''}`);
    failed += 1;
  }
}

const example = JSON.parse(
  readFileSync(join(root, 'theme-packs/example/theme.json'), 'utf8'),
);
const exampleCss = themePackToCss(parseThemePackJson(example), '');
check(
  '模板包生成 :root[data-theme] 选择器',
  exampleCss.includes(':root[data-theme="example-dusk"]'),
  exampleCss,
);
check('模板包不注入失效背景 URL', exampleCss.includes('--bg-image: none;'), exampleCss);

// 用户包声明了背景资源，但 zip / 文件夹里没有该文件：本地导入必须回退纯色。
const broken = {
  version: 1,
  id: 'broken-pack',
  label: { zh: '缺图包', en: 'Missing art' },
  assets: { bgImage: 'bg.jpg' },
  tokens: {
    ...example.tokens,
    '--bg-image': 'url("./bg.jpg")',
    '--bg-image-opacity': '0.4',
  },
};
const brokenCss = themePackToCss(parseThemePackJson(broken), '');
check('声明了缺失背景时回退纯色', brokenCss.includes('--bg-image: none;'), brokenCss);
check('回退时关闭背景透明度', brokenCss.includes('--bg-image-opacity: 0;'), brokenCss);

// 远程包带资源基路径，应把相对资源解析到 JSON 所在目录。
const remoteCss = themePackToCss(
  parseThemePackJson(broken),
  'https://example.com/skins/',
);
check(
  '远程包按 JSON 目录解析背景资源',
  remoteCss.includes('https://example.com/skins/bg.jpg'),
  remoteCss,
);

// 已经嵌入 data URL 的包不能被回退掉。
const embedded = {
  ...broken,
  assets: undefined,
  tokens: {
    ...broken.tokens,
    '--bg-image': 'url("data:image/png;base64,AAAA")',
  },
};
const embeddedCss = themePackToCss(parseThemePackJson(embedded), '');
check(
  '已嵌入 data URL 的背景被保留',
  embeddedCss.includes('data:image/png;base64,AAAA'),
  embeddedCss,
);

// 契约：拒绝未知 token，缺省 token 用默认值补全。
try {
  parseThemePackJson({
    ...example,
    tokens: { ...example.tokens, '--brand-pink-3': '#ff00ff' },
  });
  check('契约外 token 被拒绝', false, '未抛出错误');
} catch (err) {
  check(
    '契约外 token 被拒绝',
    /not in contract|unknown tokens|契约外/.test(err.message),
    err.message,
  );
}
const partial = parseThemePackJson({
  version: 1,
  id: 'partial-pack',
  label: { zh: '部分令牌', en: 'Partial' },
  tokens: { '--accent': '#ff0000' },
});
check(
  '缺省令牌由默认值补全为完整契约',
  Object.keys(partial.tokens).length === 38,
  `实际 ${Object.keys(partial.tokens).length} 个`,
);

rmSync(workDir, { recursive: true, force: true });
if (failed) {
  console.error(`\nFAILED: ${failed} case(s)`);
  process.exit(1);
}
console.log('\nAll theme pack smoke tests passed.');

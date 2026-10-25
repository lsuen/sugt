#!/usr/bin/env node
/**
 * Validate built-in theme packs (JSON) against the SUGT token contract.
 * Run: node scripts/validate-themes.mjs
 */
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const THEME_DIR = dirname(fileURLToPath(import.meta.url));
const ROOT = join(THEME_DIR, '../..');
const SCHEMA_PATH = join(THEME_DIR, 'schema.ts');
const PACKS_DIR = join(THEME_DIR, 'packs');

const schemaSrc = readFileSync(SCHEMA_PATH, 'utf8');
const keysMatch = schemaSrc.match(/export const THEME_TOKEN_KEYS = \[([\s\S]*?)\] as const/);
if (!keysMatch) {
  console.error('Could not parse THEME_TOKEN_KEYS from schema.ts');
  process.exit(1);
}
const TOKEN_KEYS = [...keysMatch[1].matchAll(/'([^']+)'/g)].map((m) => m[1]);
if (TOKEN_KEYS.length < 20) {
  console.error('TOKEN_KEYS looks too small:', TOKEN_KEYS.length);
  process.exit(1);
}

let failed = 0;
const packs = readdirSync(PACKS_DIR, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => d.name);

for (const id of packs) {
  const jsonPath = join(PACKS_DIR, id, 'theme.json');
  if (!existsSync(jsonPath)) {
    console.error(`[${id}] missing theme.json`);
    failed += 1;
    continue;
  }
  const pack = JSON.parse(readFileSync(jsonPath, 'utf8'));
  if (pack.version !== 1) {
    console.error(`[${id}] bad version ${pack.version}`);
    failed += 1;
  }
  if (pack.id !== id) {
    console.error(`[${id}] id mismatch: ${pack.id}`);
    failed += 1;
  }
  const missing = TOKEN_KEYS.filter((k) => !pack.tokens?.[k]);
  const unknown = Object.keys(pack.tokens || {}).filter((k) => !TOKEN_KEYS.includes(k));
  if (missing.length) {
    console.error(`[${id}] missing tokens: ${missing.join(', ')}`);
    failed += 1;
  }
  if (unknown.length) {
    console.error(`[${id}] unknown tokens: ${unknown.join(', ')}`);
    failed += 1;
  }
  if (pack.assets?.bgImage) {
    const asset = join(PACKS_DIR, id, pack.assets.bgImage);
    if (!existsSync(asset)) {
      console.error(`[${id}] missing asset ${pack.assets.bgImage}`);
      failed += 1;
    }
  }
  if (!missing.length && !unknown.length) {
    console.log(`[ok] ${id} (${TOKEN_KEYS.length} tokens)`);
  }
}

// Also require one CSS file per builtin theme id listed in registry
const registry = readFileSync(join(ROOT, 'src/theme/registry.ts'), 'utf8');
const ids = [...registry.matchAll(/id: '([^']+)'/g)].map((m) => m[1]);
const unique = [...new Set(ids)];
console.log(`registry themes: ${unique.join(', ')}`);

if (failed) {
  console.error(`\nFAILED: ${failed} issue(s)`);
  process.exit(1);
}
console.log('\nAll theme packs valid.');

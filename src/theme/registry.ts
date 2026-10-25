import type { BuiltinThemeId, ThemeMeta } from './types';

// Order matters: defaults first, then per-theme overrides (higher specificity), then shell bindings.
import './tokens.css';
import './themes/cyber.css';
import './themes/street.css';
import './themes/skate.css';
import './themes/brickyard.css';
import './themes/shanshui.css';
import './themes/hiphop.css';
import './themes/jazz.css';
import './packs/anime-boy/theme.css';
import './packs/anime-girl/theme.css';
import './shell-theme.css';

export const DEFAULT_THEME_ID: BuiltinThemeId = 'cyber';

export const BUILTIN_THEME_IDS: BuiltinThemeId[] = [
  'cyber',
  'street',
  'skate',
  'brickyard',
  'shanshui',
  'hiphop',
  'jazz',
  'anime-boy',
  'anime-girl',
];

export function isBuiltinThemeId(id: string): id is BuiltinThemeId {
  return (BUILTIN_THEME_IDS as string[]).includes(id);
}

export const BUILTIN_THEMES: ThemeMeta[] = [
  {
    id: 'cyber',
    labelKey: 'theme.cyber',
    labelZh: '赛博控制台',
    labelEn: 'Cyber Console',
    source: { kind: 'builtin', cssHref: '' },
    swatches: ['#16f1ff', '#6d5cff', '#070b14'],
  },
  {
    id: 'street',
    labelKey: 'theme.street',
    labelZh: '街头粉笔',
    labelEn: 'Street Chalk',
    source: { kind: 'builtin', cssHref: '' },
    swatches: ['#ff2d95', '#00f0ff', '#0d0d0f'],
  },
  {
    id: 'skate',
    labelKey: 'theme.skate',
    labelZh: '滑板场',
    labelEn: 'Skate Park',
    source: { kind: 'builtin', cssHref: '' },
    swatches: ['#ffb020', '#3bb4ff', '#0b1220'],
  },
  {
    id: 'brickyard',
    labelKey: 'theme.brickyard',
    labelZh: '嘻哈砖墙',
    labelEn: 'Brickyard',
    source: { kind: 'builtin', cssHref: '' },
    swatches: ['#c43c2c', '#e8b84a', '#140c0c'],
  },
  {
    id: 'shanshui',
    labelKey: 'theme.shanshui',
    labelZh: '山水墨',
    labelEn: 'Shan-shui Ink',
    source: { kind: 'builtin', cssHref: '' },
    swatches: ['#5f93a8', '#d05a48', '#0b0e12'],
  },
  {
    id: 'hiphop',
    labelKey: 'theme.hiphop',
    labelZh: 'HIPHOP',
    labelEn: 'HIPHOP',
    source: { kind: 'builtin', cssHref: '' },
    swatches: ['#f9961a', '#ffc15a', '#0a0a0c'],
  },
  {
    id: 'jazz',
    labelKey: 'theme.jazz',
    labelZh: '爵士夜',
    labelEn: 'Jazz Night',
    source: { kind: 'builtin', cssHref: '' },
    swatches: ['#d4af37', '#8b2240', '#08070d'],
  },
  {
    id: 'anime-boy',
    labelKey: 'theme.animeBoy',
    labelZh: '二次元男',
    labelEn: 'Anime Boy',
    source: { kind: 'builtin', cssHref: '' },
    swatches: ['#5ec8ff', '#7a8cff', '#071018'],
  },
  {
    id: 'anime-girl',
    labelKey: 'theme.animeGirl',
    labelZh: '二次元女',
    labelEn: 'Anime Girl',
    source: { kind: 'builtin', cssHref: '' },
    swatches: ['#ff8fb8', '#c9a0ff', '#140c14'],
  },
];

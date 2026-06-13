export const CURRENT_VERSION = '0.2.0';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

/** 仅展示当前版本相对上一版的业务向改动（2～3 条），完整历史见 Git 记录。 */
export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '商店版：发现技能在线安装到暂存区，并一键挂到 Claude ~/.claude/skills' },
  { type: 'feat', text: '内置 4 个默认技能仓库，支持自定义 GitHub 源与 shallow 刷新' },
  { type: 'feat', text: 'Claude 插件只读展示；Codex 技能商店规划中' },
];

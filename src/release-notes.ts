export const CURRENT_VERSION = '0.2.1';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

/** 仅展示当前版本相对上一版的业务向改动（2～3 条），完整历史见 Git 记录。 */
export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '发现技能增加「已暂存/已挂载」筛选；客户端环境/技能/插件平级 Tab，Codex 与 Claude 共用技能商店' },
  { type: 'fix', text: '列表区域可滚动；刷新仓库在页面内显示状态，git 后台执行不再弹出控制台' },
  { type: 'fix', text: '插件页展示官方安装命令说明；技能可分别挂到 ~/.claude/skills 与 ~/.agents/skills' },
];

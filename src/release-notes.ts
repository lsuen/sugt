export const CURRENT_VERSION = '1.1.1';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '正式发布 1.1.1：公开版（Windows 半年有效期、macOS 无时间限制），界面中不再展示有效期信息' },
  { type: 'feat', text: '界面国际化，支持跟随系统 / 中文 / English 切换并实时刷新' },
  { type: 'feat', text: '流量悬浮窗支持横竖排布局、尺寸调节与恢复默认' },
  { type: 'fix', text: '修复 Moonshot/Kimi 自动模式下上游返回压缩流导致 AI 客户端空响应的问题' },
  { type: 'feat', text: '技能发现默认推荐 Gitee 库，支持启动预热；Anthropic 官方技能可经 GitHub 代理同步' },
  { type: 'feat', text: '客户端支持发现本机工具、高级页管理技能 / 插件 / 配置路径' },
  { type: 'fix', text: '表单弹窗默认禁止点遮罩关闭，避免误触丢失编辑内容' },
];

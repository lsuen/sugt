export const CURRENT_VERSION = '1.0.0';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '正式发布 1.0.0：本地 AI 网关、模型配置、客户端接管与技能商店一体化' },
  { type: 'feat', text: '技能发现默认推荐 Gitee 库，支持启动预热；Anthropic 官方技能可经 GitHub 代理同步' },
  { type: 'feat', text: '实验性 OpenCode Zen 免费通道（可删可改），连接探测与上游转发体验优化' },
  { type: 'feat', text: '客户端支持发现本机工具、高级页管理技能 / 插件 / 配置路径' },
  { type: 'fix', text: '表单弹窗默认禁止点遮罩关闭，避免误触丢失编辑内容' },
];

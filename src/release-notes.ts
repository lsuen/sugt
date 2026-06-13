export const CURRENT_VERSION = '0.2.3';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '同步网关：控制台请求统计卡片、60 分钟迷你图与调用来源展开' },
  { type: 'feat', text: '同步网关：添加模型支持国内服务商下拉预设（魔搭、DeepSeek、通义等）' },
  { type: 'fix', text: '同步网关：移除顶部全局刷新，改为自动刷新状态' },
  { type: 'feat', text: '发现技能支持 GitHub 克隆代理前缀，可测试并保存（适配国内加速）' },
  { type: 'feat', text: '环境 Tab 增加状态概览、新终端启动客户端、复制网关地址' },
];

export const CURRENT_VERSION = '0.2.2';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '发现技能支持 GitHub 克隆代理前缀，可测试并保存（适配国内加速）' },
  { type: 'feat', text: '环境 Tab 增加状态概览、新终端启动客户端、复制网关地址' },
  { type: 'fix', text: '接管区与技能操作按钮支持换行，避免布局挤压' },
];

export const CURRENT_VERSION = '0.1.2';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

/** 仅展示当前版本相对上一版的改动，完整历史见 Git 记录。 */
export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '连接测试与网关启动错误提示细化（网络/鉴权/模型/端口占用）' },
  { type: 'feat', text: '一键接管前弹窗预览环境变量变更与冲突' },
  { type: 'feat', text: '控制台展示最近请求实际命中的模型（含故障转移）' },
  { type: 'feat', text: 'sugt-cli env install/repair 支持 --start-gateway' },
  { type: 'fix', text: '打包脚本 -SkipBuild 时强警告元数据可能不一致' },
  { type: 'fix', text: '其他的体验优化与 bug 修复' },
];

export const CURRENT_VERSION = '0.1.1';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

/** 仅展示当前版本相对上一版的改动，完整历史见 Git 记录。 */
export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '托盘退出行为可配置：仅退出 / 停网关 / 停网关并关接管' },
  { type: 'feat', text: '控制台新增「启动时自动开网关」开关' },
  { type: 'feat', text: '接管前检查网关状态，支持「启动网关并接管」' },
  { type: 'feat', text: '客户端 Tab：接管检查与一键修复冲突变量' },
  { type: 'fix', text: '托盘点击恢复窗口时取消任务栏隐藏并聚焦' },
  { type: 'fix', text: 'Claude 接管仅写入 ANTHROPIC_AUTH_TOKEN，避免与 API Key 冲突' },
];

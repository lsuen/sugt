export const CURRENT_VERSION = '0.1.2';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

/** 仅展示当前版本相对上一版的业务向改动（2～3 条），完整历史见 Git 记录。 */
export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '连接测试与网关启动失败时，提示更易懂，方便排查 Key、模型或端口问题' },
  { type: 'feat', text: '悬停接管状态可查看环境变量；控制台可看到请求实际使用的模型' },
  { type: 'fix', text: '其他的体验优化与 bug 修复' },
];

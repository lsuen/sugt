export const CURRENT_VERSION = '1.1.6';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: 'SOUL：流式回复、自动同意写入、跨平台打开文件；CLI me send / me run' },
  { type: 'feat', text: 'SOUL：中文路径、写后核对、小游戏兜底；底栏显示项目/模型/网关与快捷键' },
  { type: 'fix', text: 'SOUL：长计划不再误判为说完；未闭合代码块会催促并必要时用内置模板' },
];

export type SkillCatalogItem = {
  id: string;
  name: string;
  description?: string | null;
  repo_id: string;
  repo_label: string;
  relative_path: string;
  staged: boolean;
  mounted: boolean;
  mounted_claude: boolean;
  mounted_codex: boolean;
};

export type SkillRepoView = {
  id: string;
  owner: string;
  repo: string;
  branch: string;
  enabled: boolean;
  clone_url: string;
  label: string;
  last_refresh_at?: string | null;
  last_error?: string | null;
  skill_count: number;
};

export type PluginItemView = {
  name: string;
  path: string;
  source?: string | null;
};

export type PluginInstallGuide = {
  client: string;
  title: string;
  summary: string;
  commands: string[];
  docs_url?: string | null;
  skills_path: string;
  plugins_path?: string | null;
};

export type PluginPanelView = {
  items: PluginItemView[];
  guide: PluginInstallGuide;
};

export type StoreSettings = {
  editor_command: string;
};

export type StoreClientPaths = {
  staging_dir: string;
  claude_skills: string;
  codex_skills: string;
  claude_plugins: string;
};

export type StoreSubview = 'main' | 'discover' | 'repos';

export type StoreClientTab = 'claude' | 'codex';

export type StoreContentTab = 'env' | 'skills' | 'plugins';

export type DiscoverFilter = 'all' | 'available' | 'staged' | 'mounted';

export type MountTarget = 'claude' | 'codex' | 'both';

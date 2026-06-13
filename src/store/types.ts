export type SkillCatalogItem = {
  id: string;
  name: string;
  description?: string | null;
  repo_id: string;
  repo_label: string;
  relative_path: string;
  staged: boolean;
  mounted: boolean;
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

export type StoreSettings = {
  editor_command: string;
};

export type StoreSubview = 'main' | 'discover' | 'repos';

export type StoreClientTab = 'claude' | 'codex';

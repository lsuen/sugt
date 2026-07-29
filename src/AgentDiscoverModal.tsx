import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Search, X } from 'lucide-react';

export type DiscoveredAgent = {
  id: string;
  name: string;
  vendor: string;
  protocol: string;
  confidence: 'high' | 'low' | string;
  already_added: boolean;
  cli_path?: string | null;
  settings_path?: string | null;
  skill_dirs: string[];
  launch_command?: string | null;
  hit_reasons: string[];
  env_vars: Record<string, string>;
  clear_vars: string[];
  warning?: string | null;
  requires_path: boolean;
};

type Props = {
  open: boolean;
  busy: boolean;
  onClose: () => void;
  onAdded: (count: number) => void;
  onError: (msg: string) => void;
};

type RowState = {
  selected: boolean;
  extraPath: string;
};

export function AgentDiscoverModal({ open, busy, onClose, onAdded, onError }: Props) {
  const [query, setQuery] = useState('');
  const [searching, setSearching] = useState(false);
  const [adding, setAdding] = useState(false);
  const [results, setResults] = useState<DiscoveredAgent[]>([]);
  const [rows, setRows] = useState<Record<string, RowState>>({});

  useEffect(() => {
    if (!open) return;
    setQuery('');
    setResults([]);
    setRows({});
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const q = query.trim();
    if (q.length < 2) {
      setResults([]);
      setRows({});
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      setSearching(true);
      invoke<DiscoveredAgent[]>('discover_agents', { query: q })
        .then((list) => {
          if (cancelled) return;
          setResults(list);
          setRows((prev) => {
            const next: Record<string, RowState> = {};
            for (const item of list) {
              next[item.id] = prev[item.id] ?? { selected: false, extraPath: '' };
            }
            return next;
          });
        })
        .catch((e) => {
          if (!cancelled) onError(String(e));
        })
        .finally(() => {
          if (!cancelled) setSearching(false);
        });
    }, 280);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query, open, onError]);

  if (!open) return null;

  const canSelect = (item: DiscoveredAgent, state: RowState) => {
    if (item.already_added) return false;
    if (!item.requires_path) return true;
    return state.extraPath.trim().length > 0;
  };

  const selectable = results.filter((item) => {
    const state = rows[item.id] ?? { selected: false, extraPath: '' };
    return canSelect(item, state);
  });

  const selectedCount = results.filter((item) => rows[item.id]?.selected && canSelect(item, rows[item.id])).length;

  const toggle = (item: DiscoveredAgent) => {
    const state = rows[item.id] ?? { selected: false, extraPath: '' };
    if (!canSelect(item, state)) return;
    setRows((prev) => ({
      ...prev,
      [item.id]: { ...state, selected: !state.selected },
    }));
  };

  const setExtraPath = (id: string, extraPath: string) => {
    setRows((prev) => {
      const cur = prev[id] ?? { selected: false, extraPath: '' };
      const nextPath = extraPath;
      const item = results.find((r) => r.id === id);
      const can = item ? canSelect(item, { ...cur, extraPath: nextPath }) : false;
      return {
        ...prev,
        [id]: {
          extraPath: nextPath,
          selected: can ? cur.selected : false,
        },
      };
    });
  };

  const addSelected = async () => {
    const payload = results
      .filter((item) => {
        const state = rows[item.id];
        return state?.selected && canSelect(item, state);
      })
      .map((item) => ({
        id: item.id,
        name: item.name,
        vendor: item.vendor,
        protocol: item.protocol,
        cliPath: item.cli_path ?? null,
        settingsPath: item.settings_path ?? null,
        skillDirs: item.skill_dirs,
        launchCommand: item.launch_command ?? null,
        hitReasons: item.hit_reasons,
        envVars: item.env_vars,
        clearVars: item.clear_vars,
        requiresPath: item.requires_path,
        extraPath: rows[item.id]?.extraPath?.trim() || null,
      }));
    if (payload.length === 0) {
      onError('请先勾选可添加的 Agent');
      return;
    }
    setAdding(true);
    try {
      const added = await invoke<unknown[]>('add_discovered_agents', { items: payload });
      onAdded(added.length);
      onClose();
    } catch (e) {
      onError(String(e));
    } finally {
      setAdding(false);
    }
  };

  return (
    <div className="modal-overlay">
      <div className="modal takeover-modal agent-discover-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>发现 Agent</h3>
          <button type="button" className="icon-btn" onClick={onClose} aria-label="关闭" disabled={adding}>
            <X size={16} />
          </button>
        </div>
        <div className="modal-body takeover-modal-body">
          <p className="hint compact takeover-modal-hint">
            在系统与用户环境变量、PATH 中搜索。发现不会自动接管，添加后出现在 Agent 列表。
          </p>
          <label className="field-label">
            搜索
            <div className="agent-discover-search">
              <Search size={14} />
              <input
                autoFocus
                placeholder="例如 cursor、claude、code"
                value={query}
                disabled={adding || busy}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
          </label>

          {searching && <p className="hint compact">正在搜索…</p>}
          {!searching && query.trim().length >= 2 && results.length === 0 && (
            <p className="hint compact">未找到匹配项</p>
          )}

          <div className="agent-discover-list">
            {results.map((item) => {
              const state = rows[item.id] ?? { selected: false, extraPath: '' };
              const enabled = canSelect(item, state);
              return (
                <div
                  key={item.id}
                  className={`agent-discover-row${item.already_added ? ' added' : ''}${item.requires_path ? ' low' : ''}`}
                >
                  <label className="agent-discover-check">
                    <input
                      type="checkbox"
                      checked={Boolean(state.selected && enabled)}
                      disabled={!enabled || adding || busy}
                      onChange={() => toggle(item)}
                    />
                    <div className="agent-discover-main">
                      <div className="agent-discover-title">
                        <strong>{item.name}</strong>
                        <span className={`badge inline${item.confidence === 'high' ? ' ok' : ''}`}>
                          {item.confidence === 'high' ? '高置信' : '低置信'}
                        </span>
                        {item.already_added && <span className="badge inline ok">已添加</span>}
                      </div>
                      <span className="hint compact">
                        {[item.vendor, item.protocol, item.launch_command].filter(Boolean).join(' · ')}
                      </span>
                      <span className="hint compact agent-discover-hits">
                        {item.hit_reasons.join('；')}
                      </span>
                      {item.cli_path && (
                        <code className="takeover-env-path">CLI {item.cli_path}</code>
                      )}
                      {item.settings_path && (
                        <code className="takeover-env-path">配置 {item.settings_path}</code>
                      )}
                      {item.warning && <p className="agent-discover-warn">{item.warning}</p>}
                      {item.requires_path && !item.already_added && (
                        <input
                          className="agent-discover-path"
                          placeholder="补充配置目录或 skills 目录，如 ~/.cursor"
                          value={state.extraPath}
                          disabled={adding || busy}
                          onChange={(e) => setExtraPath(item.id, e.target.value)}
                          onClick={(e) => e.stopPropagation()}
                        />
                      )}
                    </div>
                  </label>
                </div>
              );
            })}
          </div>
        </div>
        <div className="modal-actions takeover-modal-actions">
          <span className="hint compact">
            {selectable.length > 0 ? `可选 ${selectable.length}` : ''}
            {selectedCount > 0 ? ` · 已选 ${selectedCount}` : ''}
          </span>
          <button type="button" className="ghost" disabled={adding} onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="primary"
            disabled={adding || busy || selectedCount === 0}
            onClick={() => void addSelected()}
          >
            {adding ? '添加中…' : `添加所选${selectedCount > 0 ? ` (${selectedCount})` : ''}`}
          </button>
        </div>
      </div>
    </div>
  );
}

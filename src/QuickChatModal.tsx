import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Copy, Eraser, Loader2, Send, X } from 'lucide-react';
import { t } from './i18n';

type ChatRole = 'user' | 'assistant' | 'system';

type ChatTurn = {
  id: string;
  role: ChatRole;
  content: string;
  created_at: number;
  latency_ms?: number | null;
  input_tokens?: number | null;
  output_tokens?: number | null;
  tokens_per_sec?: number | null;
  tokens_estimated?: boolean;
  provider_id?: string | null;
  provider_name?: string | null;
  model_name?: string | null;
};

type ChatHistory = {
  provider_id: string;
  messages: ChatTurn[];
};

type ChatReply = {
  content: string;
  latency_ms: number;
  input_tokens: number;
  output_tokens: number;
  tokens_per_sec: number;
  tokens_estimated: boolean;
  provider_id: string;
  provider_name: string;
  model_name: string;
};

type ProviderOption = {
  id: string;
  name: string;
  model_name: string;
  enabled: boolean;
  base_url: string;
  api_key: string;
  api_key_masked: string;
  protocol: 'openai' | 'anthropic' | 'open_ai';
  provider: string;
};

type Props = {
  open: boolean;
  onClose: () => void;
  providers: ProviderOption[];
  activeProviderId: string;
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void;
};

function newId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function formatTime(ms: number): string {
  try {
    return new Date(ms).toLocaleString(undefined, {
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  } catch {
    return '';
  }
}

function formatLatency(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${ms}ms`;
}

function formatTps(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '—';
  return n >= 100 ? n.toFixed(0) : n.toFixed(1);
}

function roleLabel(role: ChatRole): string {
  if (role === 'user') return t('chat.roleUser');
  if (role === 'assistant') return t('chat.roleAssistant');
  return t('chat.roleSystem');
}

export function QuickChatModal({ open, onClose, providers, activeProviderId, pushToast }: Props) {
  const [chatInput, setChatInput] = useState('');
  const [chatBusy, setChatBusy] = useState(false);
  const [loadingHistory, setLoadingHistory] = useState(false);
  const [messages, setMessages] = useState<ChatTurn[]>([]);
  const [providerId, setProviderId] = useState('');
  const [confirmClear, setConfirmClear] = useState(false);
  const chatEndRef = useRef<HTMLDivElement | null>(null);
  const persistTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hydrated = useRef(false);

  const enabledProviders = providers.filter((p) => p.enabled);
  const selected = enabledProviders.find((p) => p.id === providerId);

  // 打开时加载持久化会话（不清空）
  useEffect(() => {
    if (!open) {
      hydrated.current = false;
      setConfirmClear(false);
      return;
    }
    let cancelled = false;
    setLoadingHistory(true);
    void (async () => {
      try {
        const hist = await invoke<ChatHistory>('get_quick_chat_history');
        if (cancelled) return;
        setMessages(Array.isArray(hist.messages) ? hist.messages : []);
        const preferred =
          (hist.provider_id && providers.some((p) => p.id === hist.provider_id && p.enabled)
            ? hist.provider_id
            : '') ||
          activeProviderId ||
          providers.find((p) => p.enabled)?.id ||
          '';
        setProviderId(preferred);
        hydrated.current = true;
      } catch {
        if (cancelled) return;
        const preferred = activeProviderId || providers.find((p) => p.enabled)?.id || '';
        setProviderId(preferred);
        hydrated.current = true;
      } finally {
        if (!cancelled) setLoadingHistory(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, activeProviderId, providers]);

  // 自动滚动到底部
  useEffect(() => {
    if (open) {
      chatEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [messages, open, chatBusy]);

  // ESC 关闭
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (confirmClear) setConfirmClear(false);
        else onClose();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose, confirmClear]);

  // 防抖落盘（仅 hydrated 后）
  useEffect(() => {
    if (!open || !hydrated.current) return;
    if (persistTimer.current) clearTimeout(persistTimer.current);
    persistTimer.current = setTimeout(() => {
      void invoke('save_quick_chat_history', {
        history: { provider_id: providerId, messages },
      }).catch(() => undefined);
    }, 280);
    return () => {
      if (persistTimer.current) clearTimeout(persistTimer.current);
    };
  }, [messages, providerId, open]);

  const persistNow = async (next: ChatTurn[], nextProviderId: string) => {
    try {
      await invoke('save_quick_chat_history', {
        history: { provider_id: nextProviderId, messages: next },
      });
    } catch {
      /* ignore */
    }
  };

  const sendChat = async () => {
    const text = chatInput.trim();
    if (!text || chatBusy) return;
    if (!providerId) {
      pushToast(t('chat.selectModelFirst'), 'error');
      return;
    }
    const userTurn: ChatTurn = {
      id: newId(),
      role: 'user',
      content: text,
      created_at: Date.now(),
    };
    const prior = messages
      .filter((m) => m.role === 'user' || m.role === 'assistant')
      .map((m) => ({ role: m.role, content: m.content }));

    setChatInput('');
    const withUser = [...messages, userTurn];
    setMessages(withUser);
    setChatBusy(true);
    try {
      const reply = await invoke<ChatReply>('quick_gateway_chat', {
        message: text,
        providerId,
        history: prior,
      });
      const assistantTurn: ChatTurn = {
        id: newId(),
        role: 'assistant',
        content: reply.content,
        created_at: Date.now(),
        latency_ms: reply.latency_ms,
        input_tokens: reply.input_tokens,
        output_tokens: reply.output_tokens,
        tokens_per_sec: reply.tokens_per_sec,
        tokens_estimated: reply.tokens_estimated,
        provider_id: reply.provider_id,
        provider_name: reply.provider_name,
        model_name: reply.model_name,
      };
      const next = [...withUser, assistantTurn];
      setMessages(next);
      await persistNow(next, providerId);
    } catch (e) {
      const err = String(e);
      const systemTurn: ChatTurn = {
        id: newId(),
        role: 'system',
        content: err,
        created_at: Date.now(),
        provider_id: providerId,
        provider_name: selected?.name,
        model_name: selected?.model_name,
      };
      const next = [...withUser, systemTurn];
      setMessages(next);
      await persistNow(next, providerId);
      pushToast(err, 'error');
    } finally {
      setChatBusy(false);
    }
  };

  const clearChat = async () => {
    try {
      await invoke('clear_quick_chat_history');
      setMessages([]);
      setConfirmClear(false);
      pushToast(t('chat.cleared'), 'ok');
    } catch (e) {
      pushToast(String(e), 'error');
    }
  };

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      pushToast(t('chat.copied'), 'ok');
    } catch {
      pushToast(t('chat.copyFailed'), 'error');
    }
  };

  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal modal-compact quick-chat-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <div className="quick-chat-head-text">
            <h3>{t('chat.title')}</h3>
            <span className="hint compact">
              {loadingHistory
                ? t('chat.loading')
                : messages.length === 0
                  ? t('chat.hint')
                  : t('chat.msgCount', { n: messages.length })}
            </span>
          </div>
          <div className="quick-chat-head-actions">
            {messages.length > 0 && (
              <button
                type="button"
                className="tiny ghost"
                disabled={chatBusy}
                onClick={() => setConfirmClear(true)}
                title={t('chat.clear')}
              >
                <Eraser size={14} />
                {t('chat.clear')}
              </button>
            )}
            <button type="button" className="tiny icon-only" onClick={onClose} aria-label={t('common.close')}>
              <X size={14} />
            </button>
          </div>
        </div>

        <div className="modal-body">
          <div className="quick-chat-body">
            <div className="quick-chat-toolbar">
              <label className="quick-chat-model">
                <span className="hint compact">{t('chat.model')}</span>
                <select
                  className="app-select"
                  value={providerId}
                  disabled={chatBusy || enabledProviders.length === 0}
                  onChange={(e) => setProviderId(e.target.value)}
                >
                  {enabledProviders.length === 0 && <option value="">{t('chat.noModels')}</option>}
                  {enabledProviders.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name} · {p.model_name}
                    </option>
                  ))}
                </select>
              </label>
            </div>

            <div className="quick-chat-messages" aria-live="polite">
              {!loadingHistory && messages.length === 0 && (
                <div className="quick-chat-empty">{t('chat.hint')}</div>
              )}
              {messages.map((msg) => (
                <div key={msg.id} className={`quick-chat-bubble ${msg.role}`}>
                  <div className="quick-chat-meta">
                    <span className="quick-chat-role">{roleLabel(msg.role)}</span>
                    {(msg.provider_name || msg.model_name) && (
                      <span className="quick-chat-provider" title={[msg.provider_name, msg.model_name].filter(Boolean).join(' · ')}>
                        {[msg.provider_name, msg.model_name].filter(Boolean).join(' · ')}
                      </span>
                    )}
                    <span className="quick-chat-time">{formatTime(msg.created_at)}</span>
                  </div>
                  <div className="quick-chat-text">{msg.content}</div>
                  <div className="quick-chat-foot">
                    {msg.role === 'assistant' && (
                      <span className="quick-chat-stats">
                        {msg.latency_ms != null && (
                          <span>{t('chat.latency', { v: formatLatency(msg.latency_ms) })}</span>
                        )}
                        {msg.output_tokens != null && msg.output_tokens > 0 && (
                          <span>
                            {msg.tokens_estimated ? '~' : ''}
                            {t('chat.tokens', {
                              in: msg.input_tokens ?? 0,
                              out: msg.output_tokens,
                            })}
                          </span>
                        )}
                        {msg.tokens_per_sec != null && msg.tokens_per_sec > 0 && (
                          <span>
                            {msg.tokens_estimated ? '~' : ''}
                            {t('chat.tps', { v: formatTps(msg.tokens_per_sec) })}
                          </span>
                        )}
                      </span>
                    )}
                    <button
                      type="button"
                      className="tiny icon-only ghost"
                      onClick={() => void copyText(msg.content)}
                      title={t('chat.copy')}
                      aria-label={t('chat.copy')}
                    >
                      <Copy size={12} />
                    </button>
                  </div>
                </div>
              ))}
              {chatBusy && (
                <div className="quick-chat-bubble assistant pending">
                  <div className="quick-chat-meta">
                    <span className="quick-chat-role">{roleLabel('assistant')}</span>
                    {selected && (
                      <span className="quick-chat-provider">
                        {selected.name} · {selected.model_name}
                      </span>
                    )}
                  </div>
                  <div className="quick-chat-text quick-chat-typing">
                    <Loader2 size={14} className="spin" />
                    {t('chat.thinking')}
                  </div>
                </div>
              )}
              <div ref={chatEndRef} />
            </div>

            <div className="quick-chat-input-row">
              <textarea
                className="app-input quick-chat-input"
                value={chatInput}
                rows={2}
                placeholder={t('chat.placeholder')}
                disabled={chatBusy || enabledProviders.length === 0}
                onChange={(e) => setChatInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    void sendChat();
                  }
                }}
              />
              <button
                type="button"
                className="button primary quick-chat-send"
                disabled={chatBusy || !chatInput.trim() || enabledProviders.length === 0}
                onClick={() => void sendChat()}
              >
                {chatBusy ? <Loader2 size={14} className="spin" /> : <Send size={14} />}
                {t('chat.send')}
              </button>
            </div>
          </div>
        </div>

        {confirmClear && (
          <div className="quick-chat-confirm" onClick={(e) => e.stopPropagation()}>
            <div className="quick-chat-confirm-card">
              <p>{t('chat.clearConfirm')}</p>
              <div className="quick-chat-confirm-actions">
                <button type="button" className="button ghost" onClick={() => setConfirmClear(false)}>
                  {t('common.cancel')}
                </button>
                <button type="button" className="danger" onClick={() => void clearChat()}>
                  {t('chat.clear')}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

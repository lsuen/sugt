import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { X } from 'lucide-react';
import { t } from './i18n';

type ChatMsg = { role: 'user' | 'assistant' | 'system'; content: string };

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

export function QuickChatModal({ open, onClose, providers, activeProviderId, pushToast }: Props) {
  const [chatInput, setChatInput] = useState('');
  const [chatBusy, setChatBusy] = useState(false);
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [providerId, setProviderId] = useState('');
  const chatEndRef = useRef<HTMLDivElement | null>(null);

  const enabledProviders = providers.filter((p) => p.enabled);

  // 打开时同步 provider，清空消息
  useEffect(() => {
    if (!open) return;
    setMessages([]);
    setChatInput('');
    const preferred = activeProviderId || providers.find((p) => p.enabled)?.id || '';
    setProviderId((prev) => {
      if (prev && providers.some((p) => p.id === prev && p.enabled)) return prev;
      return preferred;
    });
  }, [open, activeProviderId, providers]);

  // 自动滚动到底部
  useEffect(() => {
    if (open) {
      chatEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [messages, open]);

  // ESC 关闭
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  const sendChat = async () => {
    const text = chatInput.trim();
    if (!text || chatBusy) return;
    if (!providerId) {
      pushToast(t('chat.selectModelFirst'), 'error');
      return;
    }
    setChatInput('');
    setMessages((prev) => [...prev, { role: 'user', content: text }]);
    setChatBusy(true);
    try {
      const reply = await invoke<string>('quick_gateway_chat', {
        message: text,
        providerId,
      });
      setMessages((prev) => [...prev, { role: 'assistant', content: reply }]);
    } catch (e) {
      const err = String(e);
      setMessages((prev) => [...prev, { role: 'system', content: err }]);
      pushToast(err, 'error');
    } finally {
      setChatBusy(false);
    }
  };

  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal modal-compact quick-chat-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{t('chat.title')}</h3>
          <button type="button" className="tiny icon-only" onClick={onClose}>
            <X size={14} />
          </button>
        </div>
        <div className="modal-body">
          <div className="quick-chat-body">
            <div className="quick-chat-model">
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
            </div>
            <div className="quick-chat-messages">
              {messages.length === 0 && (
                <div style={{ color: '#6b7d8f', fontSize: 12, textAlign: 'center', padding: '20px 0' }}>
                  {t('chat.hint')}
                </div>
              )}
              {messages.map((msg, i) => (
                <div key={i} className={`quick-chat-bubble ${msg.role}`}>
                  <div className="quick-chat-role">{msg.role}</div>
                  <div className="quick-chat-text">{msg.content}</div>
                </div>
              ))}
              <div ref={chatEndRef} />
            </div>
            <div className="quick-chat-input-row">
              <input
                className="app-input"
                value={chatInput}
                placeholder={t('chat.placeholder')}
                disabled={chatBusy}
                onChange={(e) => setChatInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void sendChat(); } }}
              />
              <button type="button" className="button primary" disabled={chatBusy || !chatInput.trim()} onClick={() => void sendChat()}>
                {t('chat.send')}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

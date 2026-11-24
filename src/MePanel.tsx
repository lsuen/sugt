import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { invoke, listen } from './transport';
import {
  Bot,
  Check,
  ChevronDown,
  Copy,
  Cpu,
  Eye,
  FolderOpen,
  ImagePlus,
  type LucideIcon,
  PawPrint,
  Puzzle,
  RotateCcw,
  ScrollText,
  Send,
  Settings,
  Square,
  Trash2,
} from 'lucide-react';
import { ModalPortal } from './ModalPortal';
import { lang, t, type TKey } from './i18n';
import { MeMarkdown } from './meMarkdown';
import { clipPreview, parsePendingFiles } from './mePendingFiles';
import { loadMeAuthMode, saveMeAuthMode, type MeAuthMode, ME_AUTH_MODES } from './meAuth';

const PLACE_HISTORY_KEY = 'sugt_me_place_history';
const PLACE_HISTORY_LIMIT = 8;

type PlaceHistoryItem = {
  mode: string;
  project: string;
  path: string;
};

function pathLeaf(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/').filter(Boolean);
  return parts[parts.length - 1] || path || '…';
}

function shortenPath(path: string, head = 18, tail = 28): string {
  const normalized = path.replace(/\\/g, '/');
  if (normalized.length <= head + tail + 1) return normalized;
  return `${normalized.slice(0, head)}…${normalized.slice(-tail)}`;
}

function loadPlaceHistory(): PlaceHistoryItem[] {
  try {
    const raw = localStorage.getItem(PLACE_HISTORY_KEY);
    if (!raw) return [];
    const list = JSON.parse(raw) as PlaceHistoryItem[];
    return Array.isArray(list) ? list.filter((item) => item && item.path && item.mode) : [];
  } catch {
    return [];
  }
}

function rememberPlace(item: PlaceHistoryItem): PlaceHistoryItem[] {
  const next = [
    item,
    ...loadPlaceHistory().filter(
      (entry) => !(entry.mode === item.mode && entry.project === item.project && entry.path === item.path),
    ),
  ].slice(0, PLACE_HISTORY_LIMIT);
  localStorage.setItem(PLACE_HISTORY_KEY, JSON.stringify(next));
  return next;
}

export type MeStep = {
  command: string;
  output: string;
  exit_code: number | null;
  truncated: boolean;
  timed_out: boolean;
  note: string;
};

export type MeImage = {
  media_type: string;
  data: string;
};

export type MeMessage = {
  id: string;
  role: string;
  content: string;
  images?: MeImage[];
  steps: MeStep[];
  created_at: number;
};

const IMAGE_MAX = 3;
const IMAGE_BYTES_MAX = 2 * 1024 * 1024;
const INPUT_MAX_HEIGHT = 200;
const ME_IDEA_KEYS: TKey[] = [
  'me.suggestGateway',
  'me.suggestErrors',
  'me.suggestList',
  'me.suggestProject',
  'me.suggestLookup',
];
/** 离底部不到这个距离才跟随新消息滚动；用户往上翻历史时不打断阅读。 */
const STICK_THRESHOLD_PX = 80;

function imageSrc(image: MeImage): string {
  return `data:${image.media_type};base64,${image.data}`;
}

function approxImageBytes(data: string): number {
  return Math.floor(data.replace(/\s/g, '').length * 3 / 4);
}

async function fileToMeImage(file: File): Promise<MeImage> {
  const media_type = (file.type || 'image/png').toLowerCase();
  if (!/^image\/(png|jpe?g|webp|gif)$/.test(media_type)) {
    throw new Error(t('me.imageType'));
  }
  if (file.size > IMAGE_BYTES_MAX) {
    throw new Error(t('me.imageTooBig'));
  }
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result || ''));
    reader.onerror = () => reject(new Error(t('me.imageReadFail')));
    reader.readAsDataURL(file);
  });
  const comma = dataUrl.indexOf(',');
  const data = comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
  if (!data || approxImageBytes(data) > IMAGE_BYTES_MAX) {
    throw new Error(t('me.imageTooBig'));
  }
  return {
    media_type: media_type === 'image/jpg' ? 'image/jpeg' : media_type,
    data,
  };
}

export type MePending = {
  command: string;
  prose: string;
};

export type MeView = {
  mode: string;
  project: string;
  projects: string[];
  work_dir: string;
  projects_root: string;
  config_dir: string;
  messages: MeMessage[];
  pending: MePending | null;
};

export type MeEvent = {
  kind: string;
  text: string;
  command: string;
  output: string;
  exit_code: number | null;
  timed_out: boolean;
};

export type MeApi = {
  load: () => Promise<MeView>;
  setPlace: (mode: string, project?: string) => Promise<MeView>;
  createProject: (name: string) => Promise<MeView>;
  send: (message: string, images?: MeImage[]) => Promise<MeView>;
  confirm: () => Promise<MeView>;
  dismiss: () => Promise<MeView>;
  stop: () => Promise<void>;
  clear: () => Promise<MeView>;
  truncateLastTurn: () => Promise<MeView>;
  openDir: () => Promise<void>;
  subscribe: (onStep: (event: MeEvent) => void) => Promise<() => void>;
};

function errorText(err: unknown): string {
  if (err instanceof Error && err.message) return err.message;
  return String(err);
}

export function tauriMeApi(): MeApi {
  const language = () => lang();
  const authMode = () => loadMeAuthMode();
  return {
    load: () => invoke<MeView>('me_status'),
    setPlace: (mode, project) => invoke<MeView>('me_set_place', { mode, project: project ?? null, lang: language() }),
    createProject: (name) => invoke<MeView>('me_create_project', { name, lang: language() }),
    send: (message, images) => invoke<MeView>('me_send', {
      message,
      images: images && images.length > 0 ? images : null,
      lang: language(),
      authMode: authMode(),
      autoWrite: authMode() !== 'ask',
    }),
    confirm: () => invoke<MeView>('me_confirm', {
      lang: language(),
      authMode: authMode(),
      autoWrite: authMode() !== 'ask',
    }),
    dismiss: () => invoke<MeView>('me_dismiss', { lang: language() }),
    stop: () => invoke('me_stop'),
    clear: () => invoke<MeView>('me_clear'),
    truncateLastTurn: () => invoke<MeView>('me_truncate_last_turn'),
    openDir: () => invoke('me_open_dir'),
    subscribe: async (onStep) => {
      const unlisten = await listen<MeEvent>('sugt://me-step', (event) => onStep(event.payload));
      return unlisten;
    },
  };
}

/** SOUL 只读编排层允许的意图；写操作仍由用户点按钮走既有确定性命令。 */
export type SoulRoute = 'clients' | 'skills' | 'preview' | 'models' | 'logs' | 'settings' | 'pet';

type Props = {
  api?: MeApi;
  onOpenClients: () => void;
  onOpenModels?: () => void;
  onOpenSkills?: () => void;
  onRoute?: (route: SoulRoute) => void;
};

function looksLikeStepBudgetStop(content: string): boolean {
  return content.includes('这一轮的步数用完了')
    || content.includes('used up its steps')
    || content.includes('步数用完');
}

function looksLikeContinueable(content: string): boolean {
  const mark = t('me.stoppedMark');
  return (mark.length > 0 && content.includes(mark))
    || content.includes('好，我先停下')
    || content.includes("I'll stop")
    || looksLikeStepBudgetStop(content);
}

function summarizeRunSteps(steps: MeStep[]): { ok: boolean; text: string } | null {
  if (steps.length === 0) return null;
  const failed = steps.find((step) => step.timed_out || (step.exit_code !== null && step.exit_code !== 0));
  if (failed?.timed_out) {
    return { ok: false, text: t('me.composerDoneTimeout', { count: steps.length }) };
  }
  if (failed) {
    return {
      ok: false,
      text: t('me.composerDoneFail', { count: steps.length, code: failed.exit_code ?? '?' }),
    };
  }
  return { ok: true, text: t('me.composerDoneOk', { count: steps.length }) };
}

export function MePanel({ api, onOpenClients, onOpenModels, onOpenSkills, onRoute }: Props) {
  const client = useMemo(() => api ?? tauriMeApi(), [api]);
  const [session, setSession] = useState<MeView | null>(null);
  const [draft, setDraft] = useState('');
  const [draftImages, setDraftImages] = useState<MeImage[]>([]);
  const [naming, setNaming] = useState(false);
  const [projectName, setProjectName] = useState('');
  const [busy, setBusy] = useState(false);
  const [awaiting, setAwaiting] = useState(false);
  const [live, setLive] = useState('');
  const [liveSteps, setLiveSteps] = useState<MeStep[]>([]);
  const [error, setError] = useState('');
  const [clearArmed, setClearArmed] = useState(false);
  const [history, setHistory] = useState<PlaceHistoryItem[]>(() => loadPlaceHistory());
  const [placeOpen, setPlaceOpen] = useState(false);
  const [authMode, setAuthMode] = useState<MeAuthMode>(() => loadMeAuthMode());
  const [liveDraft, setLiveDraft] = useState('');
  const [modelLabel, setModelLabel] = useState('');
  const [gatewayOn, setGatewayOn] = useState<boolean | null>(null);
  const [runFlash, setRunFlash] = useState<{ ok: boolean; text: string } | null>(null);
  const logRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const flashTimerRef = useRef<number | null>(null);
  const stickToBottomRef = useRef(true);
  const liveDraftRef = useRef('');
  const liveStepsRef = useRef<MeStep[]>([]);
  const runPromiseRef = useRef<Promise<boolean> | null>(null);
  const awaitingRef = useRef(false);
  const busyRef = useRef(false);

  useEffect(() => {
    liveDraftRef.current = liveDraft;
  }, [liveDraft]);
  useEffect(() => {
    liveStepsRef.current = liveSteps;
  }, [liveSteps]);
  useEffect(() => {
    awaitingRef.current = awaiting;
  }, [awaiting]);
  useEffect(() => {
    busyRef.current = busy;
  }, [busy]);

  const refreshStatus = () => {
    void invoke<{
      running?: boolean;
      active_model?: string | null;
      public_model_id?: string | null;
      active_provider?: string | null;
    }>('get_status')
      .then((status) => {
        const label =
          (status.active_model && status.active_model.trim())
          || (status.public_model_id && status.public_model_id.trim())
          || (status.active_provider && status.active_provider.trim())
          || '';
        setModelLabel(label);
        setGatewayOn(typeof status.running === 'boolean' ? status.running : null);
      })
      .catch(() => {
        setModelLabel('');
        setGatewayOn(null);
      });
  };

  const showRunFlash = (flash: { ok: boolean; text: string } | null) => {
    if (flashTimerRef.current != null) {
      window.clearTimeout(flashTimerRef.current);
      flashTimerRef.current = null;
    }
    setRunFlash(flash);
    if (!flash) return;
    flashTimerRef.current = window.setTimeout(() => {
      setRunFlash(null);
      flashTimerRef.current = null;
    }, 2800);
  };

  useEffect(() => {
    let stop = false;
    let unlisten: (() => void) | undefined;
    refreshStatus();
    client.load().then((view) => {
      if (stop) return;
      setSession(view);
      setHistory(rememberPlace({
        mode: view.mode,
        project: view.project,
        path: view.work_dir,
      }));
    }).catch((err: unknown) => {
      if (!stop) setError(errorText(err));
    });
    client.subscribe((event) => {
      if (event.kind === 'thinking') {
        setLiveDraft('');
        setLive(event.text || t('me.thinking'));
      }
      if (event.kind === 'delta') {
        setLive('');
        setLiveDraft(event.text || '');
      }
      if (event.kind === 'running') {
        setLiveDraft('');
        setLive(event.command || event.text || t('me.working'));
      }
      if (event.kind === 'ran') {
        setLiveDraft('');
        setLive(event.command || t('me.working'));
        setLiveSteps((prev) => [...prev, {
          command: event.command,
          output: event.output,
          exit_code: event.exit_code,
          truncated: false,
          timed_out: event.timed_out,
          note: event.text,
        }]);
      }
    }).then((fn) => {
      if (stop) fn();
      else unlisten = fn;
    }).catch(() => undefined);
    return () => {
      stop = true;
      unlisten?.();
      if (flashTimerRef.current != null) window.clearTimeout(flashTimerRef.current);
    };
  }, [client]);

  useEffect(() => {
    const node = logRef.current;
    if (!node || !stickToBottomRef.current) return;
    node.scrollTop = node.scrollHeight;
  }, [session, live, liveDraft, liveSteps, awaiting]);

  useLayoutEffect(() => {
    const node = inputRef.current;
    if (!node) return;
    node.style.height = 'auto';
    node.style.height = `${Math.min(node.scrollHeight, INPUT_MAX_HEIGHT)}px`;
  }, [draft]);

  useEffect(() => {
    if (!clearArmed) return;
    const timer = window.setTimeout(() => setClearArmed(false), 8000);
    return () => window.clearTimeout(timer);
  }, [clearArmed]);

  useEffect(() => {
    if (!placeOpen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setPlaceOpen(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [placeOpen]);

  async function run(
    task: () => Promise<MeView>,
    waitForSu = false,
    opts?: { rollbackOnFail?: () => void },
  ) {
    const work = (async (): Promise<boolean> => {
      setError('');
      setBusy(true);
      busyRef.current = true;
      setAwaiting(waitForSu);
      awaitingRef.current = waitForSu;
      if (waitForSu) {
        setLive(t('me.thinking'));
        setLiveDraft('');
        setLiveSteps([]);
        showRunFlash(null);
      }
      let ok = false;
      let finished: MeView | null = null;
      try {
        const view = await task();
        finished = view;
        setSession(view);
        setHistory(rememberPlace({
          mode: view.mode,
          project: view.project,
          path: view.work_dir,
        }));
        ok = true;
      } catch (err) {
        setError(errorText(err));
        if (opts?.rollbackOnFail) {
          opts.rollbackOnFail();
          setError((prev) => {
            const hint = t('me.sendFailRestore');
            return prev ? `${prev} · ${hint}` : hint;
          });
        } else {
          try {
            setSession(await client.load());
          } catch {
            /* 保持当前画面 */
          }
        }
      } finally {
        if (waitForSu && ok && finished) {
          const lastAssistant = [...finished.messages].reverse().find((message) => message.role === 'assistant');
          const flash = summarizeRunSteps(lastAssistant?.steps ?? []);
          if (flash) showRunFlash(flash);
        }
        setBusy(false);
        busyRef.current = false;
        setAwaiting(false);
        awaitingRef.current = false;
        setLive('');
        setLiveDraft('');
        setLiveSteps([]);
        refreshStatus();
        inputRef.current?.focus();
      }
      return ok;
    })();
    runPromiseRef.current = work;
    try {
      return await work;
    } finally {
      if (runPromiseRef.current === work) runPromiseRef.current = null;
    }
  }

  async function addImageFiles(files: File[]) {
    if (files.length === 0) return;
    const room = IMAGE_MAX - draftImages.length;
    if (room <= 0) {
      setError(t('me.imagesMax', { count: IMAGE_MAX }));
      return;
    }
    const take = files.slice(0, room);
    try {
      const next = await Promise.all(take.map((file) => fileToMeImage(file)));
      setDraftImages((prev) => [...prev, ...next].slice(0, IMAGE_MAX));
      setError('');
      if (files.length > room) setError(t('me.imagesMax', { count: IMAGE_MAX }));
    } catch (err) {
      setError(errorText(err));
    }
  }

  function appendStoppedOptimistic(content: string, steps: MeStep[]) {
    const mark = t('me.stoppedMark');
    const body = content.trimEnd();
    const withMark = mark && !body.includes(mark) ? `${body}\n\n${mark}` : body;
    const optimistic: MeMessage = {
      id: `local-stop-${Date.now()}`,
      role: 'assistant',
      content: withMark,
      steps,
      created_at: Date.now(),
    };
    setSession((prev) => (prev ? { ...prev, messages: [...prev.messages, optimistic] } : prev));
    // 乐观气泡已上屏：关掉流式气泡，但保留 liveDraft 直到 me_send 返回。
    setAwaiting(false);
    awaitingRef.current = false;
    setLive('');
  }

  async function handleStop() {
    if (!awaitingRef.current) return;
    const snapDraft = liveDraftRef.current;
    const snapSteps = liveStepsRef.current;
    void client.stop().catch((err: unknown) => setError(errorText(err)));
    if (snapDraft.trim()) {
      appendStoppedOptimistic(snapDraft, snapSteps);
    }
  }

  async function stopThenSend(text: string, images = draftImages) {
    const message = text.trim();
    if (!message && images.length === 0) return;
    const savedImages = [...images];
    await handleStop();
    const pending = runPromiseRef.current;
    if (pending) {
      try {
        await pending;
      } catch {
        /* 上一轮已在 run 内消化 */
      }
    }
    await send(message, savedImages);
  }

  async function send(text: string, images = draftImages) {
    const message = text.trim();
    if ((!message && images.length === 0) || busyRef.current) return;
    // 先把用户气泡打进面板，再等网关推理；否则整段等待期间输入像「消失了」。
    const optimistic: MeMessage = {
      id: `local-${Date.now()}`,
      role: 'user',
      content: message,
      images: images.length > 0 ? images : undefined,
      steps: [],
      created_at: Date.now(),
    };
    const savedImages = [...images];
    setSession((prev) => (prev ? { ...prev, messages: [...prev.messages, optimistic] } : prev));
    setDraft('');
    setDraftImages([]);
    stickToBottomRef.current = true;
    await run(() => client.send(message, images), true, {
      rollbackOnFail: () => {
        setSession((prev) => (prev
          ? { ...prev, messages: prev.messages.filter((item) => !item.id.startsWith('local-')) }
          : prev));
        setDraft(message);
        setDraftImages(savedImages);
      },
    });
  }

  function regenerateLast() {
    if (!session || busyRef.current || awaitingRef.current) return;
    const messages = session.messages;
    let lastAssistant = -1;
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      if (messages[i].role === 'assistant') {
        lastAssistant = i;
        break;
      }
    }
    if (lastAssistant < 0) return;
    let prevUser = -1;
    for (let i = lastAssistant - 1; i >= 0; i -= 1) {
      if (messages[i].role === 'user') {
        prevUser = i;
        break;
      }
    }
    if (prevUser < 0) return;
    const user = messages[prevUser];
    void send(user.content, user.images ?? []);
  }

  function deleteLastTurn() {
    if (!session || busyRef.current || awaitingRef.current) return;
    void run(() => client.truncateLastTurn());
  }

  const projects = session
    ? (session.projects.includes(session.project) ? session.projects : [session.project, ...session.projects])
    : [];

  const leaf = session ? pathLeaf(session.work_dir) : '…';
  const placeLabel = session
    ? (session.mode === 'config' ? t('me.placeConfig') : leaf)
    : '';
  const modelShort = modelLabel
    ? (modelLabel.length > 28 ? `${modelLabel.slice(0, 27)}…` : modelLabel)
    : '';
  const statusHint = runFlash
    ? runFlash.text
    : awaiting
      ? t('me.composerRunning', { count: Math.max(liveSteps.length, 1) })
      : session?.pending
        ? t('me.composerPending')
        : t('me.composerHint');
  const recent = history.filter((item) => !(
    session
    && item.mode === session.mode
    && item.project === session.project
    && item.path === session.work_dir
  ));

  return (
    <div className="me-panel">
      <div className="me-desk">
        <div className="me-seg" role="group" aria-label={t('me.placeProject')}>
          <button
            type="button"
            className={session?.mode !== 'config' ? 'tiny me-seg-btn active' : 'tiny me-seg-btn'}
            disabled={busy || !session}
            onClick={() => {
              if (session && session.mode === 'config') void run(() => client.setPlace('project', session.project));
            }}
          >{t('me.placeProject')}</button>
          <button
            type="button"
            className={session?.mode === 'config' ? 'tiny me-seg-btn active' : 'tiny me-seg-btn'}
            disabled={busy || !session}
            onClick={() => {
              if (session && session.mode !== 'config') void run(() => client.setPlace('config'));
            }}
          >{t('me.placeConfig')}</button>
        </div>

        <div className="me-place">
          <button
            type="button"
            className="tiny me-place-btn"
            disabled={busy || !session}
            aria-expanded={placeOpen}
            aria-haspopup="listbox"
            aria-label={t('me.placeMenu')}
            title={session?.work_dir}
            onClick={() => setPlaceOpen((open) => !open)}
          >
            <FolderOpen size={14} />
            <span>{leaf}</span>
            <ChevronDown size={14} />
          </button>
          {placeOpen && session && (
            <ModalPortal>
              <div
                className="modal-overlay"
                onMouseDown={(event) => {
                  if (event.target === event.currentTarget) setPlaceOpen(false);
                }}
              >
                <div className="modal me-place-modal" role="dialog" aria-modal="true" aria-label={t('me.placeMenu')}>
                  <div className="modal-head">
                    <h3>{t('me.placeMenu')}</h3>
                    <button type="button" className="icon-btn" onClick={() => setPlaceOpen(false)} aria-label={t('common.close')}>×</button>
                  </div>
                  <div className="modal-body">
                    <div className="me-place-menu" role="listbox">
              <div className="me-place-current" title={session.work_dir}>
                <span>{leaf}</span>
                <code>{shortenPath(session.work_dir)}</code>
              </div>
              {recent.length > 0 && (
                <>
                  <div className="me-place-label">{t('me.placeRecent')}</div>
                  {recent.map((item) => (
                    <button
                      key={`${item.mode}:${item.project}:${item.path}`}
                      type="button"
                      className="me-place-item"
                      title={item.path}
                      role="option"
                      onClick={() => {
                        setPlaceOpen(false);
                        void run(() => client.setPlace(item.mode, item.project));
                      }}
                    >
                      <span>{pathLeaf(item.path)}</span>
                      <code>{shortenPath(item.path)}</code>
                    </button>
                  ))}
                </>
              )}
              {session.mode !== 'config' && projects.length > 0 && (
                <>
                  <div className="me-place-label">{t('me.placeProjects')}</div>
                  {projects.map((name) => (
                    <button
                      key={name}
                      type="button"
                      className={name === session.project ? 'me-place-item active' : 'me-place-item'}
                      title={`${session.projects_root}/${name}`}
                      role="option"
                      disabled={name === session.project}
                      onClick={() => {
                        setPlaceOpen(false);
                        void run(() => client.setPlace('project', name));
                      }}
                    >
                      <span>{name}</span>
                      <code>{shortenPath(`${session.projects_root}/${name}`)}</code>
                    </button>
                  ))}
                </>
              )}
              <button
                type="button"
                className={session.mode === 'config' ? 'me-place-item active' : 'me-place-item'}
                title={session.config_dir}
                role="option"
                disabled={session.mode === 'config'}
                onClick={() => {
                  setPlaceOpen(false);
                  void run(() => client.setPlace('config'));
                }}
              >
                <span>{pathLeaf(session.config_dir)}</span>
                <code>{shortenPath(session.config_dir)}</code>
              </button>
                    </div>
                  </div>
                  <div className="modal-actions">
                    <button type="button" className="ghost" onClick={() => setPlaceOpen(false)}>{t('common.close')}</button>
                  </div>
                </div>
              </div>
            </ModalPortal>
          )}
        </div>

        {session?.mode !== 'config' && (
          naming ? (
            <form
              className="me-new"
              onSubmit={(event) => {
                event.preventDefault();
                const name = projectName.trim();
                if (!name) return;
                void run(() => client.createProject(name)).then((ok) => {
                  if (!ok) return;
                  setProjectName('');
                  setNaming(false);
                });
              }}
            >
              <input
                className="me-name"
                value={projectName}
                placeholder={t('me.projectPh')}
                aria-label={t('me.projectPh')}
                onChange={(event) => setProjectName(event.target.value)}
                autoFocus
              />
              <button type="submit" className="tiny" disabled={busy || !projectName.trim()}>{t('me.create')}</button>
            </form>
          ) : (
            <button type="button" className="tiny" disabled={busy} onClick={() => setNaming(true)}>{t('me.newProject')}</button>
          )
        )}

        <div className="me-desk-actions">
          <button type="button" className="tiny" disabled={!session} onClick={() => client.openDir().catch((err: unknown) => setError(errorText(err)))}>
            <FolderOpen size={14} />{t('me.openDir')}
          </button>
          {onOpenSkills && (
            <button type="button" className="tiny" title={t('me.openSkillsHint')} onClick={onOpenSkills}>
              {t('me.openSkills')}
            </button>
          )}
          <button type="button" className="tiny" title={t('me.handoffHint')} onClick={onOpenClients}>{t('me.handoff')}</button>
        </div>
      </div>
      {session?.mode === 'config' && <p className="me-warn">{t('me.configWarn')}</p>}

      <div
        className="me-log"
        ref={logRef}
        role="log"
        aria-live="polite"
        onScroll={(event) => {
          const node = event.currentTarget;
          stickToBottomRef.current = node.scrollHeight - node.scrollTop - node.clientHeight < STICK_THRESHOLD_PX;
        }}
      >
        {session && session.messages.length === 0 && !awaiting && (
          <div className="me-hello">
            <div className="me-who">
              <span className="me-seal" aria-hidden>su</span>
              <strong>{t('me.name')}</strong>
            </div>
            <MeMarkdown text={t('me.greeting')} />
            <div className="me-ideas">
              {ME_IDEA_KEYS.map((key) => (
                <button key={key} type="button" className="me-idea" onClick={() => void send(t(key))}>{t(key)}</button>
              ))}
            </div>
          </div>
        )}
        {session?.messages.map((message, index) => {
          if (message.role === 'user') {
            const images = message.images ?? [];
            return (
              <div key={message.id} className="me-user">
                {images.length > 0 && (
                  <div className="me-user-images">
                    {images.map((image, imageIndex) => (
                      <img
                        key={`${message.id}-${imageIndex}`}
                        src={imageSrc(image)}
                        alt=""
                        className="me-thumb"
                      />
                    ))}
                  </div>
                )}
                {message.content}
              </div>
            );
          }
          const lastAssistant = !session.messages.slice(index + 1).some((item) => item.role === 'assistant');
          const pendingHere = Boolean(lastAssistant && session.pending && !awaiting);
          return (
            <SuBubble
              key={message.id}
              content={message.content}
              steps={message.steps ?? []}
              pending={pendingHere ? session.pending : null}
              busy={busy}
              isLastAssistant={lastAssistant && !awaiting}
              onRegenerate={lastAssistant && !awaiting ? () => regenerateLast() : undefined}
              onDeleteTurn={lastAssistant && !awaiting ? () => deleteLastTurn() : undefined}
              onConfirm={() => void run(() => client.confirm(), true)}
              onDismiss={() => void run(() => client.dismiss())}
              onRoute={onRoute}
            />
          );
        })}
        {awaiting && (
          <SuBubble
            content={liveDraft}
            steps={liveSteps}
            live={liveDraft ? (live || undefined) : (live || t('me.thinking'))}
            streaming={Boolean(liveDraft)}
            pending={null}
            busy={busy}
            onConfirm={() => undefined}
            onDismiss={() => undefined}
            onRoute={onRoute}
          />
        )}
        {session?.pending && !awaiting && !session.messages.some((message) => message.role === 'assistant') && (
          <SuBubble
            content=""
            steps={[]}
            pending={session.pending}
            busy={busy}
            onConfirm={() => void run(() => client.confirm(), true)}
            onDismiss={() => void run(() => client.dismiss())}
            onRoute={onRoute}
          />
        )}
      </div>

      {error && <p className="me-error" role="alert">{error}</p>}
      <p className="hint compact me-light-hint">{t('me.lightHint')}</p>

      <form
        className="me-composer"
        onSubmit={(event) => {
          event.preventDefault();
          if (awaiting && (draft.trim() || draftImages.length > 0)) {
            void stopThenSend(draft);
            return;
          }
          if (awaiting) {
            void handleStop();
            return;
          }
          void send(draft);
        }}
      >
        {draftImages.length > 0 && (
          <div className="me-draft-images" aria-label={t('me.pasteImages')}>
            {draftImages.map((image, index) => (
              <div key={`${image.media_type}-${index}`} className="me-draft-item">
                <img src={imageSrc(image)} alt="" className="me-thumb" />
                <button
                  type="button"
                  className="me-draft-remove"
                  disabled={busy && !awaiting}
                  aria-label={t('me.removeImage')}
                  onClick={() => setDraftImages((prev) => prev.filter((_, i) => i !== index))}
                >×</button>
              </div>
            ))}
          </div>
        )}
        <textarea
          ref={inputRef}
          className="me-input"
          rows={2}
          value={draft}
          placeholder={t('me.placeholder')}
          aria-label={t('me.composerLabel')}
          disabled={busy && !awaiting}
          onChange={(event) => setDraft(event.target.value)}
          onPaste={(event) => {
            const items = Array.from(event.clipboardData?.items ?? []);
            const files = items
              .filter((item) => item.kind === 'file' && item.type.startsWith('image/'))
              .map((item) => item.getAsFile())
              .filter((file): file is File => Boolean(file));
            if (files.length === 0) return;
            event.preventDefault();
            void addImageFiles(files);
          }}
          onKeyDown={(event) => {
            const composing = event.nativeEvent.isComposing || event.keyCode === 229;
            if (event.key === 'Escape') {
              if (awaiting) {
                event.preventDefault();
                void handleStop();
                return;
              }
              if (session?.pending && !busy) {
                void run(() => client.dismiss());
              }
              return;
            }
            if (event.key === 'Enter' && !event.shiftKey && !composing) {
              event.preventDefault();
              if (awaiting && (draft.trim() || draftImages.length > 0)) {
                void stopThenSend(draft);
                return;
              }
              if (awaiting) return;
              if (session?.pending && !busy && !draft.trim() && draftImages.length === 0) {
                void run(() => client.confirm(), true);
                return;
              }
              void send(draft);
            }
            if (event.key === 'Backspace' && !draft && draftImages.length > 0) {
              setDraftImages((prev) => prev.slice(0, -1));
            }
          }}
        />
        <div className="me-composer-actions">
          <label className="ghost tiny-btn me-pick-image" title={t('me.pickImage')}>
            <ImagePlus size={14} />
            <span className="me-sr-only">{t('me.pickImage')}</span>
            <input
              type="file"
              accept="image/png,image/jpeg,image/webp,image/gif"
              multiple
              hidden
              disabled={busy && !awaiting}
              onChange={(event) => {
                const files = Array.from(event.target.files ?? []);
                event.target.value = '';
                if (files.length) void addImageFiles(files);
              }}
            />
          </label>
          {session?.messages.some(
            (m) =>
              m.role === 'assistant' &&
              /tool_call|<\/?tool|invoke\s+\w+/i.test(m.content),
          ) && (
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={busy && !awaiting}
              title={t('me.rewriteBash')}
              onClick={() => setDraft(t('me.rewriteBashPrompt'))}
            >
              {t('me.rewriteBash')}
            </button>
          )}
          <button
            type="button"
            className="ghost"
            disabled={busy || !session || (session.messages.length === 0 && !session.pending && draftImages.length === 0)}
            onClick={() => {
              if (!clearArmed) {
                setClearArmed(true);
                return;
              }
              setClearArmed(false);
              void run(() => client.clear());
            }}
          >{clearArmed ? t('me.clearSure') : t('me.clear')}</button>
          {(() => {
            const lastAssistant = [...(session?.messages ?? [])].reverse().find((m) => m.role === 'assistant');
            if (!lastAssistant || busy || awaiting) return null;
            if (!looksLikeContinueable(lastAssistant.content || '')) return null;
            return (
              <button
                type="button"
                className="ghost"
                onClick={() => void send(t('me.continuePrompt'))}
              >{t('me.continue')}</button>
            );
          })()}
          <div className="me-composer-status" aria-live="polite">
            {session && session.messages.length > 0 && (
              <span className="me-status-hint" title={t('me.contextDepth', { count: session.messages.length })}>
                {t('me.contextDepth', { count: session.messages.length })}
              </span>
            )}
            {gatewayOn !== null && (
              <span
                className={gatewayOn ? 'me-gw-dot on' : 'me-gw-dot'}
                title={gatewayOn ? t('me.gatewayOn') : t('me.gatewayOff')}
                aria-label={gatewayOn ? t('me.gatewayOn') : t('me.gatewayOff')}
              />
            )}
            {placeLabel && (
              <button
                type="button"
                className="me-status-chip"
                disabled={!session}
                title={session?.work_dir ? `${t('me.openPlaceHint')}\n${session.work_dir}` : t('me.openPlaceHint')}
                onClick={() => client.openDir().catch((err: unknown) => setError(errorText(err)))}
              >{placeLabel}</button>
            )}
            {modelShort && (
              <button
                type="button"
                className="me-status-chip"
                title={modelLabel ? `${t('me.openModelHint')}\n${modelLabel}` : t('me.openModelHint')}
                onClick={() => onOpenModels?.()}
              >{modelShort}</button>
            )}
            <span className={runFlash ? (runFlash.ok ? 'me-status-hint ok' : 'me-status-hint bad') : 'me-status-hint'}>
              {statusHint}
            </span>
          </div>
          <div className="me-composer-right">
            <label className="me-auth-mode" title={t(`me.auth.${authMode}Hint` as TKey)}>
              <span className="me-auth-mode-label">{t('me.auth.label')}</span>
              <select
                className="me-auth-select"
                value={authMode}
                aria-label={t('me.auth.label')}
                onChange={(e) => {
                  const next = e.target.value as MeAuthMode;
                  saveMeAuthMode(next);
                  setAuthMode(next);
                  if (next !== 'ask' && session?.pending && !busy) {
                    void run(() => client.confirm(), true);
                  }
                }}
              >
                {ME_AUTH_MODES.map((mode) => (
                  <option key={mode} value={mode}>{t(`me.auth.${mode}` as TKey)}</option>
                ))}
              </select>
            </label>
            {awaiting ? (
              (draft.trim() || draftImages.length > 0) ? (
                <button type="submit" className="primary">
                  <Send size={15} />{t('me.stopAndSend')}
                </button>
              ) : (
                <button
                  type="button"
                  className="danger"
                  onClick={() => { void handleStop(); }}
                >
                  <Square size={14} />{t('me.stop')}
                </button>
              )
            ) : (
              <button type="submit" className="primary" disabled={busy || (!draft.trim() && draftImages.length === 0)}>
                <Send size={15} />{t('me.send')}
              </button>
            )}
          </div>
        </div>
      </form>
    </div>
  );
}

type SuBubbleProps = {
  content: string;
  steps: MeStep[];
  pending: MePending | null;
  busy: boolean;
  live?: string;
  streaming?: boolean;
  isLastAssistant?: boolean;
  onRegenerate?: () => void;
  onDeleteTurn?: () => void;
  onConfirm: () => void;
  onDismiss: () => void;
  onRoute?: (route: SoulRoute) => void;
};

const SOUL_ROUTE_ICONS: Record<SoulRoute, LucideIcon> = {
  clients: Bot,
  skills: Puzzle,
  preview: Eye,
  models: Cpu,
  logs: ScrollText,
  settings: Settings,
  pet: PawPrint,
};

const VALID_SOUL_ROUTES = Object.keys(SOUL_ROUTE_ICONS) as SoulRoute[];

/// 解析模型输出的意图标记；未知 / 半截标记一律剥离，绝不当作正文或指令执行。
function parseSoulRoutes(raw: string): { text: string; routes: SoulRoute[] } {
  if (!raw.includes('SUGT_ROUTE')) return { text: raw, routes: [] };
  const routes: SoulRoute[] = [];
  const pattern = /\[\[SUGT_ROUTE\]\]([a-z]+)\[\[\/SUGT_ROUTE\]\]/g;
  let text = raw.replace(pattern, (_match, name: string) => {
    const route = name as SoulRoute;
    if (VALID_SOUL_ROUTES.includes(route) && !routes.includes(route)) routes.push(route);
    return '';
  });
  // 流式时标记可能只写了一半（含正文中间残片），清掉避免露馅。
  text = text
    .replace(/\[\[SUGT_ROUTE\]\][a-z]*$/g, '')
    .replace(/\[\[\/?SUGT_ROUTE\]\]?/g, '')
    .replace(/\[\[\/?SUGT_ROU(?:TE(?:\]\])?)?$/, '')
    .replace(/\n{3,}/g, '\n\n')
    .trimEnd();
  return { text, routes };
}

function SoulRouteCard({ routes, onRoute }: { routes: SoulRoute[]; onRoute: (route: SoulRoute) => void }) {
  if (routes.length === 0) return null;
  return (
    <div className="me-route-card">
      <p className="hint compact">{t('me.routeHint')}</p>
      <div className="me-route-actions">
        {routes.map((route) => {
          const Icon = SOUL_ROUTE_ICONS[route];
          return (
            <button key={route} type="button" className="ghost tiny-btn me-route-btn" onClick={() => onRoute(route)}>
              <Icon size={14} aria-hidden />
              {t(`me.route.${route}` as TKey)}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function BubbleTools({
  text,
  showTurnActions,
  busy,
  onRegenerate,
  onDeleteTurn,
}: {
  text: string;
  showTurnActions?: boolean;
  busy?: boolean;
  onRegenerate?: () => void;
  onDeleteTurn?: () => void;
}) {
  const [state, setState] = useState<'idle' | 'ok' | 'fail'>('idle');
  useEffect(() => {
    if (state === 'idle') return;
    const timer = window.setTimeout(() => setState('idle'), 1600);
    return () => window.clearTimeout(timer);
  }, [state]);
  const label = state === 'ok' ? t('me.copied') : state === 'fail' ? t('me.copyFail') : t('me.copy');
  return (
    <div className="me-su-tools">
      <button
        type="button"
        className={state === 'ok' ? 'me-tool-btn ok' : 'me-tool-btn'}
        aria-label={label}
        title={label}
        onClick={() => {
          navigator.clipboard.writeText(text).then(() => setState('ok'), () => setState('fail'));
        }}
      >
        {state === 'ok' ? <Check size={13} aria-hidden /> : <Copy size={13} aria-hidden />}
        <span>{label}</span>
      </button>
      {showTurnActions && onRegenerate && (
        <button
          type="button"
          className="me-tool-btn"
          disabled={busy}
          aria-label={t('me.regenerate')}
          title={t('me.regenerate')}
          onClick={onRegenerate}
        >
          <RotateCcw size={13} aria-hidden />
          <span>{t('me.regenerate')}</span>
        </button>
      )}
      {showTurnActions && onDeleteTurn && (
        <button
          type="button"
          className="me-tool-btn"
          disabled={busy}
          aria-label={t('me.deleteTurn')}
          title={t('me.deleteTurn')}
          onClick={onDeleteTurn}
        >
          <Trash2 size={13} aria-hidden />
          <span>{t('me.deleteTurn')}</span>
        </button>
      )}
    </div>
  );
}

function SuBubble({
  content,
  steps,
  pending,
  busy,
  live,
  streaming,
  isLastAssistant,
  onRegenerate,
  onDeleteTurn,
  onConfirm,
  onDismiss,
  onRoute,
}: SuBubbleProps) {
  const liveProcess = Boolean(live) || Boolean(streaming);
  const hasFailedStep = steps.some(
    (step) => step.timed_out || (step.exit_code !== null && step.exit_code !== 0),
  );
  // 进行中强制展开；结束后失败默认展开且可折叠，成功默认收起可展开。
  const processProps = liveProcess
    ? ({ open: true } as const)
    : hasFailedStep
      ? ({ defaultOpen: true } as const)
      : ({ defaultOpen: false } as const);
  const parsed = useMemo(() => parseSoulRoutes(content), [content]);
  return (
    <article className="me-su">
      <span className={live || streaming ? 'me-seal live' : 'me-seal'} aria-hidden>su</span>
      <div className="me-su-body">
        {live && <p className="me-live">{live}</p>}
        {parsed.text && (
          <div className={streaming ? 'me-stream' : undefined}>
            <MeMarkdown text={parsed.text} />
            {streaming && <span className="me-caret" aria-hidden />}
          </div>
        )}
        {onRoute && <SoulRouteCard routes={parsed.routes} onRoute={onRoute} />}
        {parsed.text && !live && !streaming && (
          <BubbleTools
            text={parsed.text}
            showTurnActions={Boolean(isLastAssistant)}
            busy={busy}
            onRegenerate={onRegenerate}
            onDeleteTurn={onDeleteTurn}
          />
        )}
        {steps.length > 0 && (
          <details className="me-process" {...processProps}>
            <summary>
              {live || streaming
                ? t('me.stepsLive', { count: steps.length })
                : t('me.stepsSummary', { count: steps.length })}
            </summary>
            <div className="me-process-list">
              {steps.map((step, index) => (
                <StepSlip key={`${step.command}-${index}`} step={step} />
              ))}
            </div>
          </details>
        )}
        {pending && (
          <PendingNote pending={pending} busy={busy} onConfirm={onConfirm} onDismiss={onDismiss} />
        )}
      </div>
    </article>
  );
}

function PendingNote({
  pending,
  busy,
  onConfirm,
  onDismiss,
}: {
  pending: MePending;
  busy: boolean;
  onConfirm: () => void;
  onDismiss: () => void;
}) {
  const files = parsePendingFiles(pending.command);
  return (
    <div className="me-note">
      <strong>{t('me.confirmTitle')}</strong>
      <p className="me-note-hint">{t('me.confirmHint')} · {t('me.auth.askHint')}</p>
      {files.length > 0 ? (
        <div className="me-files">
          {files.map((file) => (
            <article key={`${file.append ? 'a' : 'w'}:${file.path}`} className="me-file">
              <header>
                <span>{file.append ? t('me.fileAppend') : t('me.fileWrite')}</span>
                <code title={file.path}>{file.path}</code>
              </header>
              {file.content == null ? (
                <p className="me-file-hint">{t('me.fileNoPreview')}</p>
              ) : file.content === '' ? (
                <p className="me-file-hint">{t('me.fileEmpty')}</p>
              ) : (
                <pre>{clipPreview(file.content)}</pre>
              )}
            </article>
          ))}
        </div>
      ) : (
        <pre className="me-note-raw">{pending.command}</pre>
      )}
      <details className="me-cmd">
        <summary>{t('me.showCommand')}</summary>
        <pre>{pending.command}</pre>
      </details>
      <div className="me-note-actions">
        <button type="button" className="primary" disabled={busy} onClick={onConfirm}>{t('me.write')}</button>
        <button type="button" className="ghost" disabled={busy} onClick={onDismiss}>{t('me.hold')}</button>
      </div>
    </div>
  );
}

function StepSlip({ step }: { step: MeStep }) {
  const status = step.timed_out
    ? t('me.stepTimeout')
    : step.exit_code === null
      ? ''
      : step.exit_code === 0
        ? t('me.stepOk')
        : t('me.stepExit', { code: step.exit_code });
  return (
    <details className="me-slip">
      <summary>
        <span>bash</span>
        <code title={step.command}>{step.command}</code>
        {status && <em className={step.exit_code === 0 && !step.timed_out ? 'ok' : undefined}>{status}</em>}
        {step.truncated && <em>{t('me.stepTruncated')}</em>}
      </summary>
      {step.note && <p className="me-slip-note">{step.note}</p>}
      {step.output && <pre>{step.output}</pre>}
    </details>
  );
}

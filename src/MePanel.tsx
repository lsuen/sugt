import React, { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ChevronDown, FolderOpen, Send, Square } from 'lucide-react';
import { lang, t } from './i18n';
import { MeMarkdown } from './meMarkdown';
import { clipPreview, parsePendingFiles } from './mePendingFiles';

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
  openDir: () => Promise<void>;
  subscribe: (onStep: (event: MeEvent) => void) => Promise<() => void>;
};

function errorText(err: unknown): string {
  if (err instanceof Error && err.message) return err.message;
  return String(err);
}

const emptyEvent = (kind: string, text = '', command = ''): MeEvent => ({
  kind,
  text,
  command,
  output: '',
  exit_code: null,
  timed_out: false,
});

const AUTO_WRITE_KEY = 'sugt_me_auto_write';

function loadAutoWrite(): boolean {
  try {
    return localStorage.getItem(AUTO_WRITE_KEY) === '1';
  } catch {
    return false;
  }
}

function saveAutoWrite(on: boolean): void {
  try {
    localStorage.setItem(AUTO_WRITE_KEY, on ? '1' : '0');
  } catch {
    /* ignore */
  }
}

export function tauriMeApi(): MeApi {
  const language = () => lang();
  return {
    load: () => invoke<MeView>('me_status'),
    setPlace: (mode, project) => invoke<MeView>('me_set_place', { mode, project: project ?? null, lang: language() }),
    createProject: (name) => invoke<MeView>('me_create_project', { name, lang: language() }),
    send: (message, images) => invoke<MeView>('me_send', {
      message,
      images: images && images.length > 0 ? images : null,
      lang: language(),
      autoWrite: loadAutoWrite(),
    }),
    confirm: () => invoke<MeView>('me_confirm', { lang: language() }),
    dismiss: () => invoke<MeView>('me_dismiss', { lang: language() }),
    stop: () => invoke('me_stop'),
    clear: () => invoke<MeView>('me_clear'),
    openDir: () => invoke('me_open_dir'),
    subscribe: async (onStep) => {
      const unlisten = await listen<MeEvent>('sugt://me-step', (event) => onStep(event.payload));
      return unlisten;
    },
  };
}

export function createPreviewMeApi(): MeApi {
  const home = '/Users/me/Documents/.sugt';
  let view: MeView = {
    mode: 'project',
    project: 'default',
    projects: ['default'],
    work_dir: `${home}/projects/default`,
    projects_root: `${home}/projects`,
    config_dir: home,
    messages: [],
    pending: null,
  };
  let onStep: ((event: MeEvent) => void) | null = null;
  const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));
  const place = (next: Partial<MeView>): MeView => {
    view = { ...view, ...next };
    if (view.mode === 'config') view.work_dir = view.config_dir;
    else view.work_dir = `${view.projects_root}/${view.project}`;
    return view;
  };
  const chats = new Map<string, { messages: MeMessage[]; pending: MePending | null }>();
  const keyOf = (mode: string, project: string) => (mode === 'config' ? 'config' : `project:${project}`);
  const stash = () => {
    chats.set(keyOf(view.mode, view.project), { messages: view.messages, pending: view.pending });
  };
  const restore = (mode: string, project: string) => {
    const saved = chats.get(keyOf(mode, project));
    return {
      messages: saved?.messages ?? [],
      pending: saved?.pending ?? null,
    };
  };
  return {
    load: async () => view,
    setPlace: async (mode, project) => {
      stash();
      const nextProject = project ?? view.project;
      const chat = restore(mode, nextProject);
      return place({ mode, project: nextProject, ...chat });
    },
    createProject: async (name) => {
      const project = name.trim();
      if (!project || project.includes('/') || project.includes('..')) {
        throw new Error('项目名不能带路径');
      }
      stash();
      const projects = view.projects.includes(project) ? view.projects : [...view.projects, project].sort();
      const chat = restore('project', project);
      return place({ mode: 'project', project, projects, ...chat });
    },
    send: async (message, images = []) => {
      const text = message.trim();
      if (!text && images.length === 0) return view;
      onStep?.(emptyEvent('thinking', t('me.thinking')));
      await wait(280);
      const user = {
        id: `u-${view.messages.length}`,
        role: 'user',
        content: text,
        images,
        steps: [],
        created_at: Date.now(),
      };
      const writing = /建|写|project|notes|file/i.test(text);
      if (writing) {
        const command = [
          'mkdir -p notes',
          "cat > notes/readme.txt << 'EOF'",
          'hello',
          '',
          'from su',
          'EOF',
        ].join('\n');
        view = {
          ...view,
          pending: { command, prose: '写到 notes/readme.txt' },
          messages: [...view.messages, user, {
            id: `a-${view.messages.length}`,
            role: 'assistant',
            content: '我把说明写到 `notes/readme.txt`。',
            steps: [],
            created_at: Date.now(),
          }],
        };
        stash();
        return view;
      }
      onStep?.(emptyEvent('running', '', 'ls -la'));
      await wait(220);
      const step: MeStep = {
        command: 'ls -la',
        output: 'total 0\n',
        exit_code: 0,
        truncated: false,
        timed_out: false,
        note: '',
      };
      onStep?.({ ...emptyEvent('ran', '', step.command), output: step.output, exit_code: 0 });
      view = {
        ...view,
        pending: null,
        messages: [...view.messages, user, {
          id: `a-${view.messages.length}`,
          role: 'assistant',
          content: '这里还是空的。\n\n- 要我建项目，直接说名字\n- 也可以让我 `ls` 再看一眼',
          steps: [step],
          created_at: Date.now(),
        }],
      };
      stash();
      return view;
    },
    confirm: async () => {
      if (!view.pending) return view;
      const command = view.pending.command;
      onStep?.(emptyEvent('running', '', command));
      await wait(240);
      const step: MeStep = {
        command,
        output: '',
        exit_code: 0,
        truncated: false,
        timed_out: false,
        note: view.pending.prose,
      };
      onStep?.({ ...emptyEvent('ran', step.note, command), exit_code: 0 });
      view = {
        ...view,
        pending: null,
        messages: [...view.messages, {
          id: `u-${view.messages.length}`,
          role: 'user',
          content: t('me.write'),
          steps: [],
          created_at: Date.now(),
        }, {
          id: `a-${view.messages.length}`,
          role: 'assistant',
          content: '写好了。\n\n复杂的下一步，**Claude Code** 或 **Codex** 会更稳。',
          steps: [step],
          created_at: Date.now(),
        }],
      };
      stash();
      return view;
    },
    dismiss: async () => {
      view = {
        ...view,
        pending: null,
        messages: [...view.messages, {
          id: `u-${view.messages.length}`,
          role: 'user',
          content: t('me.hold'),
          steps: [],
          created_at: Date.now(),
        }, {
          id: `a-${view.messages.length}`,
          role: 'assistant',
          content: '好，那我先不动。',
          steps: [],
          created_at: Date.now(),
        }],
      };
      stash();
      return view;
    },
    stop: async () => undefined,
    clear: async () => {
      const next = place({ messages: [], pending: null });
      stash();
      return next;
    },
    openDir: async () => undefined,
    subscribe: async (listener) => {
      onStep = listener;
      return () => {
        if (onStep === listener) onStep = null;
      };
    },
  };
}

type Props = {
  api?: MeApi;
  onOpenClients: () => void;
};

export function MePanel({ api, onOpenClients }: Props) {
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
  const [autoWrite, setAutoWrite] = useState(() => loadAutoWrite());
  const logRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const placeMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let stop = false;
    let unlisten: (() => void) | undefined;
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
      if (event.kind === 'thinking') setLive(event.text || t('me.thinking'));
      if (event.kind === 'running') setLive(event.command || event.text || t('me.working'));
      if (event.kind === 'ran') {
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
    };
  }, [client]);

  useEffect(() => {
    const node = logRef.current;
    if (!node) return;
    node.scrollTop = node.scrollHeight;
  }, [session, live, liveSteps, awaiting]);

  useEffect(() => {
    if (!clearArmed) return;
    const timer = window.setTimeout(() => setClearArmed(false), 8000);
    return () => window.clearTimeout(timer);
  }, [clearArmed]);

  useEffect(() => {
    if (!placeOpen) return;
    const onPointer = (event: MouseEvent) => {
      if (!placeMenuRef.current?.contains(event.target as Node)) setPlaceOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setPlaceOpen(false);
    };
    window.addEventListener('mousedown', onPointer);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', onPointer);
      window.removeEventListener('keydown', onKey);
    };
  }, [placeOpen]);

  async function run(task: () => Promise<MeView>, waitForSu = false) {
    setError('');
    setBusy(true);
    setAwaiting(waitForSu);
    if (waitForSu) {
      setLive(t('me.thinking'));
      setLiveSteps([]);
    }
    let ok = false;
    try {
      const view = await task();
      setSession(view);
      setHistory(rememberPlace({
        mode: view.mode,
        project: view.project,
        path: view.work_dir,
      }));
      ok = true;
    } catch (err) {
      setError(errorText(err));
      try {
        setSession(await client.load());
      } catch {
        /* 保持当前画面 */
      }
    } finally {
      setBusy(false);
      setAwaiting(false);
      setLive('');
      setLiveSteps([]);
      inputRef.current?.focus();
    }
    return ok;
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

  async function send(text: string, images = draftImages) {
    const message = text.trim();
    if ((!message && images.length === 0) || busy) return;
    setDraft('');
    setDraftImages([]);
    await run(() => client.send(message, images), true);
  }

  const projects = session
    ? (session.projects.includes(session.project) ? session.projects : [session.project, ...session.projects])
    : [];

  const leaf = session ? pathLeaf(session.work_dir) : '…';
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

        <div className="me-place" ref={placeMenuRef}>
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
          <button
            type="button"
            className={autoWrite ? 'tiny me-auto-write on' : 'tiny me-auto-write'}
            aria-pressed={autoWrite}
            title={autoWrite ? t('me.autoWriteOn') : t('me.autoWriteHint')}
            disabled={busy}
            onClick={() => {
              const next = !autoWrite;
              saveAutoWrite(next);
              setAutoWrite(next);
              if (next && session?.pending) {
                void run(() => client.confirm(), true);
              }
            }}
          >{t('me.autoWrite')}</button>
          <button type="button" className="tiny" disabled={busy || !session} onClick={() => client.openDir().catch((err: unknown) => setError(errorText(err)))}>
            <FolderOpen size={14} />{t('me.openDir')}
          </button>
          <button type="button" className="tiny" onClick={onOpenClients}>{t('me.handoff')}</button>
        </div>
      </div>
      {session?.mode === 'config' && <p className="me-warn">{t('me.configWarn')}</p>}

      <div className="me-log" ref={logRef} role="log" aria-live="polite">
        {session && session.messages.length === 0 && !awaiting && (
          <div className="me-hello">
            <div className="me-who">
              <span className="me-seal" aria-hidden>su</span>
              <strong>{t('me.name')}</strong>
            </div>
            <MeMarkdown text={t('me.greeting')} />
            <div className="me-ideas">
              <button type="button" className="tiny" onClick={() => void send(t('me.suggestList'))}>{t('me.suggestList')}</button>
              <button type="button" className="tiny" onClick={() => void send(t('me.suggestProject'))}>{t('me.suggestProject')}</button>
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
              onConfirm={() => void run(() => client.confirm(), true)}
              onDismiss={() => void run(() => client.dismiss())}
            />
          );
        })}
        {awaiting && (
          <SuBubble
            content=""
            steps={liveSteps}
            live={live || t('me.thinking')}
            pending={null}
            busy={busy}
            onConfirm={() => undefined}
            onDismiss={() => undefined}
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
          />
        )}
      </div>

      {error && <p className="me-error" role="alert">{error}</p>}

      <form
        className="me-composer"
        onSubmit={(event) => {
          event.preventDefault();
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
                  disabled={busy}
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
          disabled={busy}
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
            if (event.key === 'Enter' && !event.shiftKey && !composing) {
              event.preventDefault();
              void send(draft);
            }
            if (event.key === 'Backspace' && !draft && draftImages.length > 0) {
              setDraftImages((prev) => prev.slice(0, -1));
            }
            if (event.key === 'Escape' && session?.pending && !busy) {
              void run(() => client.dismiss());
            }
          }}
        />
        <div className="me-composer-actions">
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
          {awaiting ? (
            <button
              type="button"
              className="danger"
              onClick={() => {
                void client.stop().catch((err: unknown) => setError(errorText(err)));
              }}
            >
              <Square size={14} />{t('me.stop')}
            </button>
          ) : (
            <button type="submit" className="primary" disabled={busy || (!draft.trim() && draftImages.length === 0)}>
              <Send size={15} />{t('me.send')}
            </button>
          )}
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
  onConfirm: () => void;
  onDismiss: () => void;
};

function SuBubble({ content, steps, pending, busy, live, onConfirm, onDismiss }: SuBubbleProps) {
  const processOpen = Boolean(live);
  return (
    <article className="me-su">
      <span className={live ? 'me-seal live' : 'me-seal'} aria-hidden>su</span>
      <div className="me-su-body">
        {live && <p className="me-live">{live}</p>}
        {content && <MeMarkdown text={content} />}
        {steps.length > 0 && (
          <details className="me-process" open={processOpen}>
            <summary>
              {live
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
      <p className="me-note-hint">{t('me.confirmHint')} · {t('me.autoWriteHint')}</p>
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

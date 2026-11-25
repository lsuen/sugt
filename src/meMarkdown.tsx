import React, { useEffect, useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { t } from './i18n';

/** 轻量 Markdown：加粗、斜体、行内代码、列表；工具调用 / 过程输出 / 命令块折叠。 */
export function MeMarkdown({ text, className }: { text: string; className?: string }) {
  const parts = splitChatParts(text);
  if (parts.length === 0) return null;
  return (
    <div className={className ? `me-md ${className}` : 'me-md'}>
      {parts.map((part, index) => {
        if (part.kind !== 'md') {
          // 模型常常把命令 / HTML 直接贴成正文（没加代码围栏）。这类内容收进有高度上限的
          // 容器里滚动，否则一屏对话会被一段长文本拉成一堵墙。
          if (part.kind === 'cmd' && part.boxed) {
            return <CodeBox key={index} raw={part.raw} />;
          }
          return (
            <details key={index} className={part.kind === 'cmd' ? 'me-tool-fold me-cmd-fold' : 'me-tool-fold'}>
              <summary title={part.summary}>{part.summary}</summary>
              <pre className="me-tool-raw">{part.raw}</pre>
            </details>
          );
        }
        const blocks = splitBlocks(part.text);
        if (blocks.length === 0) return null;
        return (
          <React.Fragment key={index}>
            {blocks.map((block, blockIndex) => (
              <Block key={blockIndex} block={block} />
            ))}
          </React.Fragment>
        );
      })}
    </div>
  );
}

type ChatPart =
  | { kind: 'md'; text: string }
  | { kind: 'tools' | 'shell' | 'safety'; raw: string; summary: string }
  | { kind: 'cmd'; raw: string; summary: string; boxed?: boolean };

/** 有高度上限的代码/命令容器：内容再长也只在框内滚动，不挤占对话。 */
function CodeBox({ raw }: { raw: string }) {
  const [state, setState] = useState<'idle' | 'ok' | 'fail'>('idle');
  useEffect(() => {
    if (state === 'idle') return;
    const timer = window.setTimeout(() => setState('idle'), 1600);
    return () => window.clearTimeout(timer);
  }, [state]);
  const lines = raw.split('\n').filter((line) => line.trim()).length;
  const label = state === 'ok' ? t('me.copied') : state === 'fail' ? t('me.copyFail') : t('me.copy');
  return (
    <div className="me-code">
      <div className="me-code-head">
        <span>{t('me.codeBlock', { lines })}</span>
        <button
          type="button"
          className={state === 'ok' ? 'me-code-copy ok' : 'me-code-copy'}
          aria-label={label}
          title={label}
          onClick={() => {
            navigator.clipboard.writeText(raw).then(() => setState('ok'), () => setState('fail'));
          }}
        >
          {state === 'ok' ? <Check size={12} aria-hidden /> : <Copy size={12} aria-hidden />}
          <span>{label}</span>
        </button>
      </div>
      <pre className="me-code-raw">{raw}</pre>
    </div>
  );
}

const TOOL_BLOCK_RE =
  /<\|tool_call_start\|>[\s\S]*?(?:<\|tool_call_end\|>|$)|<tool_call\b[^>]*>[\s\S]*?<\/tool_call>/gi;

const FENCE_RE = /```(?:bash|sh|shell|zsh|pwsh|powershell|cmd)?\n?[\s\S]*?```/gi;

const LOOSE_TOOL_RE =
  /(?:^|\n)\s*(?:\[\s*)?(?:write|read|search_replace|apply_patch|edit_file|StrReplace)\s*\(\s*path\s*=/i;

export function splitChatParts(text: string): ChatPart[] {
  const source = text.replace(/\r\n/g, '\n');
  if (!source.trim()) return [];

  let parts: ChatPart[] = [{ kind: 'md', text: source }];
  parts = splitByRegex(parts, TOOL_BLOCK_RE, (raw) => ({
    kind: 'tools',
    raw: raw.trim(),
    summary: summarizeToolBlob(raw),
  }));
  parts = splitByRegex(parts, FENCE_RE, (raw) => ({
    kind: 'cmd',
    raw: raw.trim(),
    summary: summarizeFence(raw),
  }));
  parts = parts.flatMap((part) => {
    if (part.kind !== 'md') return [part];
    return splitShellDumps(part.text);
  });
  parts = parts.flatMap((part) => {
    if (part.kind !== 'md') return [part];
    return splitCodeDumps(part.text);
  });
  parts = parts.flatMap((part) => {
    if (part.kind !== 'md') return [part];
    return splitSafetyBlocks(part.text);
  });

  if (parts.length === 1 && parts[0].kind === 'md' && LOOSE_TOOL_RE.test(parts[0].text)) {
    return [{
      kind: 'tools',
      raw: parts[0].text.trim(),
      summary: summarizeToolBlob(parts[0].text),
    }];
  }

  return parts.filter((part) => {
    if (part.kind === 'md') return part.text.trim().length > 0;
    return part.raw.trim().length > 0;
  });
}

function splitByRegex(
  parts: ChatPart[],
  re: RegExp,
  toPart: (raw: string) => Exclude<ChatPart, { kind: 'md' }>,
): ChatPart[] {
  const out: ChatPart[] = [];
  for (const part of parts) {
    if (part.kind !== 'md') {
      out.push(part);
      continue;
    }
    const source = part.text;
    let cursor = 0;
    re.lastIndex = 0;
    let match: RegExpExecArray | null;
    let found = false;
    while ((match = re.exec(source)) !== null) {
      found = true;
      if (match.index > cursor) pushMd(out, source.slice(cursor, match.index));
      out.push(toPart(match[0]));
      cursor = match.index + match[0].length;
    }
    if (!found) {
      out.push(part);
    } else if (cursor < source.length) {
      pushMd(out, source.slice(cursor));
    }
  }
  return out;
}

function splitShellDumps(text: string): ChatPart[] {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const out: ChatPart[] = [];
  let buf: string[] = [];
  let shellBuf: string[] = [];

  const flushMd = () => {
    if (buf.length === 0) return;
    pushMd(out, buf.join('\n'));
    buf = [];
  };
  const flushShell = () => {
    if (shellBuf.length === 0) return;
    const raw = shellBuf.join('\n').trim();
    if (raw) {
      out.push({
        kind: 'shell',
        raw,
        summary: t('me.shellFold', { lines: shellBuf.filter((line) => line.trim()).length }),
      });
    }
    shellBuf = [];
  };

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    const inShell = shellBuf.length > 0;
    if (isShellDumpLine(line) || (inShell && !line.trim() && nextIsShellDump(lines, i + 1))) {
      flushMd();
      shellBuf.push(line);
    } else {
      flushShell();
      buf.push(line);
    }
  }
  flushShell();
  flushMd();
  return out.length > 0 ? out : [{ kind: 'md', text }];
}

/** heredoc 起手式：`cat << 'EOF' > file`。要求前面有命令名，避免把 `x << 2` 当代码块。 */
const HEREDOC_RE = /\b(?:cat|tee|python3?|node|bash|sh|zsh|pwsh|powershell)\b[^\n]*<<-?\s*(['"]?)([A-Za-z_][A-Za-z0-9_]*)\1/;
/** 一眼能认出是标记语言的行（HTML / CSS / JS 标签片断）。 */
const MARKUP_RE = /<!doctype|<html[\s>]|<\/[a-z][\w-]*>|<style[\s>]|<script[\s>]/i;
/** 以命令名开头的行；配合 `> | & ;` 判定为 Shell 片段。 */
const SHELL_HEAD_RE =
  /^(?:\$\s+)?(?:sudo\s+)?(?:cat|echo|printf|tee|curl|wget|npm|pnpm|yarn|npx|cargo|node|python3?|deno|git|docker|find|grep|sed|awk|rm|cp|mv|mkdir|touch|chmod|tar|scp|ssh|systemctl|powershell|pwsh)\b/i;
const CODE_MIN_CHARS = 72;

/** 一行是否像「没加围栏的代码」。太短的串（比如一句 `npm run dev`）交给正文即可。 */
function looksLikeCodeDump(line: string): boolean {
  const trimmed = line.trim();
  if (trimmed.length < CODE_MIN_CHARS) return false;
  if (MARKUP_RE.test(trimmed)) return true;
  if (SHELL_HEAD_RE.test(trimmed) && /[>|;&]/.test(trimmed)) return true;
  const marks = (trimmed.match(/[{}();=<>]/g) || []).length;
  return marks >= 16 && marks / trimmed.length >= 0.08;
}

/** heredoc 里混进正文的行：中文、没有代码符号、不是缩进行。用来判断这段代码其实已经结束。 */
function looksLikeProse(line: string | undefined): boolean {
  const source = line ?? '';
  const trimmed = source.trim();
  if (!trimmed) return false;
  if (/^\s{2,}/.test(source)) return false;
  if (!/[\u3400-\u9fff]/.test(trimmed)) return false;
  return !/[{}<>();=]/.test(trimmed);
}

/**
 * 抽走没加围栏的代码 / 命令段落（模型直接把 heredoc、整段 HTML 当正文发出来的情况）。
 * 这些内容交给有高度上限的容器显示，正文保持原样。
 */
function splitCodeDumps(text: string): ChatPart[] {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const out: ChatPart[] = [];
  let buf: string[] = [];
  let codeBuf: string[] = [];
  let heredocEnd: string | null = null;

  const flushMd = () => {
    if (buf.length === 0) return;
    pushMd(out, buf.join('\n'));
    buf = [];
  };
  const flushCode = () => {
    if (codeBuf.length === 0) return;
    const raw = codeBuf.join('\n').trim();
    if (raw) out.push({ kind: 'cmd', raw, summary: summarizeDump(raw), boxed: true });
    codeBuf = [];
  };

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    if (heredocEnd) {
      if (line.trim() === heredocEnd) {
        codeBuf.push(line);
        heredocEnd = null;
        continue;
      }
      // 没有 EOF 收尾的半截粘贴：空行之后接一句中文正文，就认定代码到此为止。
      if (!line.trim() && looksLikeProse(lines[i + 1])) {
        heredocEnd = null;
        flushCode();
        buf.push(line);
        continue;
      }
      codeBuf.push(line);
      continue;
    }
    const heredoc = HEREDOC_RE.exec(line);
    if (heredoc) {
      flushMd();
      codeBuf.push(line);
      heredocEnd = heredoc[2];
      continue;
    }
    if (looksLikeCodeDump(line)) {
      flushMd();
      codeBuf.push(line);
      continue;
    }
    flushCode();
    buf.push(line);
  }
  flushCode();
  flushMd();
  return out.length > 0 ? out : [{ kind: 'md', text }];
}

function summarizeDump(raw: string): string {
  const first = raw.split('\n').find((line) => line.trim())?.trim() ?? '';
  const short = first.length > 48 ? `${first.slice(0, 48)}…` : first;
  return short ? t('me.cmdFoldOne', { preview: short }) : t('me.cmdFold');
}

function splitSafetyBlocks(text: string): ChatPart[] {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const safety = lines.filter((line) => isSafetyLine(line));
  const rest = lines.filter((line) => !isSafetyLine(line));
  if (safety.length === 0) return [{ kind: 'md', text }];
  const out: ChatPart[] = [];
  if (rest.some((line) => line.trim())) pushMd(out, rest.join('\n'));
  out.push({
    kind: 'safety',
    raw: safety.join('\n'),
    summary: t('me.safetyFold'),
  });
  return out;
}

function pushMd(parts: ChatPart[], text: string) {
  const trimmed = text.replace(/^\n+|\n+$/g, '');
  if (!trimmed.trim()) return;
  parts.push({ kind: 'md', text: trimmed });
}

function isSafetyLine(line: string): boolean {
  const lower = line.trim().toLowerCase();
  return (
    lower.startsWith('user safety:')
    || lower.startsWith('response safety:')
    || lower.startsWith('safety categories:')
    || lower.startsWith('safety category:')
  );
}

function isShellDumpLine(line: string): boolean {
  const t = line.trim();
  if (!t) return false;
  if (/^exit:\s*/i.test(t)) return true;
  if (/^===/.test(t)) return true;
  if (/^-{3,}$/.test(t)) return true;
  if (/^total\s+\d+/i.test(t)) return true;
  if (/^[dl-][rwxsStT-]{9}/.test(t)) return true;
  if (/^drwx/.test(t) || /^-rw/.test(t)) return true;
  return false;
}

function nextIsShellDump(lines: string[], from: number): boolean {
  for (let i = from; i < lines.length; i += 1) {
    if (!lines[i].trim()) continue;
    return isShellDumpLine(lines[i]);
  }
  return false;
}

function summarizeToolBlob(raw: string): string {
  const ops: { name: string; path: string }[] = [];
  const re =
    /\b(write|read|search_replace|apply_patch|edit_file|StrReplace|bash)\s*\(\s*(?:path\s*=\s*)?['"]([^'"]+)['"]/gi;
  let match: RegExpExecArray | null;
  while ((match = re.exec(raw)) !== null) {
    ops.push({ name: match[1].toLowerCase(), path: pathLeaf(match[2]) });
  }
  if (ops.length === 0) {
    if (/write\s*\(/i.test(raw)) return t('me.toolFoldWrite');
    if (/read\s*\(/i.test(raw)) return t('me.toolFoldRead');
    return t('me.toolFold');
  }
  if (ops.length === 1) {
    return t('me.toolFoldOne', { name: ops[0].name, file: ops[0].path });
  }
  const names = [...new Set(ops.map((op) => op.name))].slice(0, 3).join('/');
  return t('me.toolFoldMulti', { count: ops.length, names });
}

function summarizeFence(raw: string): string {
  const body = raw.replace(/^```[^\n]*\n?/, '').replace(/```$/, '').trim();
  return summarizeDump(body);
}

function pathLeaf(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/').filter(Boolean);
  return parts[parts.length - 1] || path;
}

type Block =
  | { kind: 'p'; lines: string[] }
  | { kind: 'ul'; items: string[] }
  | { kind: 'ol'; items: string[] };

function splitBlocks(text: string): Block[] {
  const chunks = text.replace(/\r\n/g, '\n').trim().split(/\n{2,}/);
  const out: Block[] = [];
  for (const chunk of chunks) {
    const lines = chunk.split('\n').map((line) => line.trimEnd());
    if (lines.length > 0 && lines.every((line) => /^[-*]\s+\S/.test(line))) {
      out.push({ kind: 'ul', items: lines.map((line) => line.replace(/^[-*]\s+/, '')) });
      continue;
    }
    if (lines.length > 0 && lines.every((line) => /^\d+\.\s+\S/.test(line))) {
      out.push({ kind: 'ol', items: lines.map((line) => line.replace(/^\d+\.\s+/, '')) });
      continue;
    }
    out.push({ kind: 'p', lines });
  }
  return out;
}

function Block({ block }: { block: Block }) {
  if (block.kind === 'ul') {
    return (
      <ul>
        {block.items.map((item, index) => (
          <li key={index}>{renderInline(item)}</li>
        ))}
      </ul>
    );
  }
  if (block.kind === 'ol') {
    return (
      <ol>
        {block.items.map((item, index) => (
          <li key={index}>{renderInline(item)}</li>
        ))}
      </ol>
    );
  }
  return (
    <p>
      {block.lines.map((line, index) => (
        <React.Fragment key={index}>
          {index > 0 && <br />}
          {renderInline(line)}
        </React.Fragment>
      ))}
    </p>
  );
}

function renderInline(text: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  const pattern = /(`+)([^`]+)\1|\*\*([^*]+)\*\*|\*([^*]+)\*|__([^_]+)__|_([^_]+)_/g;
  let last = 0;
  let match: RegExpExecArray | null;
  let key = 0;
  while ((match = pattern.exec(text)) !== null) {
    if (match.index > last) nodes.push(text.slice(last, match.index));
    if (match[2] !== undefined) {
      nodes.push(<code key={key++}>{match[2]}</code>);
    } else if (match[3] !== undefined || match[5] !== undefined) {
      nodes.push(<strong key={key++}>{match[3] ?? match[5]}</strong>);
    } else {
      nodes.push(<em key={key++}>{match[4] ?? match[6]}</em>);
    }
    last = match.index + match[0].length;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

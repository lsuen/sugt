/** 从待确认的 shell 里尽量抽出「要写的文件 + 内容预览」。抽不出就退回整段命令。 */

export type PendingFile = {
  path: string;
  content: string | null;
  append: boolean;
};

const PREVIEW_CHARS = 1600;

export function parsePendingFiles(command: string): PendingFile[] {
  const text = command.replace(/\r\n/g, '\n');
  const found = new Map<string, PendingFile>();

  for (const file of parseHeredocs(text)) {
    found.set(fileKey(file), file);
  }
  for (const file of parseRedirectLiterals(text)) {
    const key = fileKey(file);
    if (!found.has(key) || (found.get(key)?.content == null && file.content != null)) {
      found.set(key, file);
    }
  }
  for (const file of parseTouches(text)) {
    const key = fileKey(file);
    if (!found.has(key)) found.set(key, file);
  }

  return [...found.values()];
}

export function clipPreview(content: string, limit = PREVIEW_CHARS): string {
  const chars = [...content];
  if (chars.length <= limit) return content;
  return `${chars.slice(0, limit).join('')}\n…`;
}

function fileKey(file: PendingFile): string {
  return `${file.append ? 'a' : 'w'}:${file.path}`;
}

function parseHeredocs(text: string): PendingFile[] {
  const out: PendingFile[] = [];
  const re =
    /(?:^|[\n;&|])\s*(?:cat|tee)\s+(>>?)\s*([^\s;&|]+)\s*<<\s*(['"]?)(\w+)\3\n([\s\S]*?)\n\4\b/gm;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text)) !== null) {
    out.push({
      path: stripQuotes(match[2]),
      content: match[5],
      append: match[1] === '>>',
    });
  }
  // tee FILE <<EOF（没有 >）
  const teeRe =
    /(?:^|[\n;&|])\s*tee\s+(-a\s+)?([^\s;&|]+)\s*<<\s*(['"]?)(\w+)\3\n([\s\S]*?)\n\4\b/gm;
  while ((match = teeRe.exec(text)) !== null) {
    out.push({
      path: stripQuotes(match[2]),
      content: match[5],
      append: Boolean(match[1]),
    });
  }
  return out;
}

function parseRedirectLiterals(text: string): PendingFile[] {
  const out: PendingFile[] = [];
  const re =
    /(?:^|[\n;&|])\s*(?:printf|echo)\s+(?:-[neE]+\s+)*((?:'[^']*'|"[^"\\]*(?:\\.[^"\\]*)*"|\\.|[^\s;&|>])+)\s*(>>?)\s*([^\s;&|]+)/gm;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text)) !== null) {
    out.push({
      path: stripQuotes(match[3]),
      content: decodeShellLiteral(match[1].trim()),
      append: match[2] === '>>',
    });
  }
  return out;
}

function parseTouches(text: string): PendingFile[] {
  const out: PendingFile[] = [];
  const re = /(?:^|[\n;&|])\s*touch\s+([^\n;&|]+)/gm;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text)) !== null) {
    for (const token of match[1].trim().split(/\s+/)) {
      const path = stripQuotes(token);
      if (!path || path.startsWith('-')) continue;
      out.push({ path, content: '', append: false });
    }
  }
  return out;
}

function stripQuotes(value: string): string {
  if (
    (value.startsWith("'") && value.endsWith("'"))
    || (value.startsWith('"') && value.endsWith('"'))
  ) {
    return value.slice(1, -1);
  }
  return value;
}

function decodeShellLiteral(raw: string): string | null {
  if (raw.startsWith("'") && raw.endsWith("'")) {
    return raw.slice(1, -1).replace(/\\n/g, '\n').replace(/\\t/g, '\t');
  }
  if (raw.startsWith('"') && raw.endsWith('"')) {
    return raw
      .slice(1, -1)
      .replace(/\\n/g, '\n')
      .replace(/\\t/g, '\t')
      .replace(/\\r/g, '\r')
      .replace(/\\\\/g, '\\')
      .replace(/\\"/g, '"');
  }
  if (raw.includes('%') || raw.includes('$(') || raw.includes('`')) return null;
  return raw.replace(/\\n/g, '\n').replace(/\\t/g, '\t').replace(/\\\\/g, '\\');
}

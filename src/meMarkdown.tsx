import React from 'react';

/** 轻量 Markdown：加粗、斜体、行内代码、列表。不渲染 HTML，也不引入依赖。 */
export function MeMarkdown({ text, className }: { text: string; className?: string }) {
  const blocks = splitBlocks(text);
  if (blocks.length === 0) return null;
  return (
    <div className={className ? `me-md ${className}` : 'me-md'}>
      {blocks.map((block, index) => (
        <Block key={index} block={block} />
      ))}
    </div>
  );
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

import type { ReactNode } from 'react';

const SAFE_PROTOCOL = /^https?:\/\//i;
const BLOCK_START =
  /^(#{1,6}\s|```|>\s?|\s*[-*]\s+|\s*\d+\.\s+|\s*(?:---+|\*\*\*+)\s*$)/;

function inline(text: string, keyPrefix: string): ReactNode[] {
  const pattern =
    /(`[^`]+`)|(\*\*[^*]+\*\*)|(\[[^\]]+\]\([^)\s]+\))|(\*[^*]+\*)/g;
  const nodes: ReactNode[] = [];
  let last = 0;
  let index = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    if (match.index > last) {
      nodes.push(text.slice(last, match.index));
    }
    const token = match[0];
    const key = `${keyPrefix}-${index++}`;
    if (token.startsWith('`')) {
      nodes.push(<code key={key}>{token.slice(1, -1)}</code>);
    } else if (token.startsWith('**')) {
      nodes.push(
        <strong key={key}>
          {inline(token.slice(2, -2), `${key}-strong`)}
        </strong>,
      );
    } else if (token.startsWith('*')) {
      nodes.push(
        <em key={key}>{inline(token.slice(1, -1), `${key}-em`)}</em>,
      );
    } else {
      const link = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(token);
      if (link && SAFE_PROTOCOL.test(link[2])) {
        nodes.push(
          <a
            key={key}
            href={link[2]}
            target="_blank"
            rel="noopener noreferrer"
          >
            {link[1]}
          </a>,
        );
      } else if (link) {
        // Unsafe protocols stay inert text; no raw HTML or javascript: links.
        nodes.push(link[1]);
      } else {
        nodes.push(token);
      }
    }
    last = pattern.lastIndex;
  }
  if (last < text.length) {
    nodes.push(text.slice(last));
  }
  return nodes;
}

/**
 * Small, deliberately conservative Markdown renderer. It returns React nodes,
 * so raw HTML in stored project data is always displayed as text rather than
 * executed. Links are limited to http(s); images are never loaded.
 */
export function Markdown({
  text,
  className,
}: {
  text: string;
  className?: string;
}) {
  const lines = String(text ?? '')
    .replace(/\r\n?/g, '\n')
    .split('\n');
  const blocks: ReactNode[] = [];
  let lineIndex = 0;
  let blockIndex = 0;
  const nextKey = () => `block-${blockIndex++}`;

  while (lineIndex < lines.length) {
    const line = lines[lineIndex];
    if (/^\s*$/.test(line)) {
      lineIndex += 1;
      continue;
    }

    if (/^```/.test(line)) {
      const code: string[] = [];
      lineIndex += 1;
      while (lineIndex < lines.length && !/^```/.test(lines[lineIndex])) {
        code.push(lines[lineIndex++]);
      }
      if (lineIndex < lines.length) {
        lineIndex += 1;
      }
      blocks.push(
        <pre key={nextKey()}>
          <code>{code.join('\n')}</code>
        </pre>,
      );
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      const level = heading[1].length as 1 | 2 | 3 | 4 | 5 | 6;
      const Tag = `h${level}` as
        | 'h1'
        | 'h2'
        | 'h3'
        | 'h4'
        | 'h5'
        | 'h6';
      blocks.push(<Tag key={nextKey()}>{inline(heading[2], nextKey())}</Tag>);
      lineIndex += 1;
      continue;
    }

    if (/^\s*[-*]\s+/.test(line)) {
      const items: ReactNode[] = [];
      while (lineIndex < lines.length && /^\s*[-*]\s+/.test(lines[lineIndex])) {
        const item = lines[lineIndex].replace(/^\s*[-*]\s+/, '');
        items.push(<li key={nextKey()}>{inline(item, nextKey())}</li>);
        lineIndex += 1;
      }
      blocks.push(<ul key={nextKey()}>{items}</ul>);
      continue;
    }

    if (/^\s*\d+\.\s+/.test(line)) {
      const items: ReactNode[] = [];
      while (
        lineIndex < lines.length &&
        /^\s*\d+\.\s+/.test(lines[lineIndex])
      ) {
        const item = lines[lineIndex].replace(/^\s*\d+\.\s+/, '');
        items.push(<li key={nextKey()}>{inline(item, nextKey())}</li>);
        lineIndex += 1;
      }
      blocks.push(<ol key={nextKey()}>{items}</ol>);
      continue;
    }

    if (/^\s*>\s?/.test(line)) {
      const quote: string[] = [];
      while (lineIndex < lines.length && /^\s*>\s?/.test(lines[lineIndex])) {
        quote.push(lines[lineIndex].replace(/^\s*>\s?/, ''));
        lineIndex += 1;
      }
      blocks.push(
        <blockquote key={nextKey()}>
          <p>{inline(quote.join(' '), nextKey())}</p>
        </blockquote>,
      );
      continue;
    }

    if (/^\s*(?:---+|\*\*\*+)\s*$/.test(line)) {
      blocks.push(<hr key={nextKey()} />);
      lineIndex += 1;
      continue;
    }

    const paragraph: string[] = [];
    while (
      lineIndex < lines.length &&
      !/^\s*$/.test(lines[lineIndex]) &&
      !BLOCK_START.test(lines[lineIndex])
    ) {
      paragraph.push(lines[lineIndex++]);
    }
    if (paragraph.length === 0) {
      // Defensive: never loop on an unrecognized block marker.
      paragraph.push(lines[lineIndex++]);
    }
    blocks.push(<p key={nextKey()}>{inline(paragraph.join(' '), nextKey())}</p>);
  }

  return (
    <div className={className ? `markdown ${className}` : 'markdown'}>
      {blocks.length > 0 ? (
        blocks
      ) : (
        <p className="text-muted-foreground">No content.</p>
      )}
    </div>
  );
}

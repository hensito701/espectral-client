/* ==========================================================================
   MOTD renderer — Minecraft §-code colors, port of the espectral.es widget.
   Input lines use §x (or \u00a7x) codes; output is HTML with mc-* classes
   (defined in styles/base.css).
   ========================================================================== */

export const mcColors: Record<string, string> = {
  '0': '#000000',
  '1': '#0000aa',
  '2': '#00aa00',
  '3': '#00aaaa',
  '4': '#aa0000',
  '5': '#aa00aa',
  '6': '#ffaa00',
  '7': '#aaaaaa',
  '8': '#555555',
  '9': '#5555ff',
  a: '#55ff55',
  b: '#55ffff',
  c: '#ff5555',
  d: '#ff55ff',
  e: '#ffff55',
  f: '#ffffff',
};

export interface MotdSegment {
  text: string;
  color: string | null;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strikethrough: boolean;
}

export function parseMotd(raw: string): MotdSegment[] {
  const segments: MotdSegment[] = [];
  let text = '';
  let color: string | null = null;
  let bold = false;
  let italic = false;
  let underline = false;
  let strikethrough = false;

  const push = (): void => {
    if (text) segments.push({ text, color, bold, italic, underline, strikethrough });
    text = '';
  };

  for (let i = 0; i < raw.length; i += 1) {
    const ch = raw[i];
    if (ch !== '§' && ch !== '\u00a7') {
      text += ch;
      continue;
    }
    const code = raw[i + 1];
    if (code === undefined) {
      text += ch;
      break;
    }
    const c = code.toLowerCase();
    const isColor = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
    if (isColor) {
      push();
      color = c;
    } else if (c === 'l') {
      push();
      bold = true;
    } else if (c === 'o') {
      push();
      italic = true;
    } else if (c === 'n') {
      push();
      underline = true;
    } else if (c === 'm') {
      push();
      strikethrough = true;
    } else if (c === 'r') {
      push();
      color = null;
      bold = italic = underline = strikethrough = false;
    } else if (c === 'k') {
      // obfuscated — not styled
      push();
    } else {
      // Unknown code: keep both characters literally.
      text += ch + code;
      i += 1;
      continue;
    }
    i += 1;
  }
  push();
  return segments;
}

export function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/** Renders ONE line of MOTD text to HTML spans with mc-* classes. */
export function renderMotd(line: string): string {
  return parseMotd(line)
    .map((seg) => {
      const classes = ['mc'];
      if (seg.color) classes.push(`mc-${seg.color}`);
      if (seg.bold) classes.push('mc-bold');
      if (seg.italic) classes.push('mc-italic');
      if (seg.underline) classes.push('mc-underline');
      if (seg.strikethrough) classes.push('mc-strikethrough');
      return `<span class="${classes.join(' ')}">${escapeHtml(seg.text)}</span>`;
    })
    .join('');
}

/** Renders an array of MOTD lines, joined with <br>. */
export function renderMotdLines(lines: string[]): string {
  return lines.map(renderMotd).join('<br>');
}

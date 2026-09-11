import React, { useEffect, useRef, useState, useMemo } from 'react';
import { html } from './common.js';
import hljs from 'highlight.js/lib/core';

// Memoized because a large file is thousands of rows and the App re-renders
// on every drag frame.
// A row is { o, n, t, k }: old and new line index (null on the side the row
// does not exist), text, and the row kind ('', 'del', 'ins'). Without a diff
// every row exists only on the new side.
export const splitLines = (text) => { const l = text.split('\n'); if (l.length > 1 && l[l.length - 1] === '') l.pop(); return l; };
export function codeRows(src) {
  const newL = splitLines(src.text);
  if (!src.ops) return newL.map((t, i) => ({ o: null, n: i, t, k: '' }));
  const oldL = splitLines(src.old_text || '');
  const rows = [];
  for (const [tag, os, ol, ns, nl] of src.ops) {
    if (tag === '=') for (let i = 0; i < ol; i++) rows.push({ o: os + i, n: ns + i, t: newL[ns + i] ?? '', k: '' });
    else if (tag === '-') for (let i = 0; i < ol; i++) rows.push({ o: os + i, n: null, t: oldL[os + i] ?? '', k: 'del' });
    else for (let i = 0; i < nl; i++) rows.push({ o: null, n: ns + i, t: newL[ns + i] ?? '', k: 'ins' });
  }
  return rows;
}

// hljs has no toml grammar; ini reads close enough to stand in for it.
const EXT_LANG = {
  rs: 'rust', go: 'go', ts: 'typescript', tsx: 'typescript', js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript',
  py: 'python', toml: 'ini', json: 'json', md: 'markdown', html: 'xml', css: 'css', sh: 'bash', bash: 'bash', zsh: 'bash',
  yaml: 'yaml', yml: 'yaml', sql: 'sql', c: 'c', h: 'c', cpp: 'cpp', hpp: 'cpp', cc: 'cpp', java: 'java', rb: 'ruby',
  kt: 'kotlin', swift: 'swift', lua: 'lua',
};
const langForPath = (path) => { const m = /\.([^./]+)$/.exec(path || ''); return m ? EXT_LANG[m[1].toLowerCase()] : undefined; };

// Registered once per language, cached across every CodePane render (and
// across file switches) so re-opening a file never re-fetches its grammar.
const langLoads = new Map();
function ensureLanguage(lang) {
  if (!langLoads.has(lang)) {
    // A grammar that fails to load (offline, CDN gone) leaves the file plain.
    langLoads.set(lang, import(`highlight.js/lib/languages/${lang}`).then((mod) => {
      hljs.registerLanguage(lang, mod.default);
    }).catch((e) => console.warn(`highlight.js: no grammar for ${lang}:`, e.message)));
  }
  return langLoads.get(lang);
}

// hljs spans can cross the newlines we split on, so a span open at the end
// of one line must be closed there and reopened at the start of the next.
// Tracking a stack (rather than a single class) handles hljs's nesting,
// e.g. a string inside an interpolated expression.
export function splitHighlighted(text) {
  const lines = text.split('\n');
  if (lines.length > 1 && lines[lines.length - 1] === '') lines.pop();
  const tagRe = /<span class="([^"]*)">|<\/span>/g;
  const out = [];
  let stack = [];
  for (const line of lines) {
    let piece = stack.map((c) => `<span class="${c}">`).join('');
    let idx = 0, m;
    tagRe.lastIndex = 0;
    while ((m = tagRe.exec(line))) {
      piece += line.slice(idx, m.index);
      if (m[0] === '</span>') stack.pop();
      else stack.push(m[1]);
      piece += m[0];
      idx = tagRe.lastIndex;
    }
    piece += line.slice(idx) + '</span>'.repeat(stack.length);
    out.push(piece);
  }
  return out;
}

// null means "render plain text for this file": no language detected, its
// grammar hasn't loaded yet, or hljs choked on it.
function highlightLines(text, lang) {
  if (text == null || !lang || !hljs.getLanguage(lang)) return null;
  try {
    return splitHighlighted(hljs.highlight(text, { language: lang, ignoreIllegals: true }).value);
  } catch {
    return null;
  }
}

// The column a click landed on within a code cell, or null off the text.
// hljs wraps pieces of a line in nested spans and can cut a token across
// two of them, so the offset comes from the caret the browser places under
// the point, summed over the cell's text nodes, not from the span hit. The
// cell's text is the row's text verbatim (hljs only escapes), so the column
// indexes the row.
function caretColumn(td, x, y) {
  const pos = document.caretPositionFromPoint ? document.caretPositionFromPoint(x, y) : document.caretRangeFromPoint?.(x, y);
  const node = pos?.offsetNode ?? pos?.startContainer;
  if (!node || node.nodeType !== Node.TEXT_NODE || !td.contains(node)) return null;
  let col = pos.offset ?? pos.startOffset;
  const walk = document.createTreeWalker(td, NodeFilter.SHOW_TEXT);
  for (let t = walk.nextNode(); t && t !== node; t = walk.nextNode()) col += t.length;
  return col;
}

// `onClickAt(row, col)` is told where in the listing a click landed. It must
// be a stable callback: this component is memoized against per-frame App
// re-renders.
export const CodePane = React.memo(function CodePane({ src, line, range, side, onClose, onClickAt }) {
  const bodyRef = useRef(null);
  useEffect(() => {
    bodyRef.current?.querySelector('tr.hl')?.scrollIntoView({ block: 'center' });
  }, [src, line, side]);

  const lang = src && !src.error ? langForPath(src.path) : undefined;
  const [langVersion, setLangVersion] = useState(0);
  useEffect(() => {
    if (!lang || hljs.getLanguage(lang)) return;
    let live = true;
    ensureLanguage(lang).then(() => { if (live) setLangVersion((v) => v + 1); });
    return () => { live = false; };
  }, [lang]);

  const diff = !!src?.ops;
  // langVersion isn't read inside, but bumping it is what invalidates these
  // once ensureLanguage's dynamic import lands.
  const newLines = useMemo(() => src && highlightLines(src.text, lang), [src, lang, langVersion]);
  const oldLines = useMemo(() => diff && src ? highlightLines(src.old_text, lang) : null, [diff, src, lang, langVersion]);

  const close = html`<button class="btn" title="Close" onClick=${onClose}>×</button>`;
  if (!src) return html`<div class="code"><div class="code-head"><span class="meta">loading…</span>${close}</div></div>`;
  if (src.error) return html`<div class="code"><div class="code-head"><span class="err">${src.error}</span>${close}</div></div>`;
  const rows = codeRows(src);
  const at = (r) => (side === 'old' ? r.o : r.n);
  const cls = (r) => { const i = at(r); const h = i == null ? '' : i === line ? ' hl' : i >= range[0] && i <= range[1] ? ' in' : ''; return r.k + h; };
  const ins = rows.filter((r) => r.k === 'ins').length, del = rows.filter((r) => r.k === 'del').length;
  const ln = (i) => (i == null ? '' : i + 1);
  const lineHtml = (r) => (diff ? (r.n != null ? newLines?.[r.n] : oldLines?.[r.o]) : newLines?.[r.n]);
  // A drag-select ends in a click too; a selection left non-collapsed is how
  // the two are told apart.
  const onBodyClick = (e) => {
    if (!onClickAt || !window.getSelection()?.isCollapsed) return;
    const td = e.target.closest?.('td:not(.ln)');
    if (!td) return;
    const col = caretColumn(td, e.clientX, e.clientY);
    const row = rows[td.parentElement.sectionRowIndex];
    if (col != null && row) onClickAt(row, col);
  };
  return html`<div class="code">
    <div class="code-head"><b>${src.path}</b>
      ${diff && html`<span class="churn"><span class="add">+${ins}</span> <span class="del">−${del}</span></span>`}
      <span class="meta">${diff ? `${rows.length} rows` : `${rows.length} lines`} · at ${side === 'old' ? 'old ' : ''}${line + 1}</span>${close}</div>
    <div class="code-body" ref=${bodyRef}><table class=${diff ? 'diff' : ''}><tbody onClick=${onBodyClick}>
      ${rows.map((r, i) => { const hl = lineHtml(r); return html`<tr key=${i} class=${cls(r)}>${diff && html`<td class="ln o">${ln(r.o)}</td>`}<td class="ln">${ln(r.n)}</td>${hl != null ? html`<td dangerouslySetInnerHTML=${{ __html: hl }}></td>` : html`<td>${r.t}</td>`}</tr>`; })}
    </tbody></table></div>
  </div>`;
});

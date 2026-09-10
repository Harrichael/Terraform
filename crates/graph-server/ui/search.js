import { useState, useEffect, useMemo, useRef } from 'react';
import { html } from './common.js';

const MAX_ROWS = 50;

// The path minus the root component, which every entity shares.
const relPath = (n) => n.path.split('/').slice(1).join('/');

// Case-insensitive substring search over names and paths. Files whose name
// matches come first: a search is usually for a file, and symbols with the
// same name would otherwise bury it. Path-only hits come last.
export function search(graph, query, { showTests = true, hiddenIds = new Set() } = {}) {
  const q = query.trim().toLowerCase();
  if (!q || !graph) return { rows: [], more: 0 };
  const hidden = (n) => {
    if (!showTests && n.is_test) return true;
    for (let cur = n.id; cur != null; cur = graph.nodes[cur].parent) if (hiddenIds.has(cur)) return true;
    return false;
  };
  const hits = [];
  for (const n of graph.nodes) {
    if (n.parent == null) continue;
    const name = n.name.toLowerCase();
    const at = name.indexOf(q);
    const path = relPath(n);
    if (at < 0 && !path.toLowerCase().includes(q)) continue;
    if (hidden(n)) continue;
    const rank = at >= 0 ? (n.kind === 'file' ? 0 : 1) : 2;
    hits.push({ id: n.id, kind: n.kind, name: n.name, path, at, rank });
  }
  hits.sort((a, b) => a.rank - b.rank || a.name.length - b.name.length || a.path.localeCompare(b.path));
  return { rows: hits.slice(0, MAX_ROWS), more: Math.max(0, hits.length - MAX_ROWS) };
}

export function Search({ graph, showTests, hiddenIds, onPick }) {
  const [q, setQ] = useState('');
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef(null);
  const listRef = useRef(null);

  useEffect(() => {
    const onKey = (e) => {
      const tag = document.activeElement?.tagName;
      if (e.key === '/' && tag !== 'INPUT' && tag !== 'TEXTAREA' && !e.metaKey && !e.ctrlKey) { e.preventDefault(); inputRef.current?.focus(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const { rows, more } = useMemo(() => search(graph, q, { showTests, hiddenIds }), [graph, q, showTests, hiddenIds]);
  useEffect(() => { setCursor(0); }, [q]);
  useEffect(() => { listRef.current?.querySelector('.on')?.scrollIntoView({ block: 'nearest' }); }, [cursor]);

  const pick = (row) => { setOpen(false); onPick(row.id); };
  const onKeyDown = (e) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); setOpen(true); setCursor((c) => Math.min(rows.length - 1, c + 1)); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); setCursor((c) => Math.max(0, c - 1)); }
    else if (e.key === 'Enter') { if (rows[cursor]) pick(rows[cursor]); }
    else if (e.key === 'Escape') { setQ(''); setOpen(false); e.currentTarget.blur(); }
  };
  const mark = (r) => (r.at < 0 ? r.name : html`${r.name.slice(0, r.at)}<b>${r.name.slice(r.at, r.at + q.trim().length)}</b>${r.name.slice(r.at + q.trim().length)}`);

  return html`<div class="search">
    <input ref=${inputRef} type="search" placeholder="search files and symbols  /" value=${q}
      onInput=${(e) => { setQ(e.target.value); setOpen(true); }} onFocus=${() => setOpen(true)}
      onBlur=${() => setOpen(false)} onKeyDown=${onKeyDown} />
    ${open && q.trim() && html`<div class="search-list" ref=${listRef} onMouseDown=${(e) => e.preventDefault()}>
      ${rows.map((r, i) => html`<div key=${r.id} class=${'search-row' + (i === cursor ? ' on' : '')} onMouseEnter=${() => setCursor(i)} onClick=${() => pick(r)}>
        <span class="kind">${r.kind}</span><span class="name">${mark(r)}</span><span class="path">${r.path}</span>
      </div>`)}
      ${!rows.length && html`<div class="search-empty">no match</div>`}
      ${more > 0 && html`<div class="search-empty">${more} more…</div>`}
    </div>`}
  </div>`;
}

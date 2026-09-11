import React from 'react';
import htm from 'htm';

export const html = htm.bind(React.createElement);

export const EDGE_COLOR = { call: '#2f80ed', import: '#27ae60', type_ref: '#9b51e0', var_ref: '#f2994a', generic: '#8a8f98' };
export const edgeColor = (kind) => EDGE_COLOR[kind] || EDGE_COLOR.generic;
// Diff mode: how an entity or reference differs between the base tree and
// the working tree. `mixed` only occurs on coalesced edges.
export const CHANGE_COLOR = { same: '#b0b4bb', added: '#1a9c4a', removed: '#d64545', modified: '#e0a100', mixed: '#8a6fd1' };
export const changeColor = (s) => CHANGE_COLOR[s] || CHANGE_COLOR.same;
export const fmtChurn = (n) => (n.added || n.removed)
  ? html`<span class="churn">${n.added ? html`<span class="add">+${n.added}</span>` : ''}${n.added && n.removed ? ' ' : ''}${n.removed ? html`<span class="del">−${n.removed}</span>` : ''}</span>`
  : null;
// Plain-text twin of fmtChurn, for measuring labels.
export const churnText = (n) => [n.added ? `+${n.added}` : '', n.removed ? `−${n.removed}` : ''].filter(Boolean).join(' ');
export const statusClass = (n) => (n.status ? ' st-' + n.status : '') + (n.is_test ? ' is-test' : '');
export const NODE_H = 48;
export const estWidth = (name) => Math.min(320, Math.max(120, 8 * name.length + 40));
// Room on a leaf's right for its action buttons (+ and ×), so the name never
// runs under them; the CSS padding-right on .ent must match.
export const LEAF_ACTIONS_W = 34;
// A box must be wide enough for its label and the three action buttons. The
// text here mirrors what ContainerNode renders: name, size, then churn.
export const boxMinWidth = (data) => estWidth([data.name, fmtLoc(data), churnText(data)].filter(Boolean).join(' · ')) + 90;

// Node components are defined once, outside App, so they reach the current
// zoom handlers through this mutable slot instead of per-render node data.
export const actions = {
  zoomIn: () => {}, collapse: () => {}, prune: () => {}, hide: () => {},
  togglePopover: () => {}, closePopover: () => {}, setBundling: () => {}, resetBundling: () => {},
};

// Only folders and files get a size: for a function the line range already
// says it, and the label would just be noise.
export const fmtLoc = (n) => (n.loc > 0 && (n.kind === 'folder' || n.kind === 'file') ? `${n.loc.toLocaleString()} loc` : '');
export const leafRange = (n) => (n.kind !== 'folder' ? `${n.line_start + 1}–${n.line_end + 1}` : '');
// A leaf's sub-line as plain text, in the shape EntityNode renders it (the
// `· test` suffix is CSS). Kept next to the width estimate so the two cannot
// drift apart.
export const leafSubText = (n) => [n.kind, fmtLoc(n) || leafRange(n), churnText(n), n.is_test ? 'test' : ''].filter(Boolean).join(' · ');
// Whichever line is wider sets the leaf: the name in the node's semibold
// base font, or the sub-line at 10px, hence the two per-character widths.
// In diff mode the sub-line (`folder · 7,078 loc · +589 −51`) often wins.
export const leafWidth = (n) =>
  Math.min(320, Math.max(120, Math.max(8 * n.name.length, 5 * leafSubText(n).length) + 40)) + LEAF_ACTIONS_W;

// One edge per node pair, keeping the kind that says most about coupling. An
// import is a prerequisite of the call or type use it enables, and `generic`
// is mostly path qualifiers, so hiding them behind a stronger edge loses
// little; the panel still lists every kind.
export const PRECEDENCE = ['call', 'type_ref', 'var_ref', 'import', 'generic'];

export const ANIM_MS = 350;
export const PAD = 10, LABEL_H = 18;

async function readJson(res) {
  try { return await res.json(); } catch { return null; }
}
export async function fetchJson(url, init) {
  const res = await fetch(url, init);
  const body = await readJson(res);
  if (!res.ok) {
    const msg = body && body.error ? body.error : `${res.status} ${res.statusText}`.trim();
    throw Object.assign(new Error(msg), { status: res.status, body });
  }
  return body;
}

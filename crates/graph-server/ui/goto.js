// A click on an identifier in the code pane goes to what it refers to: the
// reference sited on that line, resolved to the drawn node that shows its
// target. No DOM here; code.js turns the click into a row and a column.

const IDENT = /[\p{L}\p{N}_]/u;
const isIdent = (ch) => ch != null && IDENT.test(ch);

// The identifier spanning column `col` of `text`, or null when the click was
// not on one. A caret sits between characters, so the one to its right is
// what was clicked, unless the caret is where a word ends.
export function tokenAt(text, col) {
  const i = isIdent(text[col]) ? col : isIdent(text[col - 1]) ? col - 1 : -1;
  if (i < 0) return null;
  let start = i, end = i + 1;
  while (isIdent(text[start - 1])) start--;
  while (isIdent(text[end])) end++;
  return { token: text.slice(start, end), start, end };
}

const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
// Whole-token occurrence: `focus` is not in `focused`.
const hasToken = (text, name) => !!name && new RegExp(`(?<![\\p{L}\\p{N}_])${escapeRe(name)}(?![\\p{L}\\p{N}_])`, 'u').test(text);

// The entities the identifier at `col` of `row` (a code.js row, { o, n, t })
// may refer to, from the references sited on that line inside `fileId`.
// The target's name has to be the clicked token; a target whose name does not
// occur on the line at all (an alias, a file imported by its stem) is the
// exception, reachable from any identifier on the line. Several ids come
// back only when the graph itself has several same-named targets there; the
// caller decides whether the view still tells them apart.
export function referenceTargets(graph, fileId, row, col) {
  const tok = tokenAt(row.t, col);
  if (!tok) return [];
  const fileOf = (id) => { for (let n = graph.nodes[id]; n; n = n.parent != null ? graph.nodes[n.parent] : null) if (n.kind === 'file') return n.id; return null; };
  // A removed reference's sites are lines of the old file (CONTRACT.md).
  const sited = graph.references.filter((r) => r.sites?.includes(r.status === 'removed' ? row.o : row.n) && fileOf(r.from) === fileId);
  const nameOf = (r) => graph.nodes[r.to]?.name;
  const named = sited.filter((r) => nameOf(r) === tok.token);
  const picked = named.length ? named : sited.filter((r) => !hasToken(row.t, nameOf(r)));
  return [...new Set(picked.map((r) => r.to))];
}

// The React Flow node showing entity `id`: itself when drawn (as a leaf or a
// box), else the nearest drawn ancestor it is folded into; null when nothing
// on its chain is drawn (hidden, or outside the scope).
export function nearestDrawn(graph, id, isDrawn) {
  for (let cur = id; cur != null; cur = graph.nodes[cur]?.parent) {
    if (isDrawn(String(cur))) return String(cur);
    if (isDrawn(`c${cur}`)) return `c${cur}`;
  }
  return null;
}

// The one node to select for a click, or null. Ambiguous targets still
// resolve when every one of them is folded into the same drawn node.
export function drawnTarget(graph, fileId, row, col, isDrawn) {
  const drawn = new Set(referenceTargets(graph, fileId, row, col).map((id) => nearestDrawn(graph, id, isDrawn)).filter((x) => x != null));
  return drawn.size === 1 ? [...drawn][0] : null;
}

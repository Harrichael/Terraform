import { Position, MarkerType } from '@xyflow/react';
import { PRECEDENCE, edgeColor, changeColor, estWidth, NODE_H, LEAF_ACTIONS_W } from './common.js';
import { bundleEnds } from './bundling.js';

export const rank = (kind) => { const i = PRECEDENCE.indexOf(kind); return i < 0 ? PRECEDENCE.length : i; };
export function collapseByPrecedence(edges) {
  const byPair = new Map();
  for (const e of edges) {
    const key = `${e.source}>${e.target}|${e.data.status || ''}`;
    const cur = byPair.get(key);
    if (!cur) { byPair.set(key, { ...e, data: { ...e.data, kinds: { [e.data.kind]: e.data.count }, members: e.data.members ? [...e.data.members] : undefined } }); continue; }
    cur.data.kinds[e.data.kind] = (cur.data.kinds[e.data.kind] || 0) + e.data.count;
    cur.data.count += e.data.count;
    if (cur.data.members && e.data.members) cur.data.members.push(...e.data.members);
    if (rank(e.data.kind) < rank(cur.data.kind)) {
      cur.data.kind = e.data.kind; cur.style = e.style; cur.markerEnd = e.markerEnd; cur.id = e.id;
    }
  }
  return [...byPair.values()].map((e) => ({ ...e, style: { ...e.style, strokeWidth: 1.5 + Math.min(e.data.count, 12) * 0.25 } }));
}

export function assignOffsets(edges) {
  const groups = new Map();
  for (const e of edges) {
    const key = e.source < e.target ? `${e.source}|${e.target}` : `${e.target}|${e.source}`;
    (groups.get(key) || groups.set(key, []).get(key)).push(e);
  }
  for (const g of groups.values()) {
    g.forEach((e, i) => {
      const sign = e.source < e.target ? 1 : -1;
      e.data.offset = sign * (i - (g.length - 1) / 2) * 10;
    });
  }
  return edges;
}

export function mkNode(ent, dir) {
  return {
    id: String(ent.id), type: 'entity', position: { x: 0, y: 0 }, data: ent,
    // Explicit dimensions make every node "initialized" up front. Without
    // them, onlyRenderVisibleElements leaves off-screen nodes unmeasured and
    // fitView (which skips unmeasured nodes) frames only what is on screen.
    width: estWidth(ent.name) + LEAF_ACTIONS_W, height: NODE_H,
    sourcePosition: dir === 'LR' ? Position.Right : Position.Bottom,
    targetPosition: dir === 'LR' ? Position.Left : Position.Top,
  };
}
// `status` is only set in diff mode; it is part of the id because an added
// and a removed reference between one pair are two distinct edges.
export function mkRefEdge(source, target, kind, count = 1, status = null, byChange = false) {
  const color = byChange && status ? changeColor(status) : edgeColor(kind);
  const dash = byChange && status === 'removed' ? { strokeDasharray: '6 4' } : {};
  return {
    id: `r-${source}-${target}-${kind}${status ? '-' + status : ''}`, source, target, type: 'ref',
    data: { kind, status, ref: true, offset: 0, count },
    style: { stroke: color, strokeWidth: 1.5 + Math.min(count, 12) * 0.25, ...dash },
    markerEnd: { type: MarkerType.ArrowClosed, color, width: 16, height: 16 },
  };
}
export function mkContainEdge(parent, child) {
  return {
    id: `c-${parent}-${child}`, source: String(parent), target: String(child), type: 'ref',
    data: { kind: 'contains', ref: false, offset: 0 }, style: { stroke: '#b0b4bb', strokeDasharray: '4 3' },
  };
}

// Raw mode ranks on containment only: ranking on references too makes dagre
// slow and the tree very wide, so references are drawn but not fed to dagre.
export function buildModel(graph, coalesced, view, dir, onePerPair, { byChange = false, changesOnly = false, hideTests = false, bundling = new Map(), scopeId = null, hiddenIds = new Set() } = {}) {
  const finish = (refs) => assignOffsets(onePerPair ? collapseByPrecedence(refs) : refs);
  const isDiff = !!graph.diff;
  const unchanged = (n) => isDiff && changesOnly && n.status === 'same';
  const hidden = (n) => hideTests && n.is_test;
  // An entity is user-hidden if it, or an ancestor, was explicitly hidden:
  // hiding a box is hiding its whole subtree in one click.
  const hiddenByUser = (id) => { for (let cur = id; cur != null; cur = graph.nodes[cur]?.parent) if (hiddenIds.has(cur)) return true; return false; };
  const dropped = (id) => hidden(graph.nodes[id] || {}) || hiddenByUser(id);
  // With tests hidden, a node's size is the non-test part of it.
  const shown = (n) => (hideTests && n.test_loc ? { ...n, loc: n.loc - n.test_loc } : n);
  const refEdge = (e, count) => mkRefEdge(String(e.from), String(e.to), e.kind, count, e.status, byChange);
  if (view === 'raw') {
    // Every ancestor of a changed entity is itself modified, and every
    // descendant of a test or user-hidden entity is itself dropped, so
    // dropping unchanged or dropped nodes never orphans a kept one.
    const kept = graph.nodes.filter((n) => !unchanged(n) && !dropped(n.id));
    const keptIds = new Set(kept.map((n) => n.id));
    const nodes = kept.map((n) => mkNode(shown(n), dir));
    const contain = kept.filter((n) => n.parent != null && keptIds.has(n.parent)).map((n) => mkContainEdge(n.parent, n.id));
    const refs = finish(graph.references.filter((e) => keptIds.has(e.from) && keptIds.has(e.to)).map((e) => refEdge(e, 1)));
    return { nodes, edges: [...contain, ...refs], rankEdges: contain, hiddenTests: graph.nodes.filter(hidden).length };
  }
  if (!coalesced) return { nodes: [], edges: [], rankEdges: [], hiddenTests: 0 };
  const inScope = (id) => {
    if (scopeId == null) return true;
    for (let cur = id; cur != null; cur = graph.nodes[cur].parent) if (cur === scopeId) return true;
    return false;
  };
  // Every ancestor of a leaf is an entity the user has zoomed into; draw it
  // as a container around its descendants. That includes the root, whose box
  // is the only way back to the single-node view. A scope keeps just one
  // container's subtree.
  const containers = new Map();
  const scopedLeaves = coalesced.leaves.filter((id) => graph.nodes[id] && inScope(id));
  const shownLeaves = scopedLeaves.filter((id) => !dropped(id));
  for (const id of shownLeaves) {
    for (let p = graph.nodes[id].parent; p != null && inScope(p); p = graph.nodes[p].parent) {
      containers.set(p, graph.nodes[p]);
    }
  }
  const boxId = (id) => (containers.has(id) ? `c${id}` : undefined);
  // Parents must precede children in the node list; walking ancestors from
  // the leaves builds the map deepest-first, so sort by depth.
  const depth = (n) => { let d = 0; for (let p = n.parent; p != null; p = graph.nodes[p].parent) d++; return d; };
  const boxes = [...containers.values()].sort((a, b) => depth(a) - depth(b)).map((ent) => ({
    id: `c${ent.id}`, type: 'container', position: { x: 0, y: 0 }, data: { ...shown(ent), bundling: bundling.get(ent.id) },
    parentId: boxId(ent.parent), width: 200, height: 100, selectable: true,
    className: unchanged(ent) ? 'faded' : undefined,
  }));
  const hasChildren = new Set(graph.nodes.filter((n) => n.parent != null && !dropped(n.id)).map((n) => n.parent));
  const leaves = shownLeaves.map((id) => graph.nodes[id])
    .map((n) => ({
      ...mkNode(shown(n), dir), parentId: boxId(n.parent),
      data: { ...shown(n), zoomable: hasChildren.has(n.id) },
      // A collapsed leaf is a whole subtree, so it is faded rather than
      // hidden; hiding it would change what its box means.
      className: unchanged(n) ? 'faded' : undefined,
    }));
  // Each leaf-to-leaf edge is lifted to the pair of siblings where the two
  // sides meet (a box and a node, or two boxes) and bundled per kind there,
  // so by default an edge is routed at one layout level instead of crossing
  // whatever lies between two distant leaves. The per-box levels in
  // `bundling` then pull either end back toward its leaf; bundling.js owns
  // that policy.
  const chain = (leaf) => {
    const out = [leaf];
    for (let p = graph.nodes[leaf].parent; p != null && containers.has(p); p = graph.nodes[p].parent) out.push(p);
    return out;
  };
  const drawnId = (ch, i) => (i === 0 ? String(ch[0]) : `c${ch[i]}`);
  // A coalesced edge between two shown leaves can still be carried partly by
  // test or user-hidden entities nested inside them (e.g. a file's `mod
  // tests` calling another file, or a hidden function deep in a still-shown
  // folder), so both filters apply to the underlying references, not just
  // the edge's own endpoints.
  const visibleRefs = (e) => {
    if (dropped(e.from) || dropped(e.to)) return null;
    const refs = e.refs.filter((i) => { const r = graph.references[i]; return r && !dropped(r.from) && !dropped(r.to); });
    return refs.length ? { ...e, refs } : null;
  };
  const bundles = new Map();
  for (const raw of coalesced.edges) {
    if (!inScope(raw.from) || !inScope(raw.to)) continue;
    const e = visibleRefs(raw);
    if (!e) continue;
    const cf = chain(e.from), ct = chain(e.to);
    const ends = bundleEnds(cf, ct, bundling);
    if (!ends) continue;
    const pair = [drawnId(cf, ends[0]), drawnId(ct, ends[1])];
    const key = `${pair[0]}|${pair[1]}|${e.kind}|${e.status || ''}`;
    const b = bundles.get(key) || bundles.set(key, { source: pair[0], target: pair[1], kind: e.kind, status: e.status, members: [] }).get(key);
    b.members.push(e);
  }
  const edges = finish([...bundles.values()].map((b) => {
    const edge = mkRefEdge(b.source, b.target, b.kind, b.members.length, b.status, byChange);
    edge.data.members = b.members;
    return edge;
  }));
  return { nodes: [...boxes, ...leaves], edges, rankEdges: edges, hiddenTests: scopedLeaves.filter((id) => hidden(graph.nodes[id])).length };
}

import { Position } from '@xyflow/react';
import dagre from '@dagrejs/dagre';
import { PAD, LABEL_H, boxMinWidth } from './common.js';

// Bottom-up layout: each container's direct children are laid out on their
// own, ranked only by the edges among them (an edge into another box is
// lifted to that box). The finished container is then a fixed-size node at
// the level above. Dagre's compound mode was tried first and ranks globally,
// so one edge to a far-away node stretched a whole box tall and thin.
export function layout(nodes, rankEdges, dir) {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const byParent = new Map();
  for (const n of nodes) {
    const key = n.parentId ?? null;
    (byParent.get(key) || byParent.set(key, []).get(key)).push(n);
  }
  // The child of `level` that contains `id`, or null when `id` is elsewhere.
  const liftTo = (id, level) => {
    let cur = byId.get(id);
    while (cur && (cur.parentId ?? null) !== level) cur = byId.get(cur.parentId);
    return cur ? cur.id : null;
  };

  const size = new Map(), pos = new Map(), dirOf = new Map();
  const run = (kids, edges, rankdir, spacing) => {
    const g = new dagre.graphlib.Graph();
    g.setGraph({ rankdir, marginx: 0, marginy: 0, ...spacing });
    g.setDefaultEdgeLabel(() => ({}));
    for (const k of kids) {
      const s = k.type === 'container' ? size.get(k.id) : { w: k.width, h: k.height };
      g.setNode(k.id, { width: s.w, height: s.h });
    }
    for (const [a, b] of edges) g.setEdge(a, b);
    dagre.layout(g);
    let w = 0, h = 0;
    for (const k of kids) { const p = g.node(k.id); w = Math.max(w, p.x + p.width / 2); h = Math.max(h, p.y + p.height / 2); }
    return { g, w, h };
  };
  const place = (level) => {
    const kids = byParent.get(level) || [];
    for (const k of kids) if (k.type === 'container') place(k.id);
    const seen = new Set(), edges = [];
    for (const e of rankEdges) {
      const a = liftTo(e.source, level), b = liftTo(e.target, level);
      if (!a || !b || a === b || seen.has(`${a}>${b}`)) continue;
      seen.add(`${a}>${b}`);
      edges.push([a, b]);
    }
    let best;
    if (level === null) {
      best = { ...run(kids, edges, dir, { nodesep: 36, ranksep: 64 }), rankdir: dir };
    } else {
      // A dependency chain inside a box makes dagre stack every node on its
      // own rank. Rather than let that dictate a tall thin box, lay the box
      // out both ways and keep the orientation nearest a wide, readable shape.
      const target = Math.log(1.6);
      for (const rankdir of ['TB', 'LR']) {
        const r = { ...run(kids, edges, rankdir, { nodesep: 28, ranksep: 52 }), rankdir };
        r.score = Math.abs(Math.log(r.w / r.h) - target);
        if (!best || r.score < best.score) best = r;
      }
    }
    dirOf.set(level, best.rankdir);
    const inset = level === null ? { x: 0, y: 0 } : { x: PAD, y: PAD + LABEL_H };
    for (const k of kids) {
      const p = best.g.node(k.id);
      pos.set(k.id, { x: p.x - p.width / 2 + inset.x, y: p.y - p.height / 2 + inset.y });
    }
    // Edges drawn between two direct children of this level take dagre's
    // routing, which already bends around the ranks in between.
    for (const e of rankEdges) {
      if ((byId.get(e.source)?.parentId ?? null) !== level || (byId.get(e.target)?.parentId ?? null) !== level) continue;
      const de = best.g.edge(e.source, e.target);
      if (de?.points) routedLocal.set(e.id, { level, points: de.points.map((p) => ({ x: p.x + inset.x, y: p.y + inset.y })) });
    }
    if (level !== null) {
      size.set(level, { w: Math.max(best.w + 2 * PAD, boxMinWidth(byId.get(level).data)), h: best.h + 2 * PAD + LABEL_H });
    }
  };
  const routedLocal = new Map();
  place(null);

  const absCache = new Map();
  const abs = (id) => {
    if (id == null) return { x: 0, y: 0 };
    if (!absCache.has(id)) {
      const p = pos.get(id), o = abs(byId.get(id).parentId ?? null);
      absCache.set(id, { x: p.x + o.x, y: p.y + o.y });
    }
    return absCache.get(id);
  };
  const routed = new Map();
  for (const [id, r] of routedLocal) {
    const o = abs(r.level);
    const e = rankEdges.find((x) => x.id === id);
    routed.set(id, {
      points: r.points.map((p) => ({ x: p.x + o.x, y: p.y + o.y })),
      at: { s: abs(e.source), t: abs(e.target) },
    });
  }

  const laid = nodes.map((n) => {
    const d = dirOf.get(n.parentId ?? null);
    const out = {
      ...n, position: pos.get(n.id),
      sourcePosition: d === 'LR' ? Position.Right : Position.Bottom,
      targetPosition: d === 'LR' ? Position.Left : Position.Top,
    };
    if (n.type === 'container') { const s = size.get(n.id); out.width = s.w; out.height = s.h; }
    return out;
  });
  return { nodes: laid, routed };
}

export function withRouting(edges, routed) {
  return edges.map((e) => {
    const r = routed.get(e.id);
    return r ? { ...e, data: { ...e.data, ...r } } : { ...e, data: { ...e.data, points: undefined, at: undefined } };
  });
}

// Layout that keeps the user's mental map. A fresh dagre run after every zoom
// reorders and re-ranks whole levels for one changed box, so the diagram
// yanked around on each click. Here only the inside of a newly opened box is
// laid out; the box then takes the place of the leaf it replaced, its
// siblings move only as far as its growth forces them, and every ancestor is
// refit around its children. Boxes that already existed keep their positions
// and orientation. The price is that upper levels drift from rank-optimal
// over many steps; Re-layout is the way back. A leaf new to a level that
// already existed (a file added while its folder is open) starts at that
// level's content origin and is pushed into place. Returns null when a node
// appears that has no place to inherit at all (a fresh box replacing
// nothing), which means a full layout is due.
const PUSH_GAP = 24;
export function incrementalLayout(prevNodes, prevEdges, nodes, rankEdges, edges, dir) {
  const prev = new Map(prevNodes.map((n) => [n.id, n]));
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const entityOf = (id) => id.replace(/^c/, '');
  const fresh = nodes.filter((n) => !prev.has(n.id));
  const newBoxes = fresh.filter((n) => n.type === 'container' && prev.has(entityOf(n.id)));
  const newBoxIds = new Set(newBoxes.map((n) => n.id));
  const enclosingNewBox = (n) => { for (let p = n.parentId; p; p = byId.get(p)?.parentId) if (newBoxIds.has(p)) return p; return null; };
  const hadLevel = (n) => (n.parentId ? prev.has(n.parentId) : prevNodes.some((p) => !p.parentId));
  for (const n of fresh) {
    if (newBoxIds.has(n.id) || enclosingNewBox(n)) continue;
    if (n.type !== 'container' && (prev.has(`c${n.id}`) || hadLevel(n))) continue;
    return null;
  }

  const state = new Map();
  for (const n of nodes) {
    const p = prev.get(n.id);
    if (p) state.set(n.id, { ...n, position: { ...p.position }, width: p.width, height: p.height, sourcePosition: p.sourcePosition, targetPosition: p.targetPosition });
  }
  const dirty = new Set();
  const centered = (n, old) => ({ x: old.position.x + (old.width - n.width) / 2, y: old.position.y + (old.height - n.height) / 2 });
  // A collapsed box becomes a leaf sitting where the box's centre was; a leaf
  // with nothing to replace takes its level's orientation from a sibling.
  for (const n of fresh) {
    if (n.type === 'container' || enclosingNewBox(n)) continue;
    const box = prev.get(`c${n.id}`);
    const like = box || prevNodes.find((p) => (p.parentId ?? null) === (n.parentId ?? null)) || n;
    const position = box ? centered(n, box) : n.parentId ? { x: PAD, y: LABEL_H + PAD } : { x: 0, y: 0 };
    state.set(n.id, { ...n, position, sourcePosition: like.sourcePosition, targetPosition: like.targetPosition });
    dirty.add(n.id);
  }
  // Each outermost new box is laid out on its own, as if it were the whole
  // graph, then dropped in over the leaf it replaced.
  const subRouted = [];
  for (const b of newBoxes.filter((b) => !enclosingNewBox(b))) {
    const under = (n) => { for (let p = n.parentId; p; p = byId.get(p)?.parentId) if (p === b.id) return true; return false; };
    const sub = layout([{ ...b, parentId: undefined }, ...nodes.filter(under)], rankEdges, dir);
    const laidBox = sub.nodes.find((n) => n.id === b.id);
    const leaf = prev.get(entityOf(b.id));
    const placed = { ...b, width: laidBox.width, height: laidBox.height, sourcePosition: leaf.sourcePosition, targetPosition: leaf.targetPosition };
    placed.position = centered(placed, leaf);
    state.set(b.id, placed);
    dirty.add(b.id);
    // The inside is already settled and routed; only the box is a change.
    for (const n of sub.nodes) if (n.id !== b.id) state.set(n.id, n);
    subRouted.push({ box: b.id, routed: sub.routed });
  }
  if (nodes.some((n) => !state.has(n.id))) return null;

  // Settle, deepest level first: push siblings off anything that changed,
  // then refit the level's box; a refit box is itself a change one level up.
  const depthOf = (id) => { let d = 0; for (let p = byId.get(id)?.parentId; p; p = byId.get(p)?.parentId) d++; return d; };
  const levels = [...new Set(nodes.map((n) => n.parentId ?? null))].sort((a, b) => (b == null ? -1 : depthOf(b)) - (a == null ? -1 : depthOf(a)));
  const rect = (n) => ({ x: n.position.x, y: n.position.y, r: n.position.x + n.width, b: n.position.y + n.height });
  for (const level of levels) {
    const kids = [...state.values()].filter((n) => (n.parentId ?? null) === level);
    const queue = kids.filter((k) => dirty.has(k.id));
    let budget = 20 * kids.length;
    while (queue.length && budget-- > 0) {
      const a = queue.shift(), ra = rect(a);
      for (const sib of kids) {
        if (sib === a) continue;
        const rs = rect(sib);
        const ox = Math.min(ra.r, rs.r) - Math.max(ra.x, rs.x) + PUSH_GAP, oy = Math.min(ra.b, rs.b) - Math.max(ra.y, rs.y) + PUSH_GAP;
        if (ox <= 0 || oy <= 0) continue;
        // Push along the axis on which the sibling already lies further out,
        // relative to the two sizes, so rows stay rows and columns columns.
        const dx = (rs.x + rs.r - ra.x - ra.r) / (a.width + sib.width), dy = (rs.y + rs.b - ra.y - ra.b) / (a.height + sib.height);
        if (Math.abs(dy) >= Math.abs(dx)) sib.position = { x: sib.position.x, y: sib.position.y + (dy >= 0 ? oy : -oy) };
        else sib.position = { x: sib.position.x + (dx >= 0 ? ox : -ox), y: sib.position.y };
        dirty.add(sib.id);
        queue.push(sib);
      }
    }
    if (level == null || !kids.length) continue;
    const box = state.get(level);
    const x0 = Math.min(...kids.map((k) => k.position.x)), y0 = Math.min(...kids.map((k) => k.position.y));
    const x1 = Math.max(...kids.map((k) => k.position.x + k.width)), y1 = Math.max(...kids.map((k) => k.position.y + k.height));
    const shift = { x: x0 - PAD, y: y0 - PAD - LABEL_H };
    const w = Math.max(x1 - x0 + 2 * PAD, boxMinWidth(box.data)), h = y1 - y0 + 2 * PAD + LABEL_H;
    if (shift.x || shift.y || w !== box.width || h !== box.height) {
      for (const k of kids) k.position = { x: k.position.x - shift.x, y: k.position.y - shift.y };
      box.position = { x: box.position.x + shift.x, y: box.position.y + shift.y };
      box.width = w; box.height = h;
      dirty.add(level);
    }
  }

  // Routes survive when both ends moved together (a level that shifted as a
  // whole); anything touching or crossing a changed node is re-routed.
  const finalNodes = nodes.map((n) => state.get(n.id));
  const absOld = absolutePositions(prevNodes), absNew = absolutePositions(finalNodes);
  const fromSub = new Map();
  for (const { box, routed } of subRouted) {
    const o = absNew(box);
    for (const [id, r] of routed) fromSub.set(id, { points: r.points.map((p) => ({ x: p.x + o.x, y: p.y + o.y })), at: { s: { x: r.at.s.x + o.x, y: r.at.s.y + o.y }, t: { x: r.at.t.x + o.x, y: r.at.t.y + o.y } } });
  }
  const carried = edges.map((e) => {
    if (fromSub.has(e.id)) return { ...e, data: { ...e.data, ...fromSub.get(e.id) } };
    const p = prevEdges.find((x) => x.id === e.id);
    if (!p?.data?.points || !prev.has(e.source) || !prev.has(e.target)) return { ...e, data: { ...e.data, points: undefined, at: undefined } };
    const ds = { x: absNew(e.source).x - absOld(e.source).x, y: absNew(e.source).y - absOld(e.source).y };
    const dt = { x: absNew(e.target).x - absOld(e.target).x, y: absNew(e.target).y - absOld(e.target).y };
    if (Math.abs(ds.x - dt.x) > 0.5 || Math.abs(ds.y - dt.y) > 0.5) return { ...e, data: { ...e.data, points: undefined, at: undefined } };
    const mv = (q) => ({ x: q.x + ds.x, y: q.y + ds.y });
    return { ...e, data: { ...e.data, points: p.data.points.map(mv), at: { s: mv(p.data.at.s), t: mv(p.data.at.t) } } };
  });
  return { nodes: finalNodes, edges: rerouteEdges(carried, finalNodes, dirty) };
}

// Drag invariant: a node stays inside its box, and boxes never overlap their
// siblings. Rather than clamping the node at the box edge, the box grows to
// keep containing it (and its parent grows in turn); the drag hits a wall
// only when a grown or moved box would intersect a sibling. Boxes never
// shrink during a drag; Re-layout recomputes them.
export function growContainers(state) {
  const depth = (n) => { let d = 0; for (let p = n.parentId; p; p = state.get(p)?.parentId) d++; return d; };
  const boxes = [...state.values()].filter((n) => n.type === 'container').sort((a, b) => depth(b) - depth(a));
  for (const c of boxes) {
    const kids = [...state.values()].filter((n) => n.parentId === c.id);
    if (!kids.length) continue;
    let left = 0, top = 0, right = c.width, bottom = c.height;
    for (const k of kids) {
      left = Math.min(left, k.position.x - PAD);
      top = Math.min(top, k.position.y - PAD - LABEL_H);
      right = Math.max(right, k.position.x + k.width + PAD);
      bottom = Math.max(bottom, k.position.y + k.height + PAD);
    }
    if (left < 0 || top < 0) {
      c.position = { x: c.position.x + left, y: c.position.y + top };
      for (const k of kids) k.position = { x: k.position.x - left, y: k.position.y - top };
    }
    c.width = right - left;
    c.height = bottom - top;
  }
}

export function violates(state, original) {
  const changed = (n) => {
    const o = original.get(n.id);
    return !o || o.position.x !== n.position.x || o.position.y !== n.position.y || o.width !== n.width || o.height !== n.height;
  };
  const groups = new Map();
  for (const n of state.values()) {
    const key = n.parentId ?? null;
    (groups.get(key) || groups.set(key, []).get(key)).push(n);
  }
  const eps = 0.5;
  for (const sib of groups.values()) {
    for (let i = 0; i < sib.length; i++) for (let j = i + 1; j < sib.length; j++) {
      const a = sib[i], b = sib[j];
      if (a.type !== 'container' && b.type !== 'container') continue;
      if (!changed(a) && !changed(b)) continue;
      const apart = a.position.x + a.width <= b.position.x + eps || b.position.x + b.width <= a.position.x + eps
        || a.position.y + a.height <= b.position.y + eps || b.position.y + b.height <= a.position.y + eps;
      if (!apart) return true;
    }
  }
  return false;
}

// Applies proposed drag positions, letting boxes grow, and scales the move
// back along each axis separately when it would breach a wall, so a node
// dragged diagonally into a wall slides along it.
export function resolveDrag(nodes, proposals) {
  const original = new Map(nodes.map((n) => [n.id, n]));
  const attempt = (tx, ty) => {
    const state = new Map(nodes.map((n) => [n.id, { ...n, position: { ...n.position } }]));
    for (const [id, target] of proposals) {
      const n = state.get(id);
      if (n) n.position = { x: n.position.x + (target.x - n.position.x) * tx, y: n.position.y + (target.y - n.position.y) * ty };
    }
    growContainers(state);
    return state;
  };
  const ok = (s) => !violates(s, original);
  let state = attempt(1, 1);
  if (!ok(state)) {
    const maxT = (f) => { let lo = 0, hi = 1; for (let i = 0; i < 10; i++) { const m = (lo + hi) / 2; if (ok(f(m))) lo = m; else hi = m; } return lo; };
    const tx = maxT((t) => attempt(t, 0)), ty = maxT((t) => attempt(0, t));
    state = attempt(tx, ty);
    if (!ok(state)) state = tx >= ty ? attempt(tx, 0) : attempt(0, ty);
  }
  return nodes.map((n) => state.get(n.id));
}

// Post-move edge routing. Dagre's points only describe the layout it
// produced; once a node is dragged, the edges around it need a new route.
// This finds the taut shortest path around the inflated sibling rectangles
// at the edge's level (a visibility graph over their corners, nested boxes
// opaque), then smooths it exactly like the dagre routes so the two are
// indistinguishable in style: a curve that bends around obstacles and meets
// each node straight on through a short perpendicular stub.
const ROUTE_GAP = 14;
export function routeEdge(edge, nodes, abs) {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const s = byId.get(edge.source), t = byId.get(edge.target);
  if (!s || !t || (s.parentId ?? null) !== (t.parentId ?? null)) return null;
  const level = s.parentId ?? null;
  const rectOf = (n) => { const a = abs(n.id); return { x: a.x, y: a.y, r: a.x + n.width, b: a.y + n.height }; };
  const grow = (o, m) => ({ x: o.x - m, y: o.y - m, r: o.r + m, b: o.b + m });
  const handle = (o, pos) => pos === Position.Bottom ? { x: (o.x + o.r) / 2, y: o.b }
    : pos === Position.Top ? { x: (o.x + o.r) / 2, y: o.y }
    : pos === Position.Right ? { x: o.r, y: (o.y + o.b) / 2 } : { x: o.x, y: (o.y + o.b) / 2 };
  const step = (p, pos, d) => pos === Position.Bottom ? { x: p.x, y: p.y + d } : pos === Position.Top ? { x: p.x, y: p.y - d }
    : pos === Position.Right ? { x: p.x + d, y: p.y } : { x: p.x - d, y: p.y };
  const sr = rectOf(s), tr = rectOf(t);
  const sh = handle(sr, s.sourcePosition), th = handle(tr, t.targetPosition);
  const sp = step(sh, s.sourcePosition, ROUTE_GAP), tp = step(th, t.targetPosition, ROUTE_GAP);

  const region = grow({ x: Math.min(sr.x, tr.x, sp.x, tp.x), y: Math.min(sr.y, tr.y, sp.y, tp.y), r: Math.max(sr.r, tr.r, sp.x, tp.x), b: Math.max(sr.b, tr.b, sp.y, tp.y) }, 80);
  const hits = (a, o) => a.x < o.r && o.x < a.r && a.y < o.b && o.y < a.b;
  const obstacles = nodes
    .filter((n) => (n.parentId ?? null) === level && n.id !== s.id && n.id !== t.id)
    .map(rectOf).filter((o) => hits(o, region)).map((o) => grow(o, ROUTE_GAP - 2));
  obstacles.push(grow(sr, ROUTE_GAP - 4), grow(tr, ROUTE_GAP - 4));
  if (obstacles.length > 60) return null;

  // Liang–Barsky style test: does the open segment enter a rectangle's interior?
  const crosses = (p, q, o) => {
    let t0 = 0, t1 = 1;
    const dx = q.x - p.x, dy = q.y - p.y;
    const clip = (num, den) => {
      if (den === 0) return num <= 0;
      const r = num / den;
      if (den < 0) { if (r > t1) return false; if (r > t0) t0 = r; } else { if (r < t0) return false; if (r < t1) t1 = r; }
      return true;
    };
    const e = 0.01;
    return clip(p.x - (o.x + e), -dx) && clip((o.r - e) - p.x, dx) && clip(p.y - (o.y + e), -dy) && clip((o.b - e) - p.y, dy) && t0 < t1;
  };
  const inside = (pt) => obstacles.some((o) => pt.x > o.x && pt.x < o.r && pt.y > o.y && pt.y < o.b);
  if (inside(sp) || inside(tp)) return null;
  const verts = [sp, tp];
  for (const o of obstacles) {
    for (const c of [{ x: o.x, y: o.y }, { x: o.r, y: o.y }, { x: o.x, y: o.b }, { x: o.r, y: o.b }]) if (!inside(c)) verts.push(c);
  }
  const visible = (a, b) => !obstacles.some((o) => crosses(a, b, o));

  // A* over the visibility graph with Euclidean lengths.
  const dist = (a, b) => Math.hypot(a.x - b.x, a.y - b.y);
  const g = new Array(verts.length).fill(Infinity), prev = new Array(verts.length).fill(-1), closed = new Set();
  g[0] = 0;
  const open = [0];
  let found = false;
  while (open.length) {
    let bi = 0;
    for (let k = 1; k < open.length; k++) if (g[open[k]] + dist(verts[open[k]], tp) < g[open[bi]] + dist(verts[open[bi]], tp)) bi = k;
    const u = open.splice(bi, 1)[0];
    if (closed.has(u)) continue;
    closed.add(u);
    if (u === 1) { found = true; break; }
    for (let v = 0; v < verts.length; v++) {
      if (closed.has(v) || !visible(verts[u], verts[v])) continue;
      const cand = g[u] + dist(verts[u], verts[v]);
      if (cand < g[v]) { g[v] = cand; prev[v] = u; open.push(v); }
    }
  }
  if (!found) return null;
  const taut = [];
  for (let v = 1; v !== -1; v = prev[v]) taut.unshift(verts[v]);

  // Subdivide long legs so the midpoint smoothing rounds only the corners
  // (radius about half a piece) and leaves straight runs straight.
  const pts = [sh];
  const chain = [sp, ...taut.slice(1, -1), tp];
  for (let i = 0; i < chain.length; i++) {
    const a = i === 0 ? sh : chain[i - 1], b = chain[i];
    const n = Math.max(1, Math.round(dist(a, b) / 40));
    for (let k = 1; k <= n; k++) pts.push({ x: a.x + (b.x - a.x) * k / n, y: a.y + (b.y - a.y) * k / n });
  }
  pts.push(th);
  return pts;
}

// Re-route every routable edge that touches a dirty node or whose current
// path runs through one; everything else keeps its route.
export function rerouteEdges(edges, nodes, dirty) {
  const abs = absolutePositions(nodes);
  const byId = new Map(nodes.map((n) => [n.id, n]));
  // A changed node is an obstacle only to edges drawn at its own level: the
  // edges inside a box that grew still fit the box.
  const dirtyRects = new Map();
  for (const id of dirty) {
    const n = byId.get(id);
    if (!n) continue;
    const a = abs(n.id), level = n.parentId ?? null;
    (dirtyRects.get(level) || dirtyRects.set(level, []).get(level)).push({ x: a.x, y: a.y, r: a.x + n.width, b: a.y + n.height });
  }
  const crosses = (pts, level) => pts && pts.some((p, i) => i > 0 && (dirtyRects.get(level) || []).some((o) => {
    const q = pts[i - 1];
    return Math.min(p.x, q.x) < o.r && Math.max(p.x, q.x) > o.x && Math.min(p.y, q.y) < o.b && Math.max(p.y, q.y) > o.y;
  }));
  return edges.map((e) => {
    if (!dirty.has(e.source) && !dirty.has(e.target) && !crosses(e.data.points, byId.get(e.source)?.parentId ?? null)) return e;
    const points = routeEdge(e, nodes, abs);
    if (!points) return { ...e, data: { ...e.data, points: undefined, at: undefined } };
    return { ...e, data: { ...e.data, points, at: { s: abs(e.source), t: abs(e.target) } } };
  });
}

// Absolute (flow-space) top-left of a node whose position is relative to its
// parent box, as React Flow stores it.
export function absolutePositions(nodes) {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const cache = new Map();
  const abs = (id) => {
    if (id == null) return { x: 0, y: 0 };
    if (!cache.has(id)) {
      const n = byId.get(id), o = abs(n.parentId ?? null);
      cache.set(id, { x: n.position.x + o.x, y: n.position.y + o.y });
    }
    return cache.get(id);
  };
  return abs;
}

export const lerp = (a, b, t) => a + (b - a) * t;
export const easeInOut = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

// One frame of the old→new interpolation. Nodes that did not exist before
// appear in place; routed edges interpolate their points only when the same
// edge survived with the same shape, otherwise they draw as beziers until
// the animation settles.
export function interpolate(prevNodes, prevEdges, finalNodes, finalEdges, t) {
  const nodes = finalNodes.map((n) => {
    const p = prevNodes.get(n.id);
    if (!p) return n;
    const out = { ...n, position: { x: lerp(p.position.x, n.position.x, t), y: lerp(p.position.y, n.position.y, t) } };
    if (n.type === 'container' && p.width) { out.width = lerp(p.width, n.width, t); out.height = lerp(p.height, n.height, t); }
    return out;
  });
  const edges = finalEdges.map((e) => {
    if (!e.data.points) return e;
    const p = prevEdges.get(e.id);
    if (!p?.data?.points || p.data.points.length !== e.data.points.length) return { ...e, data: { ...e.data, points: undefined } };
    const pt = (a, b) => ({ x: lerp(a.x, b.x, t), y: lerp(a.y, b.y, t) });
    return { ...e, data: { ...e.data, points: e.data.points.map((q, i) => pt(p.data.points[i], q)), at: { s: pt(p.data.at.s, e.data.at.s), t: pt(p.data.at.t, e.data.at.t) } } };
  });
  return { nodes, edges };
}

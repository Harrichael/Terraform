// Where the two ends of a bundled edge are drawn. A leaf-to-leaf edge is
// lifted to the level where the two sides are siblings (the meeting level),
// then each end may be pulled back down toward its leaf by the per-box
// levels the user set. No graph here: a side is its ancestor chain of
// entity ids, leaf first, and `bundling` maps a box's entity id to
// { in, out, inward }, each 0 (off), 1 (one level) or 2 (recursive).
//
// `in` and `out` on a box say how far an edge crossing that box's border
// reaches inside it; `inward` on a box says how far an edge ending inside one
// of its sub boxes reaches into that sub box. Level 1 is one level, deliberately
// narrow: it reveals a box's immediate children and nothing below them.
//
// Rules compose and are position-independent: an end attaches at the deepest
// point any rule reaches, and a rule on a deep box fires even when every box
// between it and the meeting level is Off. The alternative, a top-down walk
// that stops at the first box granting nothing, made a setting on an inner
// box silently depend on every outer box also being set.

const level = (bundling, ent, key) => bundling.get(ent)?.[key] || 0;

// Index in `ch` where the end attaches. `k` is the meeting index; `dir` is
// 'out' for the source side and 'in' for the target side. Every box from
// ch[1] up to ch[k] has the edge crossing its border, so its own level
// applies; the box above it, ch[i+1], sees the edge ending inside its sub box
// ch[i], so its `inward` applies. Either lets the end step into ch[i], to
// ch[i-1]; level 2 goes all the way to the leaf. Boxes above ch[k+1] contain
// the whole edge and have no say.
export function attachIndex(ch, k, dir, bundling) {
  let at = k;
  for (let i = 1; i <= k; i++) {
    const reach = Math.max(level(bundling, ch[i], dir), i + 1 < ch.length ? level(bundling, ch[i + 1], 'inward') : 0);
    if (reach === 2) return 0;
    if (reach === 1) at = Math.min(at, i - 1);
  }
  return at;
}

// [source index into cf, target index into ct], or null when the chains never
// diverge (an edge from a leaf to itself). Both chains end at the same
// outermost drawn box, so the meeting pair is the first elements below the
// shared tail.
export function bundleEnds(cf, ct, bundling) {
  let ia = cf.length - 1, ib = ct.length - 1;
  while (ia >= 0 && ib >= 0 && cf[ia] === ct[ib]) { ia--; ib--; }
  if (ia < 0 || ib < 0) return null;
  return [attachIndex(cf, ia, 'out', bundling), attachIndex(ct, ib, 'in', bundling)];
}

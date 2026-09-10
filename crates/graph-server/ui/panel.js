import { Fragment, useState, useEffect } from 'react';
import { html, edgeColor, changeColor, fmtChurn, CHANGE_COLOR, EDGE_COLOR } from './common.js';
import { splitLines } from './code.js';

// Below the "onion" threshold every group starts open, since there is little
// to gain from hiding a handful of rows; above it only the top level does, so
// a big reference list does not open as one wall of text.
const OPEN_THRESHOLD = 15;
const MAX_SOURCE_FETCHES = 40;

export function Inspector({ graph, nodes, edges, view, coalesced, sel, setSelectedId, showTests, byChange, isDiff, panelError, openCode, sideOf, fileOf, isUnder, sources, loadSource, hiddenIds }) {
  const selectedId = sel?.type === 'node' ? sel.id : null;
  const selectedNode = selectedId != null ? nodes.find((n) => n.id === selectedId) : null;
  const selected = selectedNode ? selectedNode.data : null;
  const isBox = selectedNode?.type === 'container';
  const nameOf = (id) => graph?.nodes[id]?.name ?? `#${id}`;
  const hiddenEnt = (id) => !showTests && !!graph?.nodes[id]?.is_test;
  // An entity the user hid, or anything nested under one, drops out of the
  // panel the same way a hidden test does: it is not part of the story the
  // diagram is telling right now.
  const underHidden = (id) => {
    if (!hiddenIds || !hiddenIds.size) return false;
    for (let cur = id; cur != null; cur = graph.nodes[cur]?.parent) if (hiddenIds.has(cur)) return true;
    return false;
  };
  const dropEnt = (id) => hiddenEnt(id) || underHidden(id);
  const visibleRefs = (refs) => refs.filter((r) => !dropEnt(r.from) && !dropEnt(r.to));

  const labelOf = (nodeId) => nodes.find((n) => n.id === nodeId)?.data.name ?? nameOf(Number(nodeId));
  const children = selected && (view === 'raw' || isBox) ? graph.nodes.filter((n) => n.parent === selected.id && !hiddenEnt(n.id)) : [];
  const locOf = (n) => (n.loc > 0 ? html`<span class="loc">${n.loc.toLocaleString()} loc</span>` : '');
  const fileName = (id) => { const f = fileOf(id); return f == null ? '?' : graph.nodes[f].name; };
  const openAt = (id) => { const e = graph.nodes[id]; openCode(id, e.line_start, [e.line_start, e.line_end]); };
  // The nearest drawn node containing an entity, since in the coalesced view
  // the entity itself may sit inside a collapsed leaf.
  const selectEntity = (id) => {
    for (let cur = id; cur != null; cur = graph.nodes[cur].parent) {
      const rf = nodes.find((n) => n.id === String(cur) || n.id === `c${cur}`);
      if (rf) { setSelectedId(rf.id); return; }
    }
  };

  // A node's outgoing/incoming references. Raw view reads graph.references
  // directly. In the coalesced view the server's leaf-level edges are the
  // truth for a leaf or a box alike: everything from inside the selected
  // subtree to outside it. The drawn bundles are not used, since bundling
  // settings lift edges to enclosing boxes and would make a function look
  // like it calls three things when it calls ten.
  const gatherRefs = (direction) => {
    if (!selected) return [];
    const field = direction === 'outgoing' ? 'from' : 'to', other = direction === 'outgoing' ? 'to' : 'from';
    if (view === 'raw') return (graph.references || []).filter((r) => r[field] === selected.id);
    const inside = (id) => id === selected.id || isUnder(id, selected.id);
    const idx = new Set();
    for (const e of coalesced?.edges || []) if (inside(e[field]) && !inside(e[other])) for (const i of e.refs || []) idx.add(i);
    return [...idx].sort((a, b) => a - b).map((i) => graph.references[i]).filter(Boolean);
  };

  // An edge stands for every raw reference between its two subtrees, of the
  // kinds it carries. Coalescing drops references whose endpoint is an
  // expanded entity, so this list can be longer than the edge's count.
  const selectedEdge = sel?.type === 'edge' ? edges.find((e) => e.id === sel.id) : null;
  const entityOf = (rfId) => Number(String(rfId).replace(/^c/, ''));
  const underOrSelf = (id, anc) => id === anc || isUnder(id, anc);
  const edgeRefs = !selectedEdge ? [] : (() => {
    // Coalesced edges name the raw references they stand for, which is exact;
    // the subtree derivation below is the fallback for raw-view edges.
    const members = selectedEdge.data.members;
    if (members && members.every((m) => m.refs)) {
      const idx = [...new Set(members.flatMap((m) => m.refs))].sort((a, b) => a - b);
      return idx.map((i) => graph.references[i]).filter(Boolean);
    }
    const s = entityOf(selectedEdge.source), t = entityOf(selectedEdge.target);
    const kinds = new Set(selectedEdge.data.kinds ? Object.keys(selectedEdge.data.kinds) : [selectedEdge.data.kind]);
    const st = selectedEdge.data.status;
    return graph.references.filter((r) => kinds.has(r.kind) && (!st || r.status === st) && underOrSelf(r.from, s) && underOrSelf(r.to, t));
  })();

  // --- Onion grouping ----------------------------------------------------
  // Groups sit between the selected entity and the other endpoint of a
  // reference: cut the other endpoint's ancestor chain at the entity it
  // shares with the selected one (or the edge's source, for an edge), and
  // what's left is the path of folders/files/etc. to build a tree from.
  const ancestorChain = (id) => { const c = []; for (let cur = id; cur != null; cur = graph.nodes[cur]?.parent) c.unshift(cur); return c; };
  const buildGroupTree = (pairs, refPointId) => {
    const refChain = ancestorChain(refPointId);
    const root = { id: null, children: new Map(), refs: null, count: 0 };
    for (const { groupId, ref } of pairs) {
      const chain = ancestorChain(groupId);
      let cut = 0;
      while (cut < refChain.length && cut < chain.length && refChain[cut] === chain[cut]) cut++;
      const path = chain.slice(cut);
      let node = root;
      node.count++;
      for (const id of path) {
        if (!node.children.has(id)) node.children.set(id, { id, children: new Map(), refs: null, count: 0 });
        node = node.children.get(id);
        node.count++;
      }
      (node.refs || (node.refs = [])).push(ref);
    }
    return root;
  };
  const sortedChildren = (m) => [...m.values()].sort((a, b) => nameOf(a.id).localeCompare(nameOf(b.id)));

  const [expanded, setExpanded] = useState(() => new Map());
  // Shared across every tree rendered this pass: a simple global budget
  // keeps one section (e.g. Outgoing) from starving the others out of source
  // fetches when a node has a lot of references.
  const neededFiles = new Set();
  // Split once per file and side per render: the App re-renders on every
  // drag frame, and a site list can name one large file many times.
  const linesCache = new Map();
  const linesOf = (fid, text) => {
    const key = `${fid}:${text.length}`;
    if (!linesCache.has(key)) linesCache.set(key, splitLines(text));
    return linesCache.get(key);
  };

  // Renders one direction's (or one edge's) grouped reference tree. `ctx`
  // namespaces the expand/collapse state so Outgoing, Incoming and an edge's
  // own tree don't fight over the same keys; `rowKeyOf` picks, out of the
  // refs collected at a group's leaf, which entity each row is about (the
  // other endpoint for a node direction, the to-entity for an edge, where
  // grouping is by from-entity).
  // `groupIsRow` is true for a node's Outgoing/Incoming tree, where the group
  // key IS the row's own entity (its name already appears on the row button),
  // and false for an edge's tree, where groups are keyed by from-entity but
  // rows are keyed by to-entity -- there the leaf's own name is the only
  // place its identity appears, so it must not be dropped even when nothing
  // needed collapsing above it.
  const renderTree = (root, ctx, rowKeyOf, groupIsRow) => {
    const total = root.count;
    const isOpen = (id, depth) => {
      const key = `${ctx}:${id}`;
      return expanded.has(key) ? expanded.get(key) : (total <= OPEN_THRESHOLD || depth === 0);
    };
    // Read `open` synchronously: by the time a setState updater runs, React
    // has already nulled the synthetic event's currentTarget.
    const toggle = (id) => (e) => { const open = e.currentTarget.open; setExpanded((m) => new Map(m).set(`${ctx}:${id}`, open)); };

    const renderSite = (r, line) => {
      const fid = fileOf(r.from);
      // A removed reference only ever existed on the old side; so did a
      // reference from an entity that no longer exists at all.
      const removed = r.status === 'removed' || graph.nodes[r.from]?.status === 'removed';
      if (fid != null) neededFiles.add(fid);
      const src = fid != null ? sources?.get(fid) : null;
      const raw = src && !src.error ? (removed ? src.old_text : src.text) : null;
      const text = raw != null ? (linesOf(fid, raw)[line] ?? '').trim() : null;
      return html`<div class="site-line" key=${`${r.from}:${line}:${r.kind}`}>
        <button class="site" title="Open the reference site" onClick=${() => openCode(r.from, line, [line, line], removed ? 'old' : sideOf(r.from))}>${fileName(r.from)}:${line + 1}</button>
        ${text ? html`<span class="site-text" title=${text}>${text}</span>` : ''}
      </div>`;
    };
    // Two references of different kinds often share one site (a call and the
    // type it names); the line is shown once.
    const sitesOf = (refs) => {
      const seen = new Set(), out = [];
      for (const r of refs) for (const line of r.sites || []) { const k = `${r.from}:${line}`; if (!seen.has(k)) { seen.add(k); out.push([r, line]); } }
      return out;
    };

    const renderRow = (rowKey, refs) => {
      const kinds = new Map(); const statuses = new Set();
      for (const r of refs) {
        kinds.set(r.kind, (kinds.get(r.kind) || 0) + 1);
        if (r.status && r.status !== 'same') statuses.add(r.status);
      }
      return html`<div class="ref-row" key=${rowKey}>
        <div class="row-head">
          ${[...kinds].map(([k, c]) => html`<span key=${k} class="tag" style=${{ background: edgeColor(k) }}>${k}${c > 1 ? ` ×${c}` : ''}</span>`)}
          ${[...statuses].map((s) => html`<span key=${s} class="tag" style=${{ background: changeColor(s) }}>${s}</span>`)}
          <button onClick=${() => selectEntity(rowKey)}>${nameOf(rowKey)}</button>
          <button class="site" title="Open the definition" onClick=${() => openAt(rowKey)}>def ${fileName(rowKey)}:${(graph.nodes[rowKey]?.line_start ?? 0) + 1}</button>
        </div>
        ${sitesOf(refs).map(([r, line]) => renderSite(r, line))}
        ${!sitesOf(refs).length ? html`<div class="meta">(no site recorded)</div>` : ''}
      </div>`;
    };
    const renderRows = (refs) => {
      const byKey = new Map();
      for (const r of refs) { const k = rowKeyOf(r); (byKey.get(k) || byKey.set(k, []).get(k)).push(r); }
      return [...byKey.entries()].sort((a, b) => nameOf(a[0]).localeCompare(nameOf(b[0]))).map(([rk, rrefs]) => renderRow(rk, rrefs));
    };

    // Walk past any run of single-child, ref-less ancestors so the tree
    // reads as `a/b/c (3)` instead of one node per level.
    const renderNode = (node, prefix, depth) => {
      let cur = node, path = prefix;
      while (!cur.refs && cur.children.size === 1) {
        const [, child] = [...cur.children][0];
        path = [...path, nameOf(cur.id)];
        cur = child;
      }
      if (cur.refs && cur.children.size === 0) {
        const leafPath = groupIsRow ? path : [...path, nameOf(cur.id)];
        return html`<div class="ref-leaf" key=${cur.id}>
          ${leafPath.length ? html`<div class="group-path">${leafPath.join('/')}</div>` : ''}
          ${renderRows(cur.refs)}
        </div>`;
      }
      const label = [...path, nameOf(cur.id)].join('/');
      const open = isOpen(cur.id, depth);
      return html`<details key=${cur.id} open=${open} onToggle=${toggle(cur.id)}>
        <summary>${label} <span class="grp-count">(${cur.count})</span></summary>
        ${open && html`<div class="grp-body">
          ${cur.refs ? renderRows(cur.refs) : ''}
          ${sortedChildren(cur.children).map((c) => renderNode(c, [], depth + 1))}
        <//>`}
      <//>`;
    };

    return html`<div class="ref-tree">
      ${root.refs ? renderRows(root.refs) : ''}
      ${sortedChildren(root.children).map((c) => renderNode(c, [], 0))}
    </div>`;
  };

  useEffect(() => {
    if (!loadSource) return;
    [...neededFiles].slice(0, MAX_SOURCE_FETCHES).forEach((id) => loadSource(id));
  });

  const otherOf = (r, direction) => (direction === 'outgoing' ? r.to : r.from);
  const direction = (dir) => {
    const refs = selected ? visibleRefs(gatherRefs(dir)) : [];
    const tree = buildGroupTree(refs.map((r) => ({ groupId: otherOf(r, dir), ref: r })), selected?.id);
    return { refs, tree, rowKeyOf: (r) => otherOf(r, dir) };
  };
  const out = direction('outgoing');
  const inc = direction('incoming');
  const edgeVisibleRefs = visibleRefs(edgeRefs);
  const edgeTree = selectedEdge ? buildGroupTree(edgeVisibleRefs.map((r) => ({ groupId: r.from, ref: r })), entityOf(selectedEdge.source)) : null;

  const dirSection = (title, d, ctx) => html`<h3>${title} (${d.refs.length})</h3>
    ${d.refs.length ? renderTree(d.tree, ctx, d.rowKeyOf, true) : html`<div class="empty">none</div>`}`;

  return html`<${Fragment}>
    ${selected ? html`
      <h2>${selected.name}${selected.status && html`<span class="pill" style=${{ background: changeColor(selected.status) }}>${selected.status}</span>`}</h2>
      <div class="meta">${selected.path}</div>
      <div class="meta">${selected.kind}${selected.kind !== 'folder' ? ` · lines ${selected.line_start + 1}–${selected.line_end + 1}` : ''}${selected.loc > 0 ? ` · ${selected.loc.toLocaleString()} loc` : ''} · id ${selected.id}</div>
      ${fmtChurn(selected) && html`<div class="meta">changed lines: ${fmtChurn(selected)}</div>`}
      ${panelError && html`<div class="err">${panelError}</div>`}
      ${dirSection('Outgoing', out, 'out')}
      ${dirSection('Incoming', inc, 'in')}
      ${(view === 'raw' || isBox) && html`<h3>Children (${children.length})</h3>
        ${children.length
          ? html`<ul>${children.map((c) => html`<li key=${c.id}><span class="meta">${c.kind}</span><button onClick=${() => setSelectedId(nodes.some((n) => n.id === `c${c.id}`) ? `c${c.id}` : String(c.id))}>${c.name}</button>${locOf(c)}</li>`)}</ul>`
          : html`<div class="empty">none</div>`}`}
    ` : selectedEdge ? html`
      <h2>${labelOf(selectedEdge.source)} <span class="loc">→</span> ${labelOf(selectedEdge.target)}</h2>
      <div class="row">${Object.entries(selectedEdge.data.kinds || { [selectedEdge.data.kind]: selectedEdge.data.count })
        .map(([k, c]) => html`<span key=${k} class="tag" style=${{ background: edgeColor(k) }}>${k}${c > 1 ? ` ×${c}` : ''}</span>`)}
        ${selectedEdge.data.status && html`<span class="tag" style=${{ background: changeColor(selectedEdge.data.status) }}>${selectedEdge.data.status}</span>`}</div>
      <h3>References (${edgeVisibleRefs.length})</h3>
      ${edgeVisibleRefs.length ? renderTree(edgeTree, 'edge', (r) => r.to, false) : html`<div class="empty">no references of these kinds between the two</div>`}
    ` : html`<div class="empty">Click a node or an edge to inspect it. In the coalesced view, + on a node expands it and − on a box collapses it. Drag on empty canvas to select; middle/right-drag or space+drag to pan; ctrl+wheel to zoom.</div>`}
    <div class="legend">
      ${Object.entries(isDiff && byChange ? CHANGE_COLOR : EDGE_COLOR).map(([k, c]) => html`<span key=${k}><i style=${{ background: c }}></i>${k}</span>`)}
      ${!(isDiff && byChange) && html`<span><i style=${{ background: '#b0b4bb' }}></i>contains</span>`}
    </div>
  <//>`;
}

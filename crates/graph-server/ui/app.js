import { useState, useEffect, useMemo, useCallback, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import {
  ReactFlow, ReactFlowProvider, Controls, Background, ViewportPortal,
  applyNodeChanges, applyEdgeChanges, useReactFlow, getViewportForBounds,
} from '@xyflow/react';
import { html, fetchJson, actions, PRECEDENCE, ANIM_MS } from './common.js';
import { nodeTypes, edgeTypes, BundlePopover } from './nodes.js';
import { buildModel } from './model.js';
import { layout, withRouting, incrementalLayout, absolutePositions, interpolate, easeInOut, resolveDrag, rerouteEdges } from './layout.js';
import { CodePane } from './code.js';
import { Inspector } from './panel.js';
import { Search } from './search.js';

const MIN_ZOOM = 0.05, MAX_ZOOM = 2;

function App() {
  const { getViewport, setViewport } = useReactFlow();
  const canvasRef = useRef(null);
  const [view, setView] = useState('coalesced');
  const [dir, setDir] = useState('TB');
  const [graph, setGraph] = useState(null);
  const [coalesced, setCoalesced] = useState(null);
  const [nodes, setNodes] = useState([]);
  const [edges, setEdges] = useState([]);
  const [showContain, setShowContain] = useState(true);
  const [showRefs, setShowRefs] = useState(true);
  // Per-box edge bundling: entityId -> { in, out, inward }, each 0 | 1 | 2
  // (see buildModel). Absent means everything bundles at the box.
  const [bundling, setBundling] = useState(() => new Map());
  // Entities the user hid, client-side only; hiding a box hides its subtree.
  const [hiddenIds, setHiddenIds] = useState(() => new Set());
  // The one bundle popover open at a time.
  const [openPopoverId, setOpenPopoverId] = useState(null);
  // A container the coalesced view is pruned to, or null for everything.
  const [scopeId, setScopeId] = useState(null);
  const [onePerPair, setOnePerPair] = useState(true);
  // { type: 'node', id: <rf node id> } | { type: 'edge', id: <rf edge id> } | null
  const [sel, setSel] = useState(null);
  // The diagram's own selection (the outline React Flow draws) follows the
  // inspector's, so a node reached through search or a panel link is
  // highlighted like a clicked one.
  const setSelectedId = useCallback((id) => {
    setSel(id == null ? null : { type: 'node', id });
    setNodes((ns) => ns.map((n) => (n.selected === (n.id === id) ? n : { ...n, selected: n.id === id })));
  }, []);
  // The code pane: which file is open and which lines to mark.
  const [code, setCode] = useState(null);
  const [sources, setSources] = useState(() => new Map());
  const [panelW, setPanelW] = useState(460);
  // Diff mode only: colour edges by change instead of kind; hide (raw) or
  // fade (coalesced) what did not change.
  const [byChange, setByChange] = useState(true);
  const [changesOnly, setChangesOnly] = useState(false);
  const [showTests, setShowTests] = useState(true);
  const [resizing, setResizing] = useState(false);
  const [status, setStatus] = useState('');
  const [panelError, setPanelError] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    fetchJson('./graph.json').then(setGraph).catch((e) => setStatus(`graph.json: ${e.message}`));
    fetchJson('./coalesced.json').then(setCoalesced).catch((e) => setStatus(`coalesced.json: ${e.message}`));
  }, []);

  const isDiff = !!graph?.diff;
  const model = useMemo(
    () => (graph ? buildModel(graph, coalesced, view, dir, onePerPair, { byChange, changesOnly, hideTests: !showTests, bundling, scopeId, hiddenIds }) : null),
    [graph, coalesced, view, dir, onePerPair, byChange, changesOnly, showTests, bundling, scopeId, hiddenIds],
  );

  // Every layout change is animated: nodes glide from where they were to where
  // they belong, and the viewport moves with them to the node the user acted
  // on (focusRef holds candidate node ids, first existing one wins) or, with
  // no focus, to the whole graph. A timer commits the final state even if
  // animation frames never fire (background tab), so nothing is left midway.
  const nodesRef = useRef(nodes); nodesRef.current = nodes;
  const edgesRef = useRef(edges); edgesRef.current = edges;
  const focusRef = useRef(null);
  // Set by zoom, collapse and bundle actions: the next layout keeps every
  // existing node where it is instead of starting over.
  const keepPlacesRef = useRef(false);
  const pendingSelectRef = useRef(null);
  const animRef = useRef(0);

  // focusRef may also hold STAY: the layout changes but the camera does not
  // move at all (hiding a node removes what the user was looking at, and
  // refitting the whole graph would yank everything else away).
  const STAY = 'stay';
  const transitionTo = (finalNodes, finalEdges, focusIds) => {
    const token = ++animRef.current;
    const prevNodes = new Map(nodesRef.current.map((n) => [n.id, n]));
    const prevEdges = new Map(edgesRef.current.map((e) => [e.id, e]));
    const abs = absolutePositions(finalNodes);

    const el = canvasRef.current;
    const W = el?.clientWidth || 800, H = el?.clientHeight || 600;
    const focus = (Array.isArray(focusIds) ? focusIds : []).map((id) => finalNodes.find((n) => n.id === id)).find(Boolean);
    let vp = null;
    if (focus) {
      const a = abs(focus.id);
      const cur = getViewport();
      const seen = { x: -cur.x / cur.zoom, y: -cur.y / cur.zoom, w: W / cur.zoom, h: H / cur.zoom };
      const inView = a.x >= seen.x && a.y >= seen.y && a.x + focus.width <= seen.x + seen.w && a.y + focus.height <= seen.y + seen.h;
      // A target already fully on screen is left alone: panning to centre
      // it would move everything the user was looking at. Otherwise never
      // zoom out past what is needed to show it, and zoom in no further
      // than 1:1 so a small collapsed node does not fill the screen.
      if (!inView) vp = getViewportForBounds({ x: a.x, y: a.y, width: focus.width, height: focus.height }, W, H, 0.05, Math.max(cur.zoom, 1), 0.3);
    } else if (finalNodes.length && focusIds !== STAY) {
      let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
      for (const n of finalNodes) {
        if (n.parentId) continue;
        const a = abs(n.id);
        x0 = Math.min(x0, a.x); y0 = Math.min(y0, a.y); x1 = Math.max(x1, a.x + n.width); y1 = Math.max(y1, a.y + n.height);
      }
      vp = getViewportForBounds({ x: x0, y: y0, width: x1 - x0, height: y1 - y0 }, W, H, 0.05, 2, 0.15);
    }

    let done = false;
    const finish = () => {
      if (done || animRef.current !== token) return;
      done = true;
      setNodes(finalNodes); setEdges(finalEdges);
      if (vp) setViewport(vp);
      // A selection requested before this layout existed (search revealing
      // a node) is applied only now: selecting a node that is not drawn yet
      // would be undone by the liveness effect.
      const want = pendingSelectRef.current;
      if (want) { pendingSelectRef.current = null; const hit = want.find((id) => finalNodes.some((n) => n.id === id)); if (hit) setSelectedId(hit); }
    };
    if (!prevNodes.size) { finish(); return; }

    if (vp) setViewport(vp, { duration: ANIM_MS });
    const start = performance.now();
    const step = () => {
      if (done || animRef.current !== token) return;
      const t = Math.min(1, (performance.now() - start) / ANIM_MS);
      const f = interpolate(prevNodes, prevEdges, finalNodes, finalEdges, easeInOut(t));
      setNodes(f.nodes); setEdges(f.edges);
      if (t < 1) requestAnimationFrame(step); else finish();
    };
    requestAnimationFrame(step);
    setTimeout(finish, ANIM_MS + 80);
  };

  useEffect(() => {
    if (!model) return;
    setPanelError('');
    const keep = keepPlacesRef.current;
    keepPlacesRef.current = false;
    let laid = keep && view === 'coalesced' && nodesRef.current.length
      ? incrementalLayout(nodesRef.current, edgesRef.current, model.nodes, model.rankEdges, model.edges, dir)
      : null;
    if (!laid) {
      const full = layout(model.nodes, model.rankEdges, dir);
      laid = { nodes: full.nodes, edges: withRouting(model.edges, full.routed) };
    }
    // focusRef is not cleared here: with React batching, a follow-up zoom
    // request can set the next focus before this effect has run for the
    // previous one. Actions that want the whole graph clear it themselves.
    transitionTo(laid.nodes, laid.edges, focusRef.current);
  }, [model, dir]);

  useEffect(() => {
    if (!sel) return;
    const alive = sel.type === 'node' ? nodes.some((n) => n.id === sel.id) : edges.some((e) => e.id === sel.id);
    if (!alive) setSel(null);
  }, [nodes, edges, sel]);

  // A collapsed or hidden box takes its popover with it.
  useEffect(() => {
    if (openPopoverId != null && !nodes.some((n) => n.id === `c${openPopoverId}`)) setOpenPopoverId(null);
  }, [nodes, openPopoverId]);

  const fileOf = (id) => {
    for (let n = graph?.nodes[id]; n; n = n.parent != null ? graph.nodes[n.parent] : null) if (n.kind === 'file') return n.id;
    return null;
  };
  // `side` is which file the line numbers address in diff mode: removed
  // entities and references only exist in the old tree.
  const sideOf = (entityId) => (graph?.nodes[entityId]?.status === 'removed' ? 'old' : 'new');
  const loadingRef = useRef(new Set());
  const loadSource = useCallback((fileId) => {
    if (sources.has(fileId) || loadingRef.current.has(fileId)) return;
    loadingRef.current.add(fileId);
    fetchJson(`./source?id=${fileId}`)
      .then((src) => setSources((m) => new Map(m).set(fileId, src)))
      .catch((e) => setSources((m) => new Map(m).set(fileId, { error: e.message })))
      .finally(() => loadingRef.current.delete(fileId));
  }, [sources]);
  // The right pane is two tabs. Opening code from the inspector (a site or
  // definition link) is a request to read it, so the Code tab comes forward;
  // selecting a node only refreshes what the Code tab would show and leaves
  // the user on whichever tab they are reading.
  const [tab, setTab] = useState('inspect');
  const openCode = (entityId, line, range, side = sideOf(entityId), { show = true } = {}) => {
    const fileId = fileOf(entityId);
    if (fileId == null) return;
    setCode({ fileId, line, range, side });
    if (show) setTab('code');
    loadSource(fileId);
  };
  const closeCode = useCallback(() => { setCode(null); setTab('inspect'); }, []);
  // Selecting anything inside a file opens it at that entity; selecting a
  // folder leaves whatever is open alone.
  useEffect(() => {
    if (sel?.type !== 'node') return;
    const ent = nodes.find((n) => n.id === sel.id)?.data;
    // Tinting a whole file's range would paint every line; only sub-file
    // entities get their extent marked.
    if (ent) openCode(ent.id, ent.line_start, ent.kind === 'file' ? [-1, -1] : [ent.line_start, ent.line_end], undefined, { show: false });
  }, [sel]);

  // Two wheel gestures React Flow gets wrong on macOS are handled before
  // the pane sees them. Shift+wheel: React Flow only remaps it to a sideways
  // pan on non-Mac platforms and expects Chrome to send a horizontal delta,
  // which it does not, so the graph would scroll up and down. Ctrl+wheel:
  // React Flow multiplies the delta by ten because browsers also flag
  // trackpad pinches with ctrlKey, which makes one mouse tick a 4x jump. A
  // pinch never comes with a real Control keydown, so that is how the two
  // are told apart; the pinch keeps React Flow's handling.
  const ctrlHeldRef = useRef(false);
  useEffect(() => {
    const el = canvasRef.current;
    if (!el) return;
    const onKey = (e) => { if (e.key === 'Control') ctrlHeldRef.current = e.type === 'keydown'; };
    const onBlur = () => { ctrlHeldRef.current = false; };
    const onWheel = (e) => {
      if (e.metaKey || e.deltaY === 0) return;
      const v = getViewport();
      if (e.shiftKey && !e.ctrlKey && e.deltaX === 0) {
        e.preventDefault(); e.stopPropagation();
        setViewport({ x: v.x - e.deltaY * (e.deltaMode === 1 ? 20 : 1), y: v.y, zoom: v.zoom });
      } else if (e.ctrlKey && ctrlHeldRef.current) {
        e.preventDefault(); e.stopPropagation();
        const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v.zoom * Math.pow(2, -e.deltaY * (e.deltaMode === 1 ? 0.05 : 0.0025))));
        const r = el.getBoundingClientRect(), px = e.clientX - r.left, py = e.clientY - r.top;
        setViewport({ x: px - (px - v.x) * (zoom / v.zoom), y: py - (py - v.y) * (zoom / v.zoom), zoom });
      }
    };
    el.addEventListener('wheel', onWheel, { capture: true, passive: false });
    window.addEventListener('keydown', onKey); window.addEventListener('keyup', onKey); window.addEventListener('blur', onBlur);
    return () => {
      el.removeEventListener('wheel', onWheel, { capture: true });
      window.removeEventListener('keydown', onKey); window.removeEventListener('keyup', onKey); window.removeEventListener('blur', onBlur);
    };
  }, []);

  const startResize = (e) => {
    e.preventDefault();
    const x0 = e.clientX, w0 = panelW;
    setResizing(true);
    const move = (ev) => setPanelW(Math.max(240, Math.min(window.innerWidth - 320, w0 + (x0 - ev.clientX))));
    const up = () => { setResizing(false); window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', up); };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };

  const switchView = (v) => { focusRef.current = null; setView(v); setStatus(''); };
  const switchDir = (d) => { focusRef.current = null; setDir(d); };
  const reset = () => { focusRef.current = null; return post('./coalesced/reset'); };
  const toggleOnePerPair = (on) => { focusRef.current = null; setOnePerPair(on); };

  const relayout = () => {
    if (!model) return;
    const laid = layout(model.nodes, model.rankEdges, dir);
    transitionTo(laid.nodes, withRouting(model.edges, laid.routed), null);
  };

  const visibleEdges = useMemo(
    () => edges.map((e) => {
      const fade = isDiff && changesOnly && view === 'coalesced' && e.data.status === 'same';
      return { ...e, hidden: e.data.ref ? !showRefs : !showContain, style: fade ? { ...e.style, opacity: 0.25 } : e.style };
    }),
    [edges, showContain, showRefs, isDiff, changesOnly, view],
  );

  // The open popover is UI-only state, kept out of buildModel's inputs so
  // opening or closing one never triggers a re-layout; it is rendered
  // through a ViewportPortal (see below) rather than as node data.
  const openBoxId = openPopoverId != null ? `c${openPopoverId}` : null;
  const openBox = openBoxId != null ? nodes.find((n) => n.id === openBoxId) : null;
  const openBoxPos = openBox ? absolutePositions(nodes)(openBox.id) : null;

  // Position changes go through the drag resolver instead of straight into
  // the node list; everything else (selection, dragging flags) applies as-is.
  // Nodes whose position or size changed during a drag are collected, and
  // when the drag ends the edges around them are re-routed.
  const dirtyRef = useRef(new Set());
  const rerouteRef = useRef(false);
  const onNodesChange = useCallback((changes) => setNodes((ns) => {
    const moves = changes.filter((c) => c.type === 'position' && c.position);
    const rest = changes.filter((c) => !(c.type === 'position' && c.position));
    let next = ns;
    if (moves.length) {
      next = resolveDrag(ns, new Map(moves.map((c) => [c.id, c.position])));
      ns.forEach((o, i) => {
        const n = next[i];
        if (o.position.x !== n.position.x || o.position.y !== n.position.y || o.width !== n.width || o.height !== n.height) dirtyRef.current.add(n.id);
      });
    }
    if (changes.some((c) => c.type === 'position' && c.dragging === false) && dirtyRef.current.size) rerouteRef.current = true;
    const flags = moves.filter((c) => c.dragging !== undefined).map((c) => ({ id: c.id, type: 'position', dragging: c.dragging }));
    return applyNodeChanges([...rest, ...flags], next);
  }), []);
  useEffect(() => {
    if (!rerouteRef.current) return;
    rerouteRef.current = false;
    const dirty = dirtyRef.current;
    dirtyRef.current = new Set();
    setEdges((es) => rerouteEdges(es, nodes, dirty));
  }, [nodes]);
  const onEdgesChange = useCallback((c) => setEdges((es) => applyEdgeChanges(c, es)), []);
  const onNodeClick = useCallback((_, n) => { setSelectedId(n.id); setPanelError(''); }, []);
  const onEdgeClick = useCallback((_, e) => { if (e.data.ref) { setSel({ type: 'edge', id: e.id }); setPanelError(''); } }, []);
  const onPaneClick = useCallback(() => setSel(null), []);

  const post = async (path) => {
    setBusy(true); setPanelError(''); setStatus('');
    try {
      const co = await fetchJson(path, { method: 'POST' });
      setCoalesced(co);
      return co;
    } catch (e) {
      if (e.status === 409) setPanelError(e.message);
      else setStatus(`POST ${path}: ${e.message}`);
      return null;
    } finally { setBusy(false); }
  };

  // The server only zooms out one leaf at a time, and that collapses the
  // leaf's parent. Closing a box whose children are themselves expanded
  // therefore means collapsing the deepest leaves first, one request each,
  // until nothing under the box is a leaf any more.
  const isUnder = (id, boxId) => {
    for (let p = graph.nodes[id]?.parent; p != null; p = graph.nodes[p].parent) if (p === boxId) return true;
    return false;
  };
  const depthOf = (id) => { let d = 0; for (let p = graph.nodes[id]?.parent; p != null; p = graph.nodes[p].parent) d++; return d; };
  const collapse = async (boxId) => {
    let co = coalesced;
    while (co) {
      const under = co.leaves.filter((l) => isUnder(l, boxId));
      if (!under.length) return;
      const deepest = under.reduce((a, b) => (depthOf(b) > depthOf(a) ? b : a));
      // The box stays a container until the last step, then becomes a leaf.
      focusRef.current = [`c${boxId}`, String(boxId)];
      keepPlacesRef.current = true;
      co = await post(`./coalesced/zoom-out?id=${deepest}`);
    }
  };
  actions.zoomIn = (id) => { focusRef.current = [`c${id}`]; keepPlacesRef.current = true; return post(`./coalesced/zoom-in?id=${id}`); };
  actions.collapse = collapse;
  actions.prune = (id) => { focusRef.current = [`c${id}`]; setScopeId(id); };
  const clearScope = () => { focusRef.current = null; setScopeId(null); };

  // Hiding removes the node (and, since it is one entity, its whole subtree).
  actions.hide = (id) => {
    focusRef.current = STAY;
    keepPlacesRef.current = true;
    setHiddenIds((prev) => new Set(prev).add(id));
  };
  const showAllHidden = () => { focusRef.current = null; setHiddenIds(new Set()); };

  const panTo = (rfId) => {
    const n = nodesRef.current.find((x) => x.id === rfId);
    if (!n) return;
    const a = absolutePositions(nodesRef.current)(n.id);
    const el = canvasRef.current, cur = getViewport();
    const vp = getViewportForBounds({ x: a.x, y: a.y, width: n.width, height: n.height }, el?.clientWidth || 800, el?.clientHeight || 600, 0.05, Math.max(cur.zoom, 1), 0.3);
    setViewport(vp, { duration: ANIM_MS });
  };
  // Bring a searched-for entity on screen. In the coalesced view that means
  // zooming into every ancestor that is still a leaf, one request each; the
  // selection waits for the final layout (pendingSelectRef). Anything that
  // would keep the entity out of the picture (a hide, a scope) is undone.
  const revealEntity = async (id) => {
    const rf = [String(id), `c${id}`];
    for (let c = id; c != null; c = graph.nodes[c].parent) if (hiddenIds.has(c)) setHiddenIds((prev) => { const next = new Set(prev); next.delete(c); return next; });
    if (scopeId != null && id !== scopeId && !isUnder(id, scopeId)) setScopeId(null);
    const drawn = () => rf.find((x) => nodesRef.current.some((n) => n.id === x));
    if (view === 'raw') {
      if (drawn()) { setSelectedId(drawn()); panTo(drawn()); } else { focusRef.current = rf; pendingSelectRef.current = rf; }
      return;
    }
    const chain = [];
    for (let c = graph.nodes[id].parent; c != null; c = graph.nodes[c].parent) chain.unshift(c);
    let co = coalesced, zoomed = false;
    for (const anc of chain) {
      if (!co || !co.leaves.includes(anc)) continue;
      focusRef.current = rf;
      keepPlacesRef.current = true;
      pendingSelectRef.current = rf;
      co = await post(`./coalesced/zoom-in?id=${anc}`);
      zoomed = true;
    }
    if (!zoomed) {
      if (drawn()) { setSelectedId(drawn()); panTo(drawn()); } else { focusRef.current = rf; pendingSelectRef.current = rf; }
    }
  };

  actions.togglePopover = (id) => setOpenPopoverId((cur) => (cur === id ? null : id));
  actions.closePopover = () => setOpenPopoverId(null);

  actions.setBundling = (boxId, key, lvl) => {
    focusRef.current = [`c${boxId}`];
    keepPlacesRef.current = true;
    setBundling((prev) => {
      const next = new Map(prev);
      const cur = { ...(prev.get(boxId) || {}), [key]: lvl };
      if (cur.in || cur.out || cur.inward) next.set(boxId, cur); else next.delete(boxId);
      return next;
    });
  };
  actions.resetBundling = (boxId) => {
    focusRef.current = [`c${boxId}`];
    keepPlacesRef.current = true;
    setBundling((prev) => {
      const next = new Map(prev);
      for (const id of prev.keys()) if (id === boxId || isUnder(id, boxId)) next.delete(id);
      return next;
    });
  };
  // The scope is a container; once it is collapsed back into a leaf there is
  // nothing to prune to, so the view widens again on its own.
  useEffect(() => {
    if (scopeId != null && coalesced && !coalesced.leaves.some((l) => isUnder(l, scopeId))) setScopeId(null);
  }, [coalesced, scopeId]);

  const nameOf = (id) => graph?.nodes[id]?.name ?? `#${id}`;

  return html`<div class="app">
    <div class="toolbar">
      <div class="seg">
        <button class=${view === 'raw' ? 'on' : ''} onClick=${() => switchView('raw')}>Raw</button>
        <button class=${view === 'coalesced' ? 'on' : ''} onClick=${() => switchView('coalesced')}>Coalesced</button>
      </div>
      <div class="seg">
        <button class=${dir === 'TB' ? 'on' : ''} onClick=${() => switchDir('TB')}>TB</button>
        <button class=${dir === 'LR' ? 'on' : ''} onClick=${() => switchDir('LR')}>LR</button>
      </div>
      <button class="btn" onClick=${relayout}>Re-layout</button>
      ${view === 'coalesced' && html`<button class="btn" disabled=${busy} onClick=${reset}>Reset</button>`}
      <div class="sep"></div>
      ${view === 'raw' && html`<label class="chk"><input type="checkbox" checked=${showContain} onChange=${(e) => setShowContain(e.target.checked)} /> containment edges</label>`}
      <label class="chk"><input type="checkbox" checked=${showRefs} onChange=${(e) => setShowRefs(e.target.checked)} /> reference edges</label>
      ${view === 'coalesced' && scopeId != null && html`<span>showing <b>${nameOf(scopeId)}</b> <button class="btn" onClick=${clearScope}>show all</button></span>`}
      ${hiddenIds.size > 0 && html`<span>${hiddenIds.size} hidden · <button class="btn" onClick=${showAllHidden}>show all</button></span>`}
      <label class="chk" title=${'Keep only the strongest kind between a pair: ' + PRECEDENCE.join(' > ')}><input type="checkbox" checked=${onePerPair} onChange=${(e) => toggleOnePerPair(e.target.checked)} /> one edge per pair</label>
      <label class="chk" title="Test code: #[test] / #[cfg(test)] items, tests/ folders, *_test and *.spec files, and everything inside them. Unchecked hides them and the references they make."><input type="checkbox" checked=${showTests} onChange=${(e) => { focusRef.current = null; setShowTests(e.target.checked); }} /> tests${model?.hiddenTests ? ` (${model.hiddenTests} hidden)` : ''}</label>
      ${isDiff && html`<div class="sep"></div>
        <span title=${graph.diff.base_commit}><b>diff</b> ${graph.diff.base} → working tree</span>
        <label class="chk"><input type="checkbox" checked=${byChange} onChange=${(e) => { focusRef.current = null; setByChange(e.target.checked); }} /> colour edges by change</label>
        <label class="chk" title="Raw view hides unchanged entities; coalesced view fades them"><input type="checkbox" checked=${changesOnly} onChange=${(e) => { focusRef.current = null; setChangesOnly(e.target.checked); }} /> changes only</label>`}
      <${Search} graph=${graph} showTests=${showTests} hiddenIds=${hiddenIds} onPick=${revealEntity} />
      <span style=${{ color: 'var(--muted)' }}>${nodes.filter((n) => n.type !== 'container').length} nodes · ${visibleEdges.filter((e) => !e.hidden).length} edges</span>
      ${status && html`<div class="status" title=${status}>${status}</div>`}
    </div>
    <div class="main">
      <div class="canvas" ref=${canvasRef}>
        <${ReactFlow}
          nodes=${nodes} edges=${visibleEdges} nodeTypes=${nodeTypes} edgeTypes=${edgeTypes}
          onNodesChange=${onNodesChange} onEdgesChange=${onEdgesChange}
          onNodeClick=${onNodeClick} onEdgeClick=${onEdgeClick} onPaneClick=${onPaneClick}
          nodesDraggable=${true} nodesConnectable=${false} elementsSelectable=${true}
          panOnScroll=${true} panOnScrollMode="free" zoomOnScroll=${false} zoomOnPinch=${true}
          zoomActivationKeyCode="Control" zoomOnDoubleClick=${false}
          panOnDrag=${[1, 2]} selectionOnDrag=${true} selectionMode="partial"
          onlyRenderVisibleElements=${true} minZoom=${MIN_ZOOM} maxZoom=${MAX_ZOOM} proOptions=${{ hideAttribution: true }}
        >
          <${Controls} showInteractive=${false} />
          <${Background} gap=${20} color="#dcdfe4" />
          ${openBox && openBoxPos && html`<${ViewportPortal}>
            <div style=${{ position: 'absolute', transform: `translate(${openBoxPos.x}px, ${openBoxPos.y}px)`, width: `${openBox.width}px`, height: `${openBox.height}px` }}>
              <${BundlePopover} id=${openPopoverId} levels=${bundling.get(openPopoverId) || {}} />
            </div>
          <//>`}
        <//>
      </div>
      <div class=${'divider' + (resizing ? ' active' : '')} onMouseDown=${startResize}></div>
      <div class="panel" style=${{ width: panelW }}>
        <div class="panel-tabs">
          <button class=${tab === 'inspect' ? 'on' : ''} onClick=${() => setTab('inspect')}>Inspector</button>
          <button class=${tab === 'code' ? 'on' : ''} disabled=${!code} title=${code ? (sources.get(code.fileId)?.path || '') : 'Select a node inside a file to open its code'} onClick=${() => setTab('code')}>Code${code && sources.get(code.fileId)?.path ? html` <span class="tab-file">${sources.get(code.fileId).path.split('/').pop()}</span>` : ''}</button>
        </div>
        <div class="panel-top" hidden=${tab !== 'inspect'}>
        <${Inspector}
          graph=${graph} nodes=${nodes} edges=${edges} view=${view} coalesced=${coalesced}
          sel=${sel} setSelectedId=${setSelectedId} showTests=${showTests} byChange=${byChange} isDiff=${isDiff}
          panelError=${panelError} openCode=${openCode} sideOf=${sideOf} fileOf=${fileOf} isUnder=${isUnder}
          hiddenIds=${hiddenIds} sources=${sources} loadSource=${loadSource}
        />
        </div>
        ${code && tab === 'code' && html`<${CodePane} src=${sources.get(code.fileId)} line=${code.line} range=${code.range} side=${code.side} onClose=${closeCode} />`}
      </div>
    </div>
  </div>`;
}

createRoot(document.getElementById('root')).render(html`<${ReactFlowProvider}><${App} /><//>`);

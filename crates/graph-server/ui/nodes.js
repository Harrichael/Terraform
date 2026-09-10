import { useEffect, useRef } from 'react';
import { Handle, BaseEdge, getBezierPath, useInternalNode } from '@xyflow/react';
import { html, actions, fmtChurn, fmtLoc, statusClass } from './common.js';

export function ZoomButton({ glyph, title, onClick, on, cls }) {
  return html`<button class=${'zoom nodrag nopan' + (on ? ' on' : '') + (cls ? ' ' + cls : '')} title=${title}
    onClick=${(e) => { e.stopPropagation(); onClick(); }}>${glyph}</button>`;
}

export function EntityNode({ data, sourcePosition, targetPosition }) {
  const lines = data.kind !== 'folder' ? ` · ${data.line_start + 1}–${data.line_end + 1}` : '';
  const loc = fmtLoc(data), churn = fmtChurn(data);
  return html`<div class=${'ent kind-' + data.kind + statusClass(data)} title=${data.path}>
    <${Handle} type="target" position=${targetPosition} />
    <div class="ent-name">${data.name}</div>
    <div class="ent-sub">${data.kind}${churn ? html` · ${churn}` : loc ? ` · ${loc}` : lines}</div>
    <${Handle} type="source" position=${sourcePosition} />
    <${ZoomButton} glyph="×" cls="hide" title="Hide this node" onClick=${() => actions.hide(data.id)} />
    ${data.zoomable && html`<${ZoomButton} glyph="+" title="Zoom in" onClick=${() => actions.zoomIn(data.id)} />`}
  </div>`;
}

// The bundle popover: incoming/outgoing checkboxes plus an apply-to scope.
// Closes itself on an outside click or Escape. Rendered by App through a
// ViewportPortal rather than as a normal child of the box: React Flow gives
// every child of a group node a z-index above the group's own, so a popover
// nested inside the box div can never draw over the box's own contents.
export function BundlePopover({ data }) {
  const ref = useRef(null);
  useEffect(() => {
    const onDown = (e) => { if (ref.current && !ref.current.contains(e.target)) actions.closePopover(); };
    const onKey = (e) => { if (e.key === 'Escape') actions.closePopover(); };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => { document.removeEventListener('mousedown', onDown); document.removeEventListener('keydown', onKey); };
  }, []);
  const scope = data.popoverScope;
  const bundling = data.bundling || {};
  const scopes = [['self', 'this box'], ['direct', '+ direct sub boxes'], ['nested', '+ all nested boxes']];
  return html`<div class="bundle-pop nodrag nopan" ref=${ref}>
    <label class="chk"><input type="checkbox" checked=${!!bundling.in}
      onChange=${(e) => actions.setBundling(data.id, { in: e.target.checked }, scope)} /> Incoming edges drawn to contents</label>
    <label class="chk"><input type="checkbox" checked=${!!bundling.out}
      onChange=${(e) => actions.setBundling(data.id, { out: e.target.checked }, scope)} /> Outgoing edges drawn to contents</label>
    <div class="bundle-scope">
      <span>Apply to:</span>
      ${scopes.map(([s, label]) => html`<button key=${s} class=${scope === s ? 'on' : ''} onClick=${() => actions.setPopoverScope(s)}>${label}</button>`)}
    </div>
    <button class="bundle-reset" onClick=${() => actions.resetBundling(data.id)}>Bundle everything</button>
  </div>`;
}

export function ContainerNode({ data, sourcePosition, targetPosition }) {
  const churn = fmtChurn(data);
  const bundlingOn = !!(data.bundling && (data.bundling.in || data.bundling.out));
  return html`<div class=${'box kind-' + data.kind + statusClass(data)} title=${data.path}>
    <${Handle} type="target" position=${targetPosition} />
    <div class="box-label">${data.name}${churn ? html` <span class="loc">·</span> ${churn}` : fmtLoc(data) && html` <span class="loc">· ${fmtLoc(data)}</span>`}</div>
    <div class="box-actions">
      <${ZoomButton} glyph=${bundlingOn ? '⇶' : '⇉'} on=${bundlingOn}
        title=${bundlingOn ? 'Some edges are drawn to the nodes inside this box; click to change' : 'Edges into and out of this box are bundled at the box; click to change'}
        onClick=${() => actions.togglePopover(data.id)} />
      <${ZoomButton} glyph="⤢" title="Show only this box" onClick=${() => actions.prune(data.id)} />
      <${ZoomButton} glyph="−" title="Zoom out" onClick=${() => actions.collapse(data.id)} />
      <${ZoomButton} glyph="×" title="Hide this box and its contents" onClick=${() => actions.hide(data.id)} />
    </div>
    <${Handle} type="source" position=${sourcePosition} />
  </div>`;
}
export const nodeTypes = { entity: EntityNode, container: ContainerNode };

// Smooth polyline through dagre's routing points: corners are rounded by
// curving through each interior point toward the midpoint of the next segment.
export function routedPath(pts) {
  let d = `M ${pts[0].x} ${pts[0].y}`;
  for (let i = 1; i < pts.length - 1; i++) {
    const mx = (pts[i].x + pts[i + 1].x) / 2, my = (pts[i].y + pts[i + 1].y) / 2;
    d += ` Q ${pts[i].x} ${pts[i].y} ${mx} ${my}`;
  }
  const last = pts[pts.length - 1];
  return d + ` L ${last.x} ${last.y}`;
}
export const moved = (node, at) => !node || !at || Math.abs(node.internals.positionAbsolute.x - at.x) > 0.5 || Math.abs(node.internals.positionAbsolute.y - at.y) > 0.5;

// Edges sharing an endpoint pair (e.g. an import and a call between the same
// two leaves) would otherwise draw exactly on top of each other, hiding all
// but one; shift each one sideways by data.offset.
//
// When the layout supplied dagre routing points (data.points) the edge follows
// them, which is what keeps it out of the nodes between its endpoints. The
// points are only valid while both endpoints sit where the layout put them;
// after a drag the edge falls back to a plain bezier until the next re-layout.
export function RefEdge({ source, target, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, markerEnd, style: base, data, selected }) {
  // React Flow's own `.selected` rule loses to the inline stroke, so selection
  // is drawn here.
  const style = selected ? { ...base, strokeWidth: (base.strokeWidth || 1.5) + 2, filter: 'drop-shadow(0 0 2px rgba(0,0,0,.5))' } : base;
  const s = useInternalNode(source), t = useInternalNode(target);
  const routed = data?.points && !moved(s, data.at?.s) && !moved(t, data.at?.t);
  let path, lx, ly;
  if (routed) {
    path = routedPath(data.points);
    const m = data.points[Math.floor(data.points.length / 2)];
    lx = m.x; ly = m.y;
  } else {
    [path, lx, ly] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition });
  }
  const dx = targetX - sourceX, dy = targetY - sourceY, len = Math.hypot(dx, dy) || 1;
  const k = data?.offset || 0;
  return html`<g transform=${`translate(${(-dy / len) * k} ${(dx / len) * k})`}>
    <${BaseEdge} path=${path} markerEnd=${markerEnd} style=${style} />
    ${data?.count > 1 && html`<text class="edge-count" x=${lx} y=${ly} fill=${style.stroke}>${data.count}</text>`}
  </g>`;
}
export const edgeTypes = { ref: RefEdge };

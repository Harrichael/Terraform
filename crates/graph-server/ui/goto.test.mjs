// Run with `node crates/graph-server/ui/goto.test.mjs`.
//
// A small graph in the shape /graph.json serves (0-based lines, lowercase
// kinds), with the cases that bit in practice: two calls on one line, a
// callee's name as a substring on the neighbouring line, same-named targets
// in two files, a target folded into a collapsed folder, and diff-mode rows
// whose removed references site old lines.
import assert from 'node:assert/strict';
import { tokenAt, referenceTargets, nearestDrawn, drawnTarget } from './goto.js';

const N = (id, kind, name, parent, line_start = 0, line_end = 0) => ({ id, kind, name, path: name, parent, line_start, line_end, loc: 1 });
const graph = {
  nodes: [
    N(0, 'folder', 'demo', null),
    N(1, 'folder', 'src', 0),
    N(2, 'file', 'main.rs', 1, 0, 12),
    N(3, 'function', 'main', 2, 2, 12),
    N(4, 'function', 'helper', 2, 14, 16),
    N(5, 'file', 'lib.rs', 1),
    N(6, 'class', 'Point', 5),
    N(7, 'function', 'new', 6),
    N(8, 'function', 'distance', 6),
    N(9, 'file', 'ax.rs', 1),
    N(10, 'function', 'focus', 9),
    N(11, 'function', 'len', 5),
    N(12, 'file', 'util.rs', 1),
    N(13, 'function', 'len', 12),
    N(14, 'folder', 'other', 0),
    N(15, 'file', 'z.rs', 14),
    N(16, 'function', 'zap', 15),
  ],
  references: [
    { from: 2, to: 6, kind: 'import', sites: [0] },
    { from: 3, to: 7, kind: 'call', sites: [3, 4] },
    { from: 3, to: 4, kind: 'call', sites: [5] },
    { from: 3, to: 8, kind: 'call', sites: [5] },
    { from: 3, to: 10, kind: 'call', sites: [7] },
    { from: 3, to: 11, kind: 'call', sites: [8] },
    { from: 3, to: 13, kind: 'call', sites: [8] },
    { from: 3, to: 16, kind: 'call', sites: [9] },
    { from: 3, to: 13, kind: 'call', sites: [11] },
    // zap() in z.rs also calls helper on *its* line 5: same line number,
    // other file, must never be a candidate while main.rs is open.
    { from: 16, to: 4, kind: 'call', sites: [5] },
  ],
};
const src = [
  'use crate::Point;',
  '',
  'fn main() {',
  '    let p = Point::new(1, 2);',
  '    let q = Point::new(3, 4);',
  '    helper(p.distance(q));',
  '    println!("focused now: {}", 1);',
  '    ax::focus(target); // refocus later',
  '    let n = a.len() + b.len();',
  '    zap();',
  '    let x = y;',
  '    let n2 = length(v);',
  '}',
];
const MAIN = 2;
const row = (n) => ({ o: null, n, t: src[n], k: '' });
const colOf = (n, word, nth = 0) => { let i = -1; for (let k = 0; k <= nth; k++) i = src[n].indexOf(word, i + 1); assert.ok(i >= 0, `${word} on line ${n}`); return i; };
const targets = (n, word, nth) => referenceTargets(graph, MAIN, row(n), colOf(n, word, nth));
const drawnIn = (ids) => (rf) => ids.includes(rf);

let passed = 0;
const test = (name, fn) => { fn(); passed++; console.log(`ok - ${name}`); };

test('tokenAt: caret inside, at start, at end of a word; not on punctuation or blank', () => {
  const t = '    helper(p.distance(q));';
  assert.deepEqual(tokenAt(t, 6), { token: 'helper', start: 4, end: 10 });
  assert.deepEqual(tokenAt(t, 4), { token: 'helper', start: 4, end: 10 });
  assert.deepEqual(tokenAt(t, 10), { token: 'helper', start: 4, end: 10 });
  assert.equal(tokenAt(t, 12).token, 'p');
  assert.equal(tokenAt(t, 1), null);
  assert.equal(tokenAt(t, t.length), null);
  assert.equal(tokenAt('', 0), null);
});

test('a normal call resolves to the callee, wherever on the token the caret lands', () => {
  assert.deepEqual(targets(3, 'new'), [7]);
  assert.deepEqual(referenceTargets(graph, MAIN, row(3), colOf(3, 'new') + 3), [7]);
  assert.deepEqual(targets(4, 'new'), [7]);
  assert.deepEqual(targets(0, 'Point'), [6]);
});

test('two calls on one line are told apart by the clicked identifier', () => {
  assert.deepEqual(targets(5, 'helper'), [4]);
  assert.deepEqual(targets(5, 'distance'), [8]);
  // Both callees are named on the line, so a click elsewhere on it is not a
  // vote for either.
  assert.deepEqual(targets(5, 'p.'), []);
  assert.deepEqual(targets(5, 'q'), []);
});

test("the 'focus' substring trap: only a whole token on a sited line counts", () => {
  assert.deepEqual(targets(6, 'focused'), []);
  assert.deepEqual(targets(6, 'now'), []);
  assert.deepEqual(targets(7, 'focus'), [10]);
  assert.deepEqual(targets(7, 'refocus'), []);
  assert.deepEqual(targets(7, 'target'), []);
});

test('references from another file sited on the same line number are ignored', () => {
  assert.deepEqual(referenceTargets(graph, 15, { o: null, n: 5, t: '    helper();', k: '' }, 5), [4]);
  assert.deepEqual(referenceTargets(graph, 9, row(5), colOf(5, 'helper')), []);
});

test('a target whose name is not on the line is reachable from any identifier there', () => {
  assert.deepEqual(targets(11, 'length'), [13]);
  assert.deepEqual(targets(11, 'v'), [13]);
});

test('no reference, keyword, or blank: nothing', () => {
  assert.deepEqual(targets(10, 'x'), []);
  assert.deepEqual(targets(10, 'y'), []);
  assert.deepEqual(targets(3, 'let'), []);
  assert.deepEqual(referenceTargets(graph, MAIN, row(3), 0), []);
  assert.deepEqual(referenceTargets(graph, MAIN, row(1), 0), []);
});

test('nearestDrawn walks up to the first drawn leaf or box', () => {
  assert.equal(nearestDrawn(graph, 16, drawnIn(['16', 'c15', 'c14', 'c0'])), '16');
  assert.equal(nearestDrawn(graph, 16, drawnIn(['15', 'c14', 'c0'])), '15');
  assert.equal(nearestDrawn(graph, 16, drawnIn(['14', 'c0', '1'])), '14');
  assert.equal(nearestDrawn(graph, 16, drawnIn(['c14', 'c0'])), 'c14');
  assert.equal(nearestDrawn(graph, 16, drawnIn(['0'])), '0');
  // Scoped to src: nothing on zap's chain is drawn.
  assert.equal(nearestDrawn(graph, 16, drawnIn(['c1', '2', '5', '9', '12'])), null);
});

test('a collapsed target highlights the ancestor that stands for it', () => {
  const coalesced = drawnIn(['c0', 'c1', '2', '5', '9', '12', '14']);
  assert.equal(drawnTarget(graph, MAIN, row(9), colOf(9, 'zap'), coalesced), '14');
  assert.equal(drawnTarget(graph, MAIN, row(7), colOf(7, 'focus'), coalesced), '9');
  assert.equal(drawnTarget(graph, MAIN, row(5), colOf(5, 'distance'), coalesced), '5');
  const expanded = drawnIn(['c0', 'c1', 'c2', '3', '4', 'c5', 'c6', '7', '8', '11', '9', '12', 'c14', '15']);
  assert.equal(drawnTarget(graph, MAIN, row(9), colOf(9, 'zap'), expanded), '15');
  assert.equal(drawnTarget(graph, MAIN, row(5), colOf(5, 'distance'), expanded), '8');
  assert.equal(drawnTarget(graph, MAIN, row(5), colOf(5, 'helper'), expanded), '4');
});

test('same-named targets: nothing while the view tells them apart, the shared box once it does not', () => {
  assert.deepEqual(targets(8, 'len'), [11, 13]);
  assert.deepEqual(targets(8, 'len', 1), [11, 13]);
  assert.equal(drawnTarget(graph, MAIN, row(8), colOf(8, 'len'), drawnIn(['c0', 'c1', '2', '5', '12'])), null);
  assert.equal(drawnTarget(graph, MAIN, row(8), colOf(8, 'len'), drawnIn(['c0', '1', '14'])), '1');
});

test('no-op cases give null, never a fallback node', () => {
  const all = drawnIn(['c0', '1', '14']);
  assert.equal(drawnTarget(graph, MAIN, row(10), colOf(10, 'x'), all), null);
  assert.equal(drawnTarget(graph, MAIN, row(6), colOf(6, 'focused'), all), null);
  assert.equal(drawnTarget(graph, MAIN, row(9), colOf(9, 'zap'), drawnIn(['c1', '2', '5'])), null);
});

test('diff rows: a removed reference is found by its old line, others by the new line', () => {
  const g = {
    nodes: [N(0, 'folder', 'r', null), N(1, 'file', 'a.rs', 0), N(2, 'function', 'f', 1), N(3, 'function', 'gone', 1, 0, 0), N(4, 'function', 'kept', 1)],
    references: [
      { from: 2, to: 3, kind: 'call', sites: [20], status: 'removed' },
      { from: 2, to: 4, kind: 'call', sites: [20], status: 'same' },
    ],
  };
  const del = { o: 20, n: null, t: '  gone();', k: 'del' };
  const ins = { o: null, n: 20, t: '  kept();', k: 'ins' };
  const eq = { o: 20, n: 20, t: '  gone(); kept();', k: '' };
  assert.deepEqual(referenceTargets(g, 1, del, 3), [3]);
  assert.deepEqual(referenceTargets(g, 1, ins, 3), [4]);
  assert.deepEqual(referenceTargets(g, 1, eq, 3), [3]);
  assert.deepEqual(referenceTargets(g, 1, eq, 11), [4]);
  // On a deleted row only old-side references are sited: `kept` (a `same`
  // reference at new line 20) is not a candidate there, and `gone`, named on
  // the line, is not reached by clicking something else.
  assert.deepEqual(referenceTargets(g, 1, { o: 20, n: null, t: '  gone(); kept();', k: 'del' }, 11), []);
});

console.log(`\n${passed} tests passed`);

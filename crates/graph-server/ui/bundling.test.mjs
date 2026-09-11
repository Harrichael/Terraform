// Run with `node crates/graph-server/ui/bundling.test.mjs`.
//
// A tree shaped like the view the inward bug was reported on: a deep spine
// ordo > Ordo > crates > ordo > src > platform, a sub box (hal) under
// platform so "one level" and "the leaf" are different places, and leaves
// at several depths so edges meet at every level from siblings to the root's
// children. Chains are built the way model.js builds them: leaf first, up
// through every drawn box.
import assert from 'node:assert/strict';
import { attachIndex, bundleEnds } from './bundling.js';

const N = (id, name, parent) => ({ id, name, parent });
const nodes = [
  N(0, 'ordo', null),
  N(1, 'Ordo', 0),
  N(2, 'crates', 1),
  N(3, 'ordo', 2),
  N(4, 'src', 3),
  N(5, 'platform', 4),
  N(6, 'clock.rs', 5),
  N(7, 'ax.rs', 5),
  N(8, 'lib.rs', 4),
  N(9, 'net', 4),
  N(10, 'tcp.rs', 9),
  N(11, 'examples', 3),
  N(12, 'ordo-core', 2),
  N(13, 'README.md', 1),
  N(14, 'hal', 5),
  N(15, 'gpio.rs', 14),
];
const [ORDO, SRC, PLATFORM, CLOCK, AX, LIB, NET, TCP, EXAMPLES, CORE, README, HAL, GPIO] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const CRATES = 2;
const leaves = [CLOCK, AX, LIB, TCP, EXAMPLES, CORE, README, GPIO];
const containers = new Set();
for (const id of leaves) for (let p = nodes[id].parent; p != null; p = nodes[p].parent) containers.add(p);
const chain = (leaf) => { const out = [leaf]; for (let p = nodes[leaf].parent; p != null && containers.has(p); p = nodes[p].parent) out.push(p); return out; };
const drawn = (ch, i) => (i === 0 ? String(ch[0]) : `c${ch[i]}`);

const edges = [[EXAMPLES, CLOCK], [AX, CLOCK], [LIB, CLOCK], [CLOCK, CORE], [README, TCP], [TCP, CLOCK], [README, GPIO], [EXAMPLES, GPIO]];
const M = (o) => new Map(Object.entries(o).map(([k, v]) => [Number(k), v]));
const resolve = (bundling) => edges.map(([from, to]) => {
  const cf = chain(from), ct = chain(to);
  const ends = bundleEnds(cf, ct, bundling);
  return ends && `${drawn(cf, ends[0])}>${drawn(ct, ends[1])}`;
});
const one = (from, to, bundling) => resolve(bundling)[edges.findIndex(([f, t]) => f === from && t === to)];

let passed = 0;
const test = (name, fn) => { fn(); passed++; console.log(`ok - ${name}`); };

test('regression guard: an empty map attaches every edge at the meeting level, exactly as before', () => {
  // Golden captured from the previous descend() over this same tree.
  assert.deepEqual(resolve(new Map()), ['11>c4', '7>6', '8>c5', 'c3>12', '13>c2', 'c9>c5', '13>c2', '11>c4']);
});

test('settings the old walk already honoured resolve the same way', () => {
  // Goldens from the previous descend(): sibling-level inward, a meeting
  // box's own in/out, and the common parent's inward.
  assert.equal(one(LIB, CLOCK, M({ [SRC]: { inward: 1 } })), '8>6');
  assert.equal(one(TCP, CLOCK, M({ [SRC]: { inward: 1 } })), '10>6');
  assert.equal(one(CLOCK, CORE, M({ [ORDO]: { out: 1 } })), 'c4>12');
  assert.equal(one(CLOCK, CORE, M({ [CRATES]: { inward: 1 } })), 'c4>12');
  assert.equal(one(EXAMPLES, CLOCK, M({ [SRC]: { in: 1 } })), '11>c5');
  assert.equal(one(EXAMPLES, CLOCK, M({ [ORDO]: { inward: 1 } })), '11>c5');
  assert.equal(one(EXAMPLES, CLOCK, M({ [SRC]: { in: 2 } })), '11>6');
  assert.equal(one(EXAMPLES, CLOCK, M({ [ORDO]: { inward: 2 } })), '11>6');
});

test('the reported bug: inward on a box pulls in an edge arriving from far outside it', () => {
  const b = M({ [SRC]: { inward: 1 } });
  // examples is a sibling of src; the edge used to stop at box:src.
  assert.equal(one(EXAMPLES, CLOCK, b), '11>6');
  // README is two levels further out still; the source end stays where it is.
  assert.equal(one(README, TCP, b), '13>10');
});

test('inward=1 reaches exactly one level into the sub box, never further', () => {
  // gpio.rs sits in hal inside platform: one level into platform is box:hal.
  assert.equal(one(README, GPIO, M({ [SRC]: { inward: 1 } })), '13>c14');
  assert.equal(one(EXAMPLES, GPIO, M({ [SRC]: { inward: 1 } })), '11>c14');
  assert.equal(one(README, GPIO, M({ [SRC]: { inward: 2 } })), '13>15');
});

test('inward on a box whose children are leaves has no sub box to reach into', () => {
  // net holds only tcp.rs; platform holds the sub box hal, so its inward
  // fires from any distance while every box above it is Off.
  assert.deepEqual(resolve(M({ [NET]: { inward: 2 } })), resolve(new Map()));
  assert.equal(one(README, TCP, M({ [SRC]: { inward: 1 }, [NET]: { inward: 2 } })), '13>10');
  assert.equal(one(README, GPIO, M({ [PLATFORM]: { inward: 1 } })), '13>15');
  assert.equal(one(EXAMPLES, CLOCK, M({ [PLATFORM]: { inward: 1 } })), '11>c4');
});

test('a deep box fires its own setting while every box above it is Off', () => {
  assert.equal(one(EXAMPLES, CLOCK, M({ [PLATFORM]: { in: 1 } })), '11>6');
  assert.equal(one(README, GPIO, M({ [PLATFORM]: { in: 1 } })), '13>c14');
  assert.equal(one(README, GPIO, M({ [HAL]: { in: 1 } })), '13>15');
  assert.equal(one(CLOCK, CORE, M({ [PLATFORM]: { out: 1 } })), '6>12');
  // Boxes the edge never crosses (the common parent and above) do not fire.
  assert.equal(one(LIB, CLOCK, M({ [SRC]: { in: 2, out: 2 }, [ORDO]: { in: 2, out: 2 } })), '8>c5');
  assert.equal(one(TCP, CLOCK, M({ [ORDO]: { inward: 2 } })), 'c9>c5');
});

test('rules naming different depths compose: the deepest wins', () => {
  assert.equal(one(README, GPIO, M({ [SRC]: { in: 1 } })), '13>c5');
  assert.equal(one(README, GPIO, M({ [SRC]: { in: 1 }, [HAL]: { in: 1 } })), '13>15');
  assert.equal(one(README, GPIO, M({ [SRC]: { in: 1 }, [PLATFORM]: { inward: 1 } })), '13>15');
  assert.equal(one(README, GPIO, M({ [CRATES]: { inward: 1 }, [PLATFORM]: { in: 1 } })), '13>c14');
  assert.equal(one(README, GPIO, M({ [CRATES]: { inward: 1 }, [PLATFORM]: { in: 2 } })), '13>15');
  // Both ends are resolved independently.
  assert.equal(one(CLOCK, CORE, M({ [PLATFORM]: { out: 1 }, [CRATES]: { inward: 1 } })), '6>12');
});

test('attachIndex over a bare chain', () => {
  const ch = [GPIO, HAL, PLATFORM, SRC, ORDO, CRATES, 1, 0];
  assert.equal(attachIndex(ch, 5, 'in', new Map()), 5);
  assert.equal(attachIndex(ch, 5, 'in', M({ [CRATES]: { in: 1 } })), 4);
  assert.equal(attachIndex(ch, 5, 'in', M({ [HAL]: { in: 1 } })), 0);
  assert.equal(attachIndex(ch, 5, 'in', M({ [HAL]: { out: 1 } })), 5);
  assert.equal(attachIndex(ch, 5, 'out', M({ [HAL]: { out: 1 } })), 0);
  assert.equal(attachIndex(ch, 5, 'in', M({ [ORDO]: { inward: 1 } })), 2);
  assert.equal(attachIndex(ch, 5, 'in', M({ [ORDO]: { inward: 2 } })), 0);
  assert.equal(attachIndex(ch, 0, 'in', M({ [HAL]: { in: 2 }, [PLATFORM]: { inward: 2 } })), 0);
});

test('bundleEnds: siblings meet at their leaves; a self edge never diverges', () => {
  assert.deepEqual(bundleEnds(chain(AX), chain(CLOCK), new Map()), [0, 0]);
  assert.deepEqual(bundleEnds(chain(EXAMPLES), chain(CLOCK), new Map()), [0, 2]);
  assert.equal(bundleEnds(chain(CLOCK), chain(CLOCK), new Map()), null);
  assert.deepEqual(bundleEnds([CLOCK], [CLOCK], new Map()), null);
});

console.log(`\n${passed} tests passed`);

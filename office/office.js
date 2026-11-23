/* Pigeon agent office.
   One canvas, drawn in code: an isometric pixel-art office at 480x320 logical
   pixels, scaled up with nearest-neighbour. status.json drives the people, the
   wall board and the HTML dashboard. No libraries, no network calls except
   status.json on the same origin. */
(() => {
  'use strict';

  // ---------- Config ----------
  const W = 480, H = 320;            // logical canvas size
  const TW = 16, TH = 8;             // half tile width / height (2:1 iso)
  const OX = 208, OY = 104;          // screen position of floor corner (0,0)
  const RX = 15, RY = 11;            // room size in tiles
  const WALL = 92;                   // wall height in px
  const DESK_Z = 13;                 // desk top height in px
  const FPS = 30;
  const REFRESH_MS = 60000;          // re-read status.json when served over http
  const STATUSES = ['working', 'done', 'waiting', 'idle'];
  const STATUS_LABEL = { working: 'Working', done: 'Done', waiting: 'Waiting for owner', idle: 'Idle' };
  const SCENE_STATUS = { working: '#7fd6e8', done: '#7bd88f', waiting: '#ff5a1f', idle: '#8a8f98' };

  const params = new URLSearchParams(location.search);
  const reduceMQ = window.matchMedia('(prefers-reduced-motion: reduce)');

  // ---------- Small helpers ----------
  const iso = (gx, gy, z = 0) => [OX + (gx - gy) * TW, OY + (gx + gy) * TH - z];
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const $ = (sel, root = document) => root.querySelector(sel);

  // Seeded PRNG so the scene is the same every load (and screenshots are reproducible).
  function rng(seed) {
    let s = seed >>> 0;
    return () => {
      s = (s + 0x6d2b79f5) >>> 0;
      let t = s;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function shade(hex, f) {
    const n = parseInt(hex.slice(1), 16);
    let r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
    if (f < 0) { r *= 1 + f; g *= 1 + f; b *= 1 + f; }
    else { r += (255 - r) * f; g += (255 - g) * f; b += (255 - b) * f; }
    return '#' + [r, g, b].map((v) => Math.round(v).toString(16).padStart(2, '0')).join('');
  }

  function mkCanvas(w, h) {
    const c = document.createElement('canvas');
    c.width = w; c.height = h;
    const x = c.getContext('2d');
    x.imageSmoothingEnabled = false;
    return [c, x];
  }

  // Crisp polygon fill: scanlines of whole pixels, no anti-aliasing.
  function poly(ctx, pts, color) {
    ctx.fillStyle = color;
    let minY = Infinity, maxY = -Infinity;
    for (const p of pts) { if (p[1] < minY) minY = p[1]; if (p[1] > maxY) maxY = p[1]; }
    minY = Math.round(minY); maxY = Math.round(maxY);
    const xs = [];
    for (let y = minY; y < maxY; y++) {
      const yc = y + 0.5;
      xs.length = 0;
      for (let i = 0; i < pts.length; i++) {
        const a = pts[i], b = pts[(i + 1) % pts.length];
        if ((a[1] <= yc && b[1] > yc) || (b[1] <= yc && a[1] > yc)) {
          xs.push(a[0] + ((yc - a[1]) * (b[0] - a[0])) / (b[1] - a[1]));
        }
      }
      xs.sort((p, q) => p - q);
      for (let k = 0; k + 1 < xs.length; k += 2) {
        const x0 = Math.round(xs[k]), x1 = Math.round(xs[k + 1]);
        if (x1 > x0) ctx.fillRect(x0, y, x1 - x0, 1);
      }
    }
  }

  function line(ctx, x0, y0, x1, y1, color) {
    ctx.fillStyle = color;
    const dx = x1 - x0, dy = y1 - y0;
    const n = Math.max(Math.abs(dx), Math.abs(dy), 1);
    for (let i = 0; i <= n; i++) ctx.fillRect(Math.round(x0 + (dx * i) / n), Math.round(y0 + (dy * i) / n), 1, 1);
  }

  // Iso box. Faces: top, "l" = the face at y+d (front-left), "r" = the face at x+w (front-right).
  function box(ctx, x, y, w, d, z, h, c) {
    const a = iso(x, y, z + h), b = iso(x + w, y, z + h), cc = iso(x + w, y + d, z + h), dd = iso(x, y + d, z + h);
    const b0 = iso(x + w, y, z), c0 = iso(x + w, y + d, z), d0 = iso(x, y + d, z);
    if (h > 0) {
      poly(ctx, [dd, cc, c0, d0], c.l);
      poly(ctx, [cc, b, b0, c0], c.r);
    }
    poly(ctx, [a, b, cc, dd], c.t);
    if (c.e) {
      line(ctx, dd[0], dd[1], cc[0] - 1, cc[1], c.e);
      line(ctx, cc[0], cc[1], b[0] - 1, b[1], c.e);
    }
  }
  const woodC = (base) => ({ t: base, l: shade(base, -0.25), r: shade(base, -0.42), e: shade(base, 0.18) });

  // Paint a flat image onto an iso face, column by column (2:1 shear).
  // face "l": plane y = const (right back wall, x-desk monitors), slope +1/2.
  // face "r": plane x = const (left back wall, y-desk monitors), slope -1/2.
  function shear(ctx, src, face, x, y, w, d, ztop) {
    const sw = src.width, sh = src.height;
    if (face === 'l') {
      const [sx, sy] = iso(x, y + d, ztop);
      for (let u = 0; u < sw; u++) ctx.drawImage(src, u, 0, 1, sh, Math.round(sx) + u, Math.round(sy) + (u >> 1), 1, sh);
    } else {
      const [sx, sy] = iso(x + w, y + d, ztop);
      for (let u = 0; u < sw; u++) ctx.drawImage(src, u, 0, 1, sh, Math.round(sx) + u, Math.round(sy) - (u >> 1), 1, sh);
    }
  }

  // ---------- 3x5 pixel font (original) ----------
  const FONT = {
    A: [2, 5, 7, 5, 5], B: [6, 5, 6, 5, 6], C: [3, 4, 4, 4, 3], D: [6, 5, 5, 5, 6], E: [7, 4, 6, 4, 7],
    F: [7, 4, 6, 4, 4], G: [3, 4, 5, 5, 3], H: [5, 5, 7, 5, 5], I: [7, 2, 2, 2, 7], J: [1, 1, 1, 5, 2],
    K: [5, 5, 6, 5, 5], L: [4, 4, 4, 4, 7], M: [5, 7, 7, 5, 5], N: [6, 5, 5, 5, 5], O: [2, 5, 5, 5, 2],
    P: [6, 5, 6, 4, 4], Q: [2, 5, 5, 6, 3], R: [6, 5, 6, 5, 5], S: [3, 4, 2, 1, 6], T: [7, 2, 2, 2, 2],
    U: [5, 5, 5, 5, 7], V: [5, 5, 5, 5, 2], W: [5, 5, 7, 7, 5], X: [5, 5, 2, 5, 5], Y: [5, 5, 2, 2, 2],
    Z: [7, 1, 2, 4, 7], 0: [7, 5, 5, 5, 7], 1: [2, 6, 2, 2, 7], 2: [6, 1, 2, 4, 7], 3: [6, 1, 2, 1, 6],
    4: [5, 5, 7, 1, 1], 5: [7, 4, 6, 1, 6], 6: [3, 4, 6, 5, 2], 7: [7, 1, 2, 2, 2], 8: [2, 5, 2, 5, 2],
    9: [2, 5, 3, 1, 6], ' ': [0, 0, 0, 0, 0], '.': [0, 0, 0, 0, 2], ':': [0, 2, 0, 2, 0], '-': [0, 0, 7, 0, 0],
    '/': [1, 1, 2, 4, 4], '#': [5, 7, 5, 7, 5], '!': [2, 2, 2, 0, 2], '?': [6, 1, 2, 0, 2], '+': [0, 2, 7, 2, 0],
    '(': [1, 2, 2, 2, 1], ')': [4, 2, 2, 2, 4], ',': [0, 0, 0, 2, 4], '>': [4, 2, 1, 2, 4], '%': [5, 1, 2, 4, 5],
  };
  function ptext(ctx, str, x, y, color) {
    ctx.fillStyle = color;
    let cx = x;
    for (const ch of String(str).toUpperCase()) {
      const g = FONT[ch] || FONT['?'];
      for (let r = 0; r < 5; r++) for (let c = 0; c < 3; c++) if ((g[r] >> (2 - c)) & 1) ctx.fillRect(cx + c, y + r, 1, 1);
      cx += 4;
    }
    return cx - x;
  }

  // ---------- Team: looks and desks (scene only; data comes from status.json) ----------
  const LOOKS = {
    atlas:  { skin: '#e0ac85', hair: '#2b2530', shirt: '#2d3a5c', pants: '#1d2233', shoe: '#14121a', extra: 'tie',     tall: 1 },
    anvil:  { skin: '#b7774f', hair: '#3a2417', shirt: '#b0542e', pants: '#3b3a3f', shoe: '#1d1a1a', extra: 'beanie',  hat: '#3a3d44' },
    canvas: { skin: '#f1c7a2', hair: '#8a3b25', shirt: '#2f8f84', pants: '#2b2f45', shoe: '#2a1c1c', extra: 'long' },
    plume:  { skin: '#d9a17a', hair: '#1d1a20', shirt: '#e8e2d6', pants: '#3a3550', shoe: '#1b1b22', extra: 'feather' },
    warden: { skin: '#8d5a3b', hair: '#151217', shirt: '#4f5d33', pants: '#262a1e', shoe: '#121212', extra: 'cap',     hat: '#20232a' },
    proof:  { skin: '#f0c9a8', hair: '#1a1416', shirt: '#d6dde3', pants: '#384252', shoe: '#1c1c22', extra: 'glasses' },
    frame:  { skin: '#c48a63', hair: '#5b3a22', shirt: '#d9a83a', pants: '#2c2b33', shoe: '#1a1414', extra: 'phones',  hat: '#2a2a33' },
  };
  const DEFAULT_LOOK = { skin: '#d8a882', hair: '#3b3035', shirt: '#6a6f7a', pants: '#2c2f38', shoe: '#18181c' };

  // x-desks run along x, screen faces +y, person sits at +y facing up-right.
  // y-desks run along y, screen faces +x, person sits at +x facing up-left.
  const DESKS = {
    atlas:  { o: 'x', x: 6.0,  y: 2.0, w: 2.4, d: 1, seat: [7.2, 3.55],  stand: [7.2, 4.4],  entry: [7.2, 5.0],  visit: [8.3, 3.8] },
    warden: { o: 'x', x: 10.6, y: 2.0, w: 2.0, d: 1, seat: [11.6, 3.55], stand: [11.6, 4.4], entry: [11.6, 5.0], visit: [12.6, 3.8] },
    anvil:  { o: 'y', x: 0.5,  y: 3.4, w: 1, d: 2.0, seat: [2.1, 4.4],   stand: [2.9, 4.6],  entry: [2.9, 5.0],  visit: [2.6, 5.4] },
    canvas: { o: 'y', x: 0.5,  y: 7.4, w: 1, d: 2.0, seat: [2.1, 8.4],   stand: [2.9, 8.7],  entry: [2.9, 9.4],  visit: [2.6, 9.4] },
    plume:  { o: 'x', x: 5.4,  y: 6.1, w: 2.0, d: 1, seat: [6.4, 7.65],  stand: [6.4, 8.6],  entry: [6.4, 9.4],  visit: [7.6, 7.95] },
    proof:  { o: 'x', x: 9.4,  y: 6.1, w: 2.0, d: 1, seat: [10.4, 7.65], stand: [10.4, 8.6], entry: [10.4, 9.4], visit: [11.6, 7.95] },
    frame:  { o: 'y', x: 12.6, y: 6.6, w: 1, d: 2.0, seat: [14.2, 7.6],  stand: [14.1, 9.0], entry: [14.1, 9.4], visit: [13.9, 9.0] },
  };
  const SPOTS = {
    coffee: { stand: [4.4, 1.25], entry: [4.3, 1.3], face: 'up' },
    books:  { stand: [1.5, 1.3],  entry: [1.5, 1.3], face: 'up' },
  };
  // Where people go when they get up (scene flavour only; text always comes from status.json).
  const WANDER = {
    proof: ['anvil', 'coffee', 'anvil', 'books'],
    warden: ['books', 'coffee', 'anvil'],
    anvil: ['coffee', 'books'],
    frame: ['coffee', 'plume'],
    canvas: ['coffee'],
  };
  // Corridor graph (grid points). Walkers move along straight segments between these.
  const NODES = {
    b1: [1.5, 1.3], c1: [4.3, 1.3], n1: [4.3, 5.0], n2: [8.4, 5.0], n3: [12.1, 5.0],
    s1: [4.3, 9.4], s2: [8.4, 9.4], s3: [12.1, 9.4], n0: [2.9, 5.0], s0: [2.9, 9.4], n4: [14.1, 5.0], s4: [14.1, 9.4],
  };
  const EDGES = [['b1', 'c1'], ['c1', 'n1'], ['n0', 'n1'], ['n1', 'n2'], ['n2', 'n3'], ['n3', 'n4'], ['n1', 's1'], ['n2', 's2'], ['n3', 's3'],
    ['s0', 's1'], ['s1', 's2'], ['s2', 's3'], ['s3', 's4']];

  function route(from, to) {
    // Dijkstra over corridor nodes plus two temporary nodes (start entry, goal entry)
    // linked to every node on the same row or column.
    const pts = Object.assign({}, NODES, { A: from, B: to });
    const adj = {};
    const link = (p, q) => {
      const d = Math.hypot(pts[p][0] - pts[q][0], pts[p][1] - pts[q][1]);
      (adj[p] = adj[p] || []).push([q, d]); (adj[q] = adj[q] || []).push([p, d]);
    };
    EDGES.forEach(([p, q]) => link(p, q));
    for (const t of ['A', 'B']) {
      for (const k of Object.keys(NODES)) {
        if (Math.abs(NODES[k][0] - pts[t][0]) < 0.05 || Math.abs(NODES[k][1] - pts[t][1]) < 0.05) link(t, k);
      }
    }
    if (Math.abs(from[0] - to[0]) < 0.05 || Math.abs(from[1] - to[1]) < 0.05) link('A', 'B');
    const dist = { A: 0 }, prev = {}, done = new Set();
    while (true) {
      let u = null;
      for (const k in dist) if (!done.has(k) && (u === null || dist[k] < dist[u])) u = k;
      if (u === null || u === 'B') break;
      done.add(u);
      for (const [v, d] of adj[u] || []) if (dist[v] === undefined || dist[u] + d < dist[v]) { dist[v] = dist[u] + d; prev[v] = u; }
    }
    if (dist.B === undefined) return [from, to];
    const out = [];
    for (let k = 'B'; k; k = prev[k]) out.unshift(pts[k]);
    return out;
  }

  // ---------- People: sprites drawn with rectangles ----------
  // Anchor (x, y) = floor point under the person. flip mirrors left/right.
  function personDraw(ctx, look, pose, x, y, t, opts = {}) {
    const f = opts.flip ? -1 : 1;
    const R = (dx, dy, w, h, c) => {
      ctx.fillStyle = c;
      ctx.fillRect(f === 1 ? x + dx : x - dx - w, y + dy, w, h);
    };
    const L = look;
    const tall = L.tall ? 1 : 0;
    const shirtD = shade(L.shirt, -0.28), skinD = shade(L.skin, -0.2), hairL = shade(L.hair, 0.15);

    if (pose === 'sit') {
      // Seated, seen from behind and a little from the side, facing the desk (up-right when f=1).
      const hy = -27 - tall;
      const typing = opts.typing ? ((Math.floor(t * 7) + (opts.phase || 0)) % 2) : 0;
      const lean = opts.relaxed ? 1 : 0;
      // hands on the keyboard, forward of the body
      R(4, -17 - typing, 2, 2, L.skin);
      R(-6, -18 + typing, 2, 2, skinD);
      // arms
      R(3, -20, 2, 4, shirtD);
      R(-5, -20, 2, 4, shirtD);
      // torso + shoulders
      R(-4, -21 - tall, 8, 8 + tall, L.shirt);
      R(-4, -21 - tall, 2, 8 + tall, shirtD);
      // neck, head
      R(-1, hy + 7, 3, 1, skinD);
      R(-3 - lean, hy, 7, 7, L.hair);
      R(-3 - lean, hy, 7, 1, hairL);
      R(3 - lean, hy + 3, 1, 2, L.skin);          // ear / cheek on the near side
      extraBack(R, L, hy, lean);
      return;
    }

    // Standing / walking
    const walking = pose === 'walk';
    const step = walking ? Math.floor(t * 6 + (opts.phase || 0)) % 2 : 0;
    const back = !!opts.back;
    const hy = -28 - tall;
    // shoes + legs
    R(-3, -1 - (step ? 1 : 0), 3, 1, L.shoe);
    R(1, -1 - (step ? 0 : 1), 3, 1, L.shoe);
    R(-3, -9 - tall, 3, 8 + tall - (step ? 1 : 0), L.pants);
    R(1, -9 - tall, 3, 8 + tall - (step ? 0 : 1), shade(L.pants, -0.15));
    // torso
    R(-4, -18 - tall, 9, 9, L.shirt);
    R(-4, -18 - tall, 2, 9, shirtD);
    // arms swing
    const sw = walking ? (step ? 1 : -1) : 0;
    R(-6, -17 - tall + sw, 2, 7, shirtD);
    R(5, -17 - tall - sw, 2, 7, L.shirt);
    R(-6, -10 - tall + sw, 2, 2, skinD);
    R(5, -10 - tall - sw, 2, 2, L.skin);
    if (!back && L.extra === 'tie') { R(0, -18 - tall, 1, 1, '#e8e2d6'); R(1, -18 - tall, 1, 1, '#e8e2d6'); R(0, -17 - tall, 2, 5, '#8c2a35'); }
    // neck + head
    R(-1, hy + 8, 3, 2, skinD);
    if (back) {
      R(-3, hy, 7, 8, L.hair);
      R(-3, hy, 7, 1, hairL);
      extraBack(R, L, hy + 1, 0);
    } else {
      R(-3, hy, 7, 8, L.skin);
      R(-3, hy, 7, 2, L.hair);
      R(-3, hy + 2, 1, 2, L.hair);
      R(3, hy + 2, 1, 1, L.hair);
      R(-1, hy + 4, 1, 1, '#1a1418');
      R(2, hy + 4, 1, 1, '#1a1418');
      R(0, hy + 6, 2, 1, skinD);
      extraFront(R, L, hy);
    }
  }
  function extraBack(R, L, hy, lean) {
    const x0 = -3 - lean;
    switch (L.extra) {
      case 'beanie': R(x0, hy - 1, 7, 3, L.hat); R(x0, hy + 1, 7, 1, shade(L.hat, -0.3)); break;
      case 'cap': R(x0, hy - 1, 7, 3, L.hat); R(x0 + 5, hy + 1, 3, 1, shade(L.hat, -0.3)); break;
      case 'long': R(x0, hy + 6, 7, 5, L.hair); R(x0, hy + 11, 5, 2, L.hair); break;
      case 'feather': R(x0 + 2, hy - 2, 3, 2, L.hair); R(x0 + 4, hy - 4, 1, 2, '#f4efe6'); R(x0 + 5, hy - 6, 1, 3, '#f4efe6'); R(x0 + 6, hy - 7, 1, 2, '#d8d0c2'); break;
      case 'phones': R(x0, hy - 1, 7, 1, L.hat); R(x0 - 1, hy + 2, 2, 3, L.hat); R(x0 + 6, hy + 2, 2, 3, L.hat); break;
      case 'glasses': R(x0, hy - 1, 2, 1, L.hair); R(x0 + 3, hy - 1, 2, 1, L.hair); R(x0 + 6, hy + 3, 1, 1, '#9aa6b2'); break;
      case 'tie': R(x0, hy, 7, 2, shade(L.hair, 0.1)); break;
      default: break;
    }
  }
  function extraFront(R, L, hy) {
    switch (L.extra) {
      case 'beanie': R(-3, hy - 1, 7, 3, L.hat); R(-3, hy + 1, 7, 1, shade(L.hat, -0.3)); break;
      case 'cap': R(-3, hy - 1, 7, 2, L.hat); R(-4, hy + 1, 9, 1, shade(L.hat, -0.3)); break;
      case 'long': R(-4, hy + 1, 1, 8, L.hair); R(4, hy + 1, 1, 8, L.hair); break;
      case 'feather': R(-1, hy - 2, 3, 2, L.hair); R(3, hy - 4, 1, 3, '#f4efe6'); R(4, hy - 6, 1, 3, '#f4efe6'); break;
      case 'phones': R(-3, hy - 1, 7, 1, L.hat); R(-4, hy + 2, 2, 3, L.hat); R(3, hy + 2, 2, 3, L.hat); break;
      case 'glasses': R(-2, hy + 4, 2, 1, '#3a4652'); R(1, hy + 4, 3, 1, '#3a4652'); R(0, hy + 4, 1, 1, '#9aa6b2'); break;
      default: break;
    }
  }

  // Office chair at a seat. back=true draws only the backrest (drawn over a seated person).
  function chairDraw(ctx, sx, sy, o, part) {
    const f = o === 'x' ? 1 : -1;
    const R = (dx, dy, w, h, c) => { ctx.fillStyle = c; ctx.fillRect(f === 1 ? sx + dx : sx - dx - w, sy + dy, w, h); };
    if (part !== 'back') {
      R(-4, -1, 9, 1, '#1b1820');
      R(0, -7, 1, 6, '#2b2730');
      R(-5, -10, 10, 3, '#3b3446');
      R(-5, -8, 10, 1, '#2a2433');
    }
    if (part !== 'seat') {
      R(-6, -14, 11, 5, '#463d55');
      R(-6, -14, 11, 1, '#5b5070');
      R(-6, -10, 11, 1, '#2f2939');
      R(-6, -14, 1, 5, '#352e40');
    }
  }

  // ---------- Furniture ----------
  function deskDraw(ctx, D, t, agent) {
    const { x, y, w, d, o } = D;
    const top = woodC('#b07a4a');
    const leg = { t: '#5a3a24', l: '#4a2f1d', r: '#3b2416' };
    const ins = 0.08;
    box(ctx, x + ins, y + ins, 0.12, 0.12, 0, DESK_Z - 2, leg);
    box(ctx, x + w - 0.2, y + ins, 0.12, 0.12, 0, DESK_Z - 2, leg);
    box(ctx, x + ins, y + d - 0.2, 0.12, 0.12, 0, DESK_Z - 2, leg);
    box(ctx, x + w - 0.2, y + d - 0.2, 0.12, 0.12, 0, DESK_Z - 2, leg);
    // drawer block under one end
    if (o === 'x') box(ctx, x + w - 0.75, y + 0.15, 0.6, d - 0.3, 0, DESK_Z - 2, woodC('#8d5e38'));
    else box(ctx, x + 0.15, y + 0.15, w - 0.3, 0.6, 0, DESK_Z - 2, woodC('#8d5e38'));
    box(ctx, x, y, w, d, DESK_Z - 2, 2, top);

    const z = DESK_Z;
    // lamp at the far end, monitor, keyboard, mug
    if (o === 'x') {
      lampDraw(ctx, x + w - 0.35, y + 0.3, z);
      monitorDraw(ctx, x + w / 2 - 0.55, y + 0.12, 1.0, 0.55, z, 'l', t, agent);
      box(ctx, x + w / 2 - 0.45, y + 0.78, 0.8, 0.18, z, 1, { t: '#d8d0bf', l: '#a69c88', r: '#8f8674' });
      box(ctx, x + 0.25, y + 0.55, 0.22, 0.22, z, 4, { t: '#3a2d2a', l: '#e9e1d3', r: '#c9bfb0' });
      box(ctx, x + 0.2, y + 0.15, 0.45, 0.32, z, 1, { t: '#efe8da', l: '#cfc6b5', r: '#b8ae9c' });
    } else {
      lampDraw(ctx, x + 0.3, y + 0.3, z);
      monitorDraw(ctx, x + 0.12, y + d / 2 - 0.5, 0.55, 1.0, z, 'r', t, agent);
      box(ctx, x + 0.78, y + d / 2 - 0.4, 0.18, 0.8, z, 1, { t: '#d8d0bf', l: '#a69c88', r: '#8f8674' });
      box(ctx, x + 0.55, y + d - 0.45, 0.22, 0.22, z, 4, { t: '#3a2d2a', l: '#e9e1d3', r: '#c9bfb0' });
    }
  }

  function lampDraw(ctx, gx, gy, z) {
    const [bx, by] = iso(gx, gy, z);
    ctx.fillStyle = '#2b2622'; ctx.fillRect(bx - 2, by - 1, 5, 1);
    line(ctx, bx, by - 1, bx, by - 9, '#3d3631');
    line(ctx, bx, by - 9, bx + 3, by - 12, '#3d3631');
    ctx.fillStyle = '#2f5a4a'; ctx.fillRect(bx + 1, by - 14, 6, 3);
    ctx.fillStyle = '#3f7a64'; ctx.fillRect(bx + 2, by - 15, 4, 1);
    ctx.fillStyle = '#ffd99a'; ctx.fillRect(bx + 2, by - 11, 4, 1);
  }

  // Per-agent screen buffers (flat, then sheared onto the monitor face).
  const screens = {};
  function screenBuf(id, fw, fh) {
    if (!screens[id] || screens[id][0].width !== fw) screens[id] = mkCanvas(fw, fh).concat([rng(id.length * 977 + 13)]);
    return screens[id];
  }
  function monitorDraw(ctx, x, y, w, d, z, face, t, agent) {
    const body = { t: '#cfc4ad', l: '#b3a78f', r: '#968a73', e: '#e2d8c3' };
    box(ctx, x + (face === 'l' ? 0.3 : 0.15), y + (face === 'l' ? 0.15 : 0.3), face === 'l' ? 0.4 : 0.25, face === 'l' ? 0.25 : 0.4, z, 2, body);
    const mz = z + 2, mh = 12;
    box(ctx, x, y, w, d, mz, mh, body);
    // screen on the facing side
    const fw = Math.round((face === 'l' ? w : d) * 16) - 4;
    const fh = mh - 4;
    const [buf, bx, rand] = screenBuf(agent ? agent.id : 'x', fw, fh);
    paintScreen(bx, fw, fh, t, agent, rand);
    if (face === 'l') {
      const [sx, sy] = iso(x, y + d, mz + mh);
      for (let u = 0; u < fw; u++) ctx.drawImage(buf, u, 0, 1, fh, Math.round(sx) + 2 + u, Math.round(sy) + 2 + ((u + 2) >> 1), 1, fh);
    } else {
      const [sx, sy] = iso(x + w, y + d, mz + mh);
      for (let u = 0; u < fw; u++) ctx.drawImage(buf, u, 0, 1, fh, Math.round(sx) + 2 + u, Math.round(sy) + 2 - ((u + 2) >> 1), 1, fh);
    }
  }

  function paintScreen(c, w, h, t, agent, rand) {
    const st = agent ? agent.status : 'idle';
    const flick = 0.92 + 0.08 * Math.sin(t * 37 + w);
    c.fillStyle = st === 'idle' ? '#0d1416' : '#0b1a1c';
    c.fillRect(0, 0, w, h);
    if (st === 'working') {
      const scroll = Math.floor(t * 4);
      const colors = ['#7fd6e8', '#e8d27f', '#c39bff', '#9fe39a', '#e89f7f'];
      for (let r = 0; r < Math.ceil(h / 2) + 1; r++) {
        const n = scroll + r;
        const ind = (n * 7) % 3;
        const len = 2 + ((n * 13) % (w - 4));
        c.fillStyle = colors[(n * 3) % colors.length];
        c.globalAlpha = flick;
        c.fillRect(1 + ind, r * 2, Math.min(len, w - 2 - ind), 1);
      }
      c.globalAlpha = 1;
      if (Math.floor(t * 2) % 2) { c.fillStyle = '#e8f6f8'; c.fillRect(1, h - 2, 2, 1); }
    } else if (st === 'done') {
      c.fillStyle = '#7bd88f';
      const cx = Math.floor(w / 2) - 3, cy = Math.floor(h / 2) - 2;
      c.fillRect(cx, cy + 2, 1, 1); c.fillRect(cx + 1, cy + 3, 1, 1); c.fillRect(cx + 2, cy + 4, 1, 1);
      c.fillRect(cx + 3, cy + 3, 1, 1); c.fillRect(cx + 4, cy + 2, 1, 1); c.fillRect(cx + 5, cy + 1, 1, 1); c.fillRect(cx + 6, cy, 1, 1);
    } else if (st === 'waiting') {
      const on = Math.floor(t * 1.5) % 2 === 0;
      c.fillStyle = on ? '#ff5a1f' : '#7a2e14';
      c.fillRect(Math.floor(w / 2) - 1, Math.floor(h / 2) - 1, 3, 3);
      c.fillStyle = '#5b6f73';
      c.fillRect(2, h - 2, w - 4, 1);
    } else {
      // idle: a slow drifting dot
      const px = Math.floor((Math.sin(t * 0.7 + w) * 0.5 + 0.5) * (w - 2));
      const py = Math.floor((Math.cos(t * 0.5) * 0.5 + 0.5) * (h - 2));
      c.fillStyle = '#3b5a60'; c.fillRect(px, py, 2, 1);
    }
  }

  function plantDraw(ctx, gx, gy, t, big) {
    box(ctx, gx, gy, 0.5, 0.5, 0, big ? 9 : 6, { t: '#5a3424', l: '#b5603b', r: '#8f4a2d', e: '#d07a50' });
    const [px, py] = iso(gx + 0.25, gy + 0.25, big ? 9 : 6);
    const sway = Math.round(Math.sin(t * 1.3 + gx) * 0.6);
    const leaves = big
      ? [[-1, -22, 3, 8], [-5, -17, 4, 6], [2, -18, 4, 6], [-6, -11, 5, 4], [2, -12, 5, 4], [-3, -8, 6, 4], [-2, -26, 2, 5]]
      : [[-4, -10, 3, 5], [1, -11, 3, 6], [-2, -14, 3, 7], [-5, -5, 4, 3], [1, -5, 4, 3]];
    leaves.forEach(([dx, dy, w, h], i) => {
      const s = dy < -12 ? sway : 0;
      ctx.fillStyle = i % 2 ? '#3f7a3f' : '#4f9448';
      ctx.fillRect(px + dx + s, py + dy, w, h);
      ctx.fillStyle = '#69b55c';
      ctx.fillRect(px + dx + s, py + dy, Math.max(1, w - 2), 1);
    });
  }

  // ---------- Wall art (flat canvases, sheared onto walls) ----------
  function bookshelfFront(w, h, seed) {
    const [c, x] = mkCanvas(w, h);
    const r = rng(seed);
    x.fillStyle = '#4b301f'; x.fillRect(0, 0, w, h);
    const shelves = 4, sh = Math.floor((h - 2) / shelves);
    const bookCols = ['#8c2a35', '#2f5a7a', '#d9a83a', '#3f7a64', '#c9bfb0', '#6a4a8a', '#b5603b', '#1f3b4d'];
    for (let s = 0; s < shelves; s++) {
      const y0 = 1 + s * sh;
      x.fillStyle = '#2a1a10'; x.fillRect(1, y0, w - 2, sh - 1);
      let bx = 2;
      while (bx < w - 3) {
        const bw = 1 + Math.floor(r() * 2), bh = sh - 3 - Math.floor(r() * 3);
        if (r() < 0.12) { bx += 2; continue; }
        x.fillStyle = bookCols[Math.floor(r() * bookCols.length)];
        x.fillRect(bx, y0 + sh - 1 - bh, bw, bh);
        bx += bw + (r() < 0.3 ? 1 : 0);
      }
      x.fillStyle = '#7a5236'; x.fillRect(0, y0 + sh - 1, w, 1);
    }
    return c;
  }

  function windowFlat(c, w, h, t, idx) {
    // evening sky, city, the odd pigeon flying past
    const g = c.createLinearGradient(0, 0, 0, h);
    g.addColorStop(0, '#2b2457'); g.addColorStop(0.45, '#7a3f6e'); g.addColorStop(0.8, '#e07a4a'); g.addColorStop(1, '#f2b061');
    c.fillStyle = g; c.fillRect(0, 0, w, h);
    // dither the gradient a little so it reads as pixels
    c.fillStyle = 'rgba(20,10,40,0.18)';
    for (let yy = 0; yy < h; yy += 2) for (let xx = (yy >> 1) % 2; xx < w; xx += 2) if (((xx * 7 + yy * 3) % 5) === 0) c.fillRect(xx, yy, 1, 1);
    // sun
    if (idx === 0) { c.fillStyle = '#ffd27a'; c.fillRect(w - 14, h - 14, 6, 5); c.fillRect(w - 13, h - 15, 4, 7); }
    // skyline
    const r = rng(77 + idx);
    let x0 = 0;
    while (x0 < w) {
      const bw = 3 + Math.floor(r() * 6), bh = 8 + Math.floor(r() * 18);
      c.fillStyle = r() < 0.5 ? '#1e1530' : '#261a38';
      c.fillRect(x0, h - bh, bw, bh);
      for (let wy = h - bh + 2; wy < h - 1; wy += 3) for (let wx = x0 + 1; wx < x0 + bw - 1; wx += 2) {
        if (r() < 0.25) { const on = Math.sin(t * 0.6 + wx * 3 + wy) > -0.7; c.fillStyle = on ? '#ffcf73' : '#5a3d3d'; c.fillRect(wx, wy, 1, 1); }
      }
      x0 += bw;
    }
    // a pigeon crosses window 0 every ~14 s
    const period = 14, p = ((t + idx * 6) % period) / 3.2;
    if (p < 1) {
      const bx = Math.round(-4 + p * (w + 8)), by = Math.round(10 + Math.sin(p * 9) * 2);
      const flap = Math.floor(t * 8) % 2;
      c.fillStyle = '#1a1424';
      c.fillRect(bx, by, 3, 1); c.fillRect(bx + 3, by - 1, 1, 1);
      if (flap) { c.fillRect(bx - 1, by - 1, 2, 1); c.fillRect(bx + 1, by - 2, 1, 1); } else { c.fillRect(bx, by + 1, 2, 1); }
    }
    // frame + mullions
    c.fillStyle = '#6b4a35';
    c.fillRect(0, 0, w, 2); c.fillRect(0, h - 3, w, 3); c.fillRect(0, 0, 2, h); c.fillRect(w - 2, 0, 2, h);
    c.fillRect(Math.floor(w / 2) - 1, 0, 2, h); c.fillRect(0, Math.floor(h / 2) - 1, w, 2);
    c.fillStyle = '#8a6448'; c.fillRect(0, h - 3, w, 1);
  }

  function chartFlat(c, w, h, counts, t) {
    c.fillStyle = '#6b4a35'; c.fillRect(0, 0, w, h);
    c.fillStyle = '#efe9dc'; c.fillRect(2, 2, w - 4, h - 4);
    ptext(c, 'TEAM', 4, 4, '#2b2530');
    const max = Math.max(1, ...STATUSES.map((s) => counts[s] || 0));
    const bw = 5, gap = 3, base = h - 10, x0 = 5;
    STATUSES.forEach((s, i) => {
      const n = counts[s] || 0;
      const bh = Math.round((n / max) * (base - 13));
      c.fillStyle = SCENE_STATUS[s] === '#8a8f98' ? '#8a8f98' : shade(SCENE_STATUS[s], -0.15);
      c.fillRect(x0 + i * (bw + gap), base - bh, bw, bh);
      ptext(c, String(n), x0 + i * (bw + gap) + 1, base + 2, '#2b2530');
    });
    c.fillStyle = '#2b2530'; c.fillRect(4, base, w - 8, 1);
    // marker pen tray
    c.fillStyle = '#8a8f98'; c.fillRect(6, h - 2, w - 12, 1);
  }

  function boardFlat(c, w, h, agents, queueLen, t) {
    c.fillStyle = '#16141b'; c.fillRect(0, 0, w, h);
    c.fillStyle = '#0a0f14'; c.fillRect(2, 2, w - 4, h - 4);
    ptext(c, 'PIGEON TEAM', 5, 4, '#e8f0f2');
    const label = 'OWNER ' + queueLen;
    const lw = label.length * 4;
    ptext(c, label, w - 5 - lw, 4, '#ff8a5c');
    if (queueLen && Math.floor(t * 1.5) % 2 === 0) { c.fillStyle = '#ff5a1f'; c.fillRect(w - 9 - lw, 5, 3, 3); }
    c.fillStyle = '#2a3440'; c.fillRect(4, 11, w - 8, 1);
    agents.slice(0, 7).forEach((a, i) => {
      const y = 14 + i * 6;
      let col = SCENE_STATUS[a.status] || '#8a8f98';
      if (a.status === 'working' && Math.floor(t * 3 + i) % 3 === 0) col = shade(col, 0.4);
      c.fillStyle = col; c.fillRect(5, y + 1, 3, 3);
      ptext(c, a.name.slice(0, 7), 11, y, '#cfd8dc');
      const s = { working: 'WORKING', done: 'DONE', waiting: 'WAITING', idle: 'IDLE' }[a.status] || '';
      ptext(c, s, w - 5 - s.length * 4, y, col);
      if (a.status === 'working') {
        const bar = (Math.floor(t * 6) + i * 3) % 12;
        c.fillStyle = '#24404a'; c.fillRect(44, y + 2, 14, 1);
        c.fillStyle = '#7fd6e8'; c.fillRect(44 + Math.min(bar, 12), y + 2, 2, 1);
      }
    });
    // scanline flicker
    const sl = Math.floor(t * 20) % h;
    c.fillStyle = 'rgba(255,255,255,0.05)'; c.fillRect(2, sl, w - 4, 1);
  }

  function clockFlat(c, w, h) {
    c.clearRect(0, 0, w, h);
    const cx = w / 2, cy = h / 2, r = w / 2 - 1;
    c.fillStyle = '#3a2a22';
    for (let yy = 0; yy < h; yy++) for (let xx = 0; xx < w; xx++) {
      const d = Math.hypot(xx + 0.5 - cx, yy + 0.5 - cy);
      if (d <= r) { c.fillStyle = d > r - 1.5 ? '#6b4a35' : '#efe9dc'; c.fillRect(xx, yy, 1, 1); }
    }
    const now = new Date();
    const hr = ((now.getHours() % 12) + now.getMinutes() / 60) / 12 * Math.PI * 2;
    const mn = now.getMinutes() / 60 * Math.PI * 2;
    line(c, Math.round(cx - 0.5), Math.round(cy - 0.5), Math.round(cx - 0.5 + Math.sin(hr) * (r - 4)), Math.round(cy - 0.5 - Math.cos(hr) * (r - 4)), '#2b2530');
    line(c, Math.round(cx - 0.5), Math.round(cy - 0.5), Math.round(cx - 0.5 + Math.sin(mn) * (r - 2)), Math.round(cy - 0.5 - Math.cos(mn) * (r - 2)), '#8c2a35');
  }

  // ---------- Static layer: floor, walls, built-ins ----------
  const BOARD = { a: 5.2, b: 11.7, z0: 34, z1: 90 };       // right wall
  const CHART = { a: 0.9, b: 3.4, z0: 32, z1: 70 };        // left wall
  const WINDOWS = [{ a: 4.3, b: 6.5, z0: 28, z1: 80 }, { a: 7.7, b: 9.9, z0: 28, z1: 80 }];
  const CLOCK = { a: 13.2, b: 14.2, z0: 58, z1: 74 };

  function drawStatic(ctx) {
    ctx.fillStyle = '#0f0b12'; ctx.fillRect(0, 0, W, H);
    // walls
    const lw = [iso(0, 0, WALL), iso(0, RY, WALL), iso(0, RY, 0), iso(0, 0, 0)];
    const rw = [iso(0, 0, WALL), iso(RX, 0, WALL), iso(RX, 0, 0), iso(0, 0, 0)];
    poly(ctx, lw, '#3a2c3c');
    poly(ctx, rw, '#4a3746');
    // wallpaper stripes
    for (let gy = 0.5; gy < RY; gy += 1) {
      const p = iso(0, gy, WALL), q = iso(0, gy, 30);
      line(ctx, p[0], p[1], q[0], q[1], '#35283a');
    }
    for (let gx = 0.5; gx < RX; gx += 1) {
      const p = iso(gx, 0, WALL), q = iso(gx, 0, 30);
      line(ctx, p[0], p[1], q[0], q[1], '#523e4f');
    }
    // wainscot
    poly(ctx, [iso(0, 0, 26), iso(0, RY, 26), iso(0, RY, 0), iso(0, 0, 0)], '#2c2030');
    poly(ctx, [iso(0, 0, 26), iso(RX, 0, 26), iso(RX, 0, 0), iso(0, 0, 0)], '#38293a');
    const t1 = iso(0, RY, 27), t2 = iso(0, 0, 27), t3 = iso(RX, 0, 27);
    line(ctx, t1[0], t1[1], t2[0], t2[1], '#6b4a3a'); line(ctx, t2[0], t2[1], t3[0] - 1, t3[1], '#7a5644');
    const u1 = iso(0, RY, 28), u2 = iso(0, 0, 28), u3 = iso(RX, 0, 28);
    line(ctx, u1[0], u1[1], u2[0], u2[1], '#4f3a33'); line(ctx, u2[0], u2[1], u3[0] - 1, u3[1], '#5e463c');
    // wall caps
    poly(ctx, [iso(0, 0, WALL + 3), iso(0, RY, WALL + 3), iso(-0.3, RY, WALL), iso(-0.3, 0, WALL)], '#5b4656');
    poly(ctx, [iso(-0.3, -0.3, WALL + 3), iso(RX, -0.3, WALL + 3), iso(RX, 0, WALL), iso(0, 0, WALL + 3)], '#6a5364');
    // corner
    const c0 = iso(0, 0, WALL + 3), c1 = iso(0, 0, 0);
    line(ctx, c0[0], c0[1], c1[0], c1[1], '#2a1f2b');

    // floor planks
    const r = rng(42);
    const woods = ['#6b4630', '#74503a', '#654230', '#7a543b'];
    for (let gy = 0; gy < RY; gy++) {
      for (let gx = 0; gx < RX; gx++) {
        const col = woods[Math.floor(r() * woods.length)];
        poly(ctx, [iso(gx, gy), iso(gx + 1, gy), iso(gx + 1, gy + 1), iso(gx, gy + 1)], col);
        // plank seam along x, half way
        const a = iso(gx, gy + 0.5), b = iso(gx + 1, gy + 0.5);
        line(ctx, a[0], a[1], b[0] - 1, b[1], shade(col, -0.18));
        const s = iso(gx, gy), e = iso(gx + 1, gy);
        line(ctx, s[0], s[1], e[0] - 1, e[1], '#4f3324');
        if (r() < 0.25) { const k = iso(gx + 0.3 + r() * 0.4, gy + 0.25); ctx.fillStyle = shade(col, -0.25); ctx.fillRect(k[0], k[1], 2, 1); }
      }
    }
    // floor edges (room thickness)
    poly(ctx, [iso(0, RY), iso(RX, RY), iso(RX, RY, -6), iso(0, RY, -6)], '#3a2618');
    poly(ctx, [iso(RX, 0), iso(RX, RY), iso(RX, RY, -6), iso(RX, 0, -6)], '#2c1c12');
    // rug in the walkway
    poly(ctx, [iso(4.6, 4.5), iso(12.6, 4.5), iso(12.6, 5.7), iso(4.6, 5.7)], '#2e4547');
    poly(ctx, [iso(4.85, 4.62), iso(12.35, 4.62), iso(12.35, 5.58), iso(4.85, 5.58)], '#365457');
    for (let gx = 5.2; gx < 12.3; gx += 0.6) { const p = iso(gx, 5.1); ctx.fillStyle = '#4a6e6e'; ctx.fillRect(p[0], p[1], 2, 1); }
    // rug at the front desks
    poly(ctx, [iso(4.9, 8.3), iso(12.0, 8.3), iso(12.0, 9.9), iso(4.9, 9.9)], '#5a2f34');
    poly(ctx, [iso(5.1, 8.45), iso(11.8, 8.45), iso(11.8, 9.75), iso(5.1, 9.75)], '#6b3a3e');

    // static wall art
    const shW = Math.round(2.2 * 16), shH = 58;
    box(ctx, 0.3, 0, 2.2, 0.55, 0, shH, woodC('#5e3d27'));
    shear(ctx, bookshelfFront(shW, shH - 2, 5), 'l', 0.3, 0, 2.2, 0.55, shH - 1);
    box(ctx, 13.0, 0, 1.8, 0.6, 0, 26, woodC('#6d6a64'));
    const [dc, dx] = mkCanvas(29, 24);
    dx.fillStyle = '#5a5751'; dx.fillRect(0, 0, 29, 24);
    for (let k = 0; k < 3; k++) { dx.fillStyle = '#7b7870'; dx.fillRect(1, 1 + k * 8, 27, 7); dx.fillStyle = '#3b3934'; dx.fillRect(11, 4 + k * 8, 7, 1); }
    shear(ctx, dc, 'l', 13.0, 0, 1.8, 0.6, 25);
    // coffee counter
    box(ctx, 3.6, 0, 1.5, 0.65, 0, 18, woodC('#5e3d27'));
    box(ctx, 3.75, 0.08, 0.55, 0.42, 18, 12, { t: '#2a2a2e', l: '#3a3a40', r: '#232327', e: '#55555c' });
    box(ctx, 4.5, 0.15, 0.4, 0.35, 18, 4, { t: '#e9e1d3', l: '#c9bfb0', r: '#a89e8f' });
    // poster on right wall
    const [pc, px] = mkCanvas(20, 26);
    px.fillStyle = '#e9e1d3'; px.fillRect(0, 0, 20, 26);
    px.fillStyle = '#0b0b0b'; px.fillRect(1, 1, 18, 24);
    // our own little pigeon mark: geometric bird in flight
    px.fillStyle = '#f4efe6';
    px.fillRect(5, 12, 8, 3); px.fillRect(12, 10, 3, 3); px.fillRect(14, 11, 2, 1);
    px.fillRect(6, 9, 4, 3); px.fillRect(8, 6, 3, 3); px.fillRect(3, 14, 3, 2);
    px.fillStyle = '#ff5a1f'; px.fillRect(13, 11, 1, 1);
    ptext(px, 'HOME', 2, 18, '#f4efe6');
    shear(ctx, pc, 'l', 2.95, -0.02, 20 / 16, 0, 78);
  }

  // ---------- State ----------
  let data = null, dataSource = 'fallback';
  let people = [];         // scene actors (one per known agent)
  let selected = null;
  let simT = 0;
  let playing = true;
  const R0 = rng(2026);

  function counts() {
    const c = {};
    (data ? data.agents : []).forEach((a) => { c[a.status] = (c[a.status] || 0) + 1; });
    return c;
  }

  function buildPeople() {
    const prev = new Map(people.map((p) => [p.id, p]));
    people = data.agents.filter((a) => DESKS[a.id]).map((a, i) => {
      const old = prev.get(a.id);
      const D = DESKS[a.id];
      const p = old || { id: a.id, gx: D.seat[0], gy: D.seat[1], mode: 'sit', path: null, wait: 0, next: 0, phase: i };
      p.agent = a; p.desk = D; p.look = LOOKS[a.id] || DEFAULT_LOOK;
      if (!old) p.next = firstMove(a);
      return p;
    });
  }
  function firstMove(a) {
    if (a.id === 'atlas' && a.status === 'working') return 1.2;
    if (a.id === 'proof') return 0.4;
    if (WANDER[a.id] && a.status !== 'working') return 8 + R0() * 14;
    return Infinity;
  }

  function targetOf(key) {
    if (SPOTS[key]) return { stand: SPOTS[key].stand, entry: SPOTS[key].entry, face: SPOTS[key].face };
    const D = DESKS[key];
    if (D) return { stand: D.visit, entry: [D.visit[0] === D.entry[0] ? D.entry[0] : D.visit[0], D.entry[1]], faceAt: D.seat, key };
    return null;
  }

  function planTrip(p, key) {
    const tgt = targetOf(key);
    if (!tgt) return;
    const D = p.desk;
    let entry = tgt.entry;
    // keep the approach on a corridor line
    if (DESKS[key]) entry = DESKS[key].entry.slice();
    const there = [D.seat, D.stand, D.entry].concat(route(D.entry, entry).slice(1, -1), [entry, tgt.stand]);
    const back = [tgt.stand, entry].concat(route(entry, D.entry).slice(1, -1), [D.entry, D.stand, D.seat]);
    p.legs = [{ path: there, then: 'stand', hold: p.id === 'atlas' ? 3.2 : 4.5 + R0() * 3, tgt }, { path: back, then: 'sit' }];
    startLeg(p);
  }
  function startLeg(p) {
    const leg = p.legs.shift();
    p.path = leg.path.map((q) => q.slice());
    p.seg = 0;
    p.mode = 'walk';
    p.after = leg;
  }

  function step(dt) {
    simT += dt;
    const speed = 2.1; // tiles per second
    for (const p of people) {
      const a = p.agent;
      if (p.mode === 'sit') {
        if (simT >= p.next) {
          let key = null;
          if (a.id === 'atlas' && a.status === 'working') {
            const others = people.filter((q) => q.id !== 'atlas' && q.mode === 'sit').map((q) => q.id);
            p.cursor = ((p.cursor || 0) + 1) % Math.max(1, others.length);
            key = others[p.cursor];
          } else if (WANDER[a.id] && a.status !== 'working' && a.status !== 'waiting') {
            const list = WANDER[a.id];
            p.cursor = ((p.cursor === undefined ? -1 : p.cursor) + 1) % list.length;
            key = list[p.cursor];
            // do not stand where someone else is already standing
            if (people.some((q) => q !== p && q.after && q.after.tgt && targetKey(q) === key)) key = null;
          }
          if (key) planTrip(p, key);
          else p.next = simT + 6 + R0() * 6;
        }
      } else if (p.mode === 'walk') {
        let left = speed * dt;
        while (left > 0 && p.path && p.seg < p.path.length - 1) {
          const [tx, ty] = p.path[p.seg + 1];
          const dx = tx - p.gx, dy = ty - p.gy;
          const dist = Math.hypot(dx, dy);
          if (dist <= left) { p.gx = tx; p.gy = ty; p.seg++; left -= dist; }
          else { p.gx += (dx / dist) * left; p.gy += (dy / dist) * left; left = 0; }
          p.dir = [dx, dy];
        }
        if (p.seg >= p.path.length - 1) {
          if (p.after.then === 'stand') { p.mode = 'stand'; p.wait = p.after.hold; p.standTgt = p.after.tgt; }
          else { p.mode = 'sit'; p.after = null; p.legs = null; p.gx = p.desk.seat[0]; p.gy = p.desk.seat[1]; p.next = simT + nextGap(a); }
        }
      } else if (p.mode === 'stand') {
        p.wait -= dt;
        if (p.wait <= 0) startLeg(p);
      }
    }
  }
  function targetKey(q) { const t = q.after && q.after.tgt; return t ? (t.key || Object.keys(SPOTS).find((k) => SPOTS[k].stand === t.stand)) : null; }
  function nextGap(a) {
    if (a.id === 'atlas') return 5 + R0() * 4;
    if (a.id === 'proof') return 9 + R0() * 8;
    return 14 + R0() * 16;
  }

  // ---------- Render ----------
  const canvas = $('#scene');
  const [buf, bx] = mkCanvas(W, H);
  const [stat, sx] = mkCanvas(W, H);
  const out = canvas.getContext('2d');
  let staticReady = false;
  const wallBufs = {
    board: mkCanvas(Math.round((BOARD.b - BOARD.a) * 16), BOARD.z1 - BOARD.z0),
    chart: mkCanvas(Math.round((CHART.b - CHART.a) * 16), CHART.z1 - CHART.z0),
    win0: mkCanvas(Math.round((WINDOWS[0].b - WINDOWS[0].a) * 16), WINDOWS[0].z1 - WINDOWS[0].z0),
    win1: mkCanvas(Math.round((WINDOWS[1].b - WINDOWS[1].a) * 16), WINDOWS[1].z1 - WINDOWS[1].z0),
    clock: mkCanvas(16, 16),
  };
  const DUST = Array.from({ length: 46 }, (_, i) => { const r = rng(300 + i); return { w: i % 2, u: r(), v: r(), s: 0.2 + r() * 0.5, ph: r() * 6.28 }; });

  // Beam: window corners on the left wall projected onto the floor along the light direction.
  const LIGHT = [0.062, 0.018];   // grid offset per px of height
  function beamShape(win) {
    const corners = [[win.a, win.z0], [win.b, win.z0], [win.b, win.z1], [win.a, win.z1]];
    const wallPts = corners.map(([gy, z]) => iso(0, gy, z));
    const floorPts = corners.map(([gy, z]) => iso(z * LIGHT[0], gy + z * LIGHT[1], 0));
    return { wallPts, floorPts, hull: hull(wallPts.concat(floorPts)) };
  }
  function hull(pts) {
    const p = pts.slice().sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    const cross = (o, a, b) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    const lo = [], up = [];
    for (const q of p) { while (lo.length >= 2 && cross(lo[lo.length - 2], lo[lo.length - 1], q) <= 0) lo.pop(); lo.push(q); }
    for (let i = p.length - 1; i >= 0; i--) { const q = p[i]; while (up.length >= 2 && cross(up[up.length - 2], up[up.length - 1], q) <= 0) up.pop(); up.push(q); }
    return lo.slice(0, -1).concat(up.slice(0, -1));
  }
  const BEAMS = WINDOWS.map(beamShape);
  const pathPoly = (ctx, pts) => { ctx.beginPath(); pts.forEach((q, i) => (i ? ctx.lineTo(q[0], q[1]) : ctx.moveTo(q[0], q[1]))); ctx.closePath(); };

  function personScreen(p) { return iso(p.gx, p.gy).map(Math.round); }

  function render(t) {
    if (!staticReady) { drawStatic(sx); staticReady = true; }
    const ctx = bx;
    ctx.globalCompositeOperation = 'source-over';
    ctx.globalAlpha = 1;
    ctx.drawImage(stat, 0, 0);
    const agents = data ? data.agents : [];

    // dynamic wall pieces
    const [wb, wbx] = wallBufs.board;
    boardFlat(wbx, wb.width, wb.height, agents, data ? data.owner_queue.length : 0, t);
    shear(ctx, wb, 'l', BOARD.a, 0, BOARD.b - BOARD.a, 0, BOARD.z1);
    const [cb, cbx] = wallBufs.chart;
    chartFlat(cbx, cb.width, cb.height, counts(), t);
    shear(ctx, cb, 'r', 0, CHART.a, 0, CHART.b - CHART.a, CHART.z1);
    WINDOWS.forEach((wnd, i) => {
      const [c, cx] = wallBufs['win' + i];
      windowFlat(cx, c.width, c.height, t, i);
      shear(ctx, c, 'r', 0, wnd.a, 0, wnd.b - wnd.a, wnd.z1);
    });
    const [kc, kcx] = wallBufs.clock;
    if (!kc.drawnAt || t - kc.drawnAt > 20 || t < kc.drawnAt) { clockFlat(kcx, 16, 16); kc.drawnAt = t; }
    shear(ctx, kc, 'l', CLOCK.a, 0, CLOCK.b - CLOCK.a, 0, CLOCK.z1);

    // floor light patches (under furniture)
    ctx.globalCompositeOperation = 'lighter';
    BEAMS.forEach((b) => { pathPoly(ctx, b.floorPts); ctx.fillStyle = 'rgba(255,150,70,0.10)'; ctx.fill(); });
    ctx.globalCompositeOperation = 'source-over';

    // sortable drawables
    const items = [];
    for (const id in DESKS) {
      const D = DESKS[id];
      const a = agents.find((q) => q.id === id);
      items.push({ depth: D.x + D.w / 2 + D.y + D.d / 2, draw: () => deskDraw(ctx, D, t, a || { id, status: 'idle' }) });
      const p = people.find((q) => q.id === id);
      const seated = p && p.mode === 'sit';
      const [cx, cy] = iso(D.seat[0], D.seat[1]).map(Math.round);
      items.push({
        depth: D.seat[0] + D.seat[1],
        draw: () => {
          chairDraw(ctx, cx, cy, D.o, 'seat');
          if (seated) {
            const st = p.agent.status;
            personDraw(ctx, p.look, 'sit', cx, cy - 1, t, { flip: D.o === 'y', typing: st === 'working', relaxed: st === 'done', phase: p.phase });
          }
          chairDraw(ctx, cx, cy, D.o, 'back');
        },
      });
    }
    for (const p of people) {
      if (p.mode === 'sit') continue;
      const [px, py] = personScreen(p);
      let back = false, flip = false;
      if (p.mode === 'walk' && p.dir) {
        const vx = p.dir[0] - p.dir[1], vy = p.dir[0] + p.dir[1];
        back = vy < -0.01; flip = vx < 0;
      } else if (p.mode === 'stand') {
        const tg = p.standTgt;
        back = true;
        if (tg && tg.faceAt) {
          const [fx] = iso(tg.faceAt[0], tg.faceAt[1]);
          flip = fx < px;
          const [, fy] = iso(tg.faceAt[0], tg.faceAt[1]);
          back = fy < py;
        }
      }
      items.push({ depth: p.gx + p.gy + 0.05, draw: () => personDraw(ctx, p.look, p.mode === 'walk' ? 'walk' : 'stand', px, py, t, { back, flip, phase: p.phase }) });
    }
    const plants = [[0.25, 10.2, true], [2.75, 0.25, true], [12.3, 0.3, false], [14.3, 10.3, true], [5.0, 3.0, false]];
    plants.forEach(([x, y, big]) => items.push({ depth: x + y + 0.5, draw: () => plantDraw(ctx, x, y, t, big) }));
    items.sort((a, b) => a.depth - b.depth);
    items.forEach((it) => it.draw());

    // selection ring
    if (selected) {
      const p = people.find((q) => q.id === selected);
      if (p) {
        const [px, py] = personScreen(p);
        const pulse = Math.floor(t * 3) % 2;
        ctx.fillStyle = pulse ? '#ffffff' : '#ffd8c8';
        const rx = 9, ry = 4;
        for (let a = 0; a < 40; a++) {
          const ang = (a / 40) * Math.PI * 2;
          const y = Math.round(py + Math.sin(ang) * ry);
          if (Math.sin(ang) < -0.2) continue;
          ctx.fillRect(Math.round(px + Math.cos(ang) * rx), y, 1, 1);
        }
      }
    }

    // light: beams, lamps, monitors, board
    ctx.globalCompositeOperation = 'lighter';
    BEAMS.forEach((b, i) => {
      pathPoly(ctx, b.hull);
      ctx.fillStyle = `rgba(255,160,80,${0.045 + 0.01 * Math.sin(t * 0.8 + i)})`;
      ctx.fill();
    });
    const glow = (x, y, r, rgb, a) => {
      const g = ctx.createRadialGradient(x, y, 0, x, y, r);
      g.addColorStop(0, `rgba(${rgb},${a})`); g.addColorStop(1, `rgba(${rgb},0)`);
      ctx.fillStyle = g; ctx.fillRect(x - r, y - r, r * 2, r * 2);
    };
    for (const id in DESKS) {
      const D = DESKS[id];
      const lp = D.o === 'x' ? iso(D.x + D.w - 0.35, D.y + 0.3, DESK_Z) : iso(D.x + 0.3, D.y + 0.3, DESK_Z);
      const fl = 0.2 + 0.025 * Math.sin(t * 3.1 + D.x * 2) + (Math.sin(t * 17 + D.y) > 0.97 ? -0.05 : 0);
      glow(lp[0] + 4, lp[1] - 6, 34, '255,190,110', fl);
      const a = agents.find((q) => q.id === id);
      const sp = D.o === 'x' ? iso(D.x + D.w / 2, D.y + 0.7, DESK_Z + 8) : iso(D.x + 0.7, D.y + D.d / 2, DESK_Z + 8);
      const col = { working: '127,214,232', done: '123,216,143', waiting: '255,90,31', idle: '80,110,120' }[a ? a.status : 'idle'];
      glow(sp[0], sp[1], 16, col, 0.16 + 0.03 * Math.sin(t * 9 + D.x));
    }
    const bp = iso((BOARD.a + BOARD.b) / 2, 0.3, (BOARD.z0 + BOARD.z1) / 2);
    glow(bp[0], bp[1], 60, '120,190,220', 0.1);
    // dust in the beams
    DUST.forEach((d) => {
      const b = BEAMS[d.w];
      const u = (d.u + t * 0.01 * d.s) % 1, v = (d.v + Math.sin(t * 0.3 * d.s + d.ph) * 0.04 + 1) % 1;
      const wp = b.wallPts, fp = b.floorPts;
      // bilinear-ish point between wall window and floor patch
      const top = [wp[3][0] + (wp[2][0] - wp[3][0]) * u, wp[3][1] + (wp[2][1] - wp[3][1]) * u];
      const bot = [fp[0][0] + (fp[1][0] - fp[0][0]) * u, fp[0][1] + (fp[1][1] - fp[0][1]) * u];
      const k = 0.15 + v * 0.8;
      const x = Math.round(top[0] + (bot[0] - top[0]) * k + v * 10), y = Math.round(top[1] + (bot[1] - top[1]) * k);
      const tw = 0.25 + 0.35 * (0.5 + 0.5 * Math.sin(t * 2 * d.s + d.ph));
      ctx.fillStyle = `rgba(255,225,170,${tw})`;
      ctx.fillRect(x, y, 1, 1);
    });
    ctx.globalCompositeOperation = 'source-over';
    // vignette
    const vg = ctx.createRadialGradient(W / 2, H / 2, H * 0.35, W / 2, H / 2, H * 0.95);
    vg.addColorStop(0, 'rgba(10,6,14,0)'); vg.addColorStop(1, 'rgba(10,6,14,0.4)');
    ctx.fillStyle = vg; ctx.fillRect(0, 0, W, H);

    // blit to the visible canvas at an integer scale
    out.imageSmoothingEnabled = false;
    out.drawImage(buf, 0, 0, canvas.width, canvas.height);
    placeTags();
  }

  // ---------- Canvas sizing ----------
  const stage = $('#stage');
  function fitCanvas() {
    const cssW = stage.clientWidth || W;
    const k = Math.max(1, Math.ceil((cssW / W) * (window.devicePixelRatio || 1)));
    if (canvas.width !== W * k) { canvas.width = W * k; canvas.height = H * k; }
    stage.classList.toggle('is-compact', cssW < 820);
    tagSizeDirty = true;
    if (!playing) render(simT);
  }

  // ---------- Tags above heads (HTML, so text stays sharp and readable) ----------
  const tagLayer = $('#tags');
  const tagEls = new Map();
  let tagSizeDirty = true;

  function bubbleText(a) {
    if (a.task) return a.task;
    if (a.status === 'idle') return 'Idle' + (a.last_result && a.last_result.text ? ' · last: ' + a.last_result.text : '');
    return STATUS_LABEL[a.status] || '';
  }

  function buildTags() {
    tagLayer.textContent = '';
    tagEls.clear();
    data.agents.forEach((a) => {
      if (!DESKS[a.id]) return;
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'tag';
      b.dataset.id = a.id;
      b.dataset.status = a.status;
      b.setAttribute('aria-label', `${a.name}, ${a.role}. ${STATUS_LABEL[a.status] || a.status}. ${bubbleText(a)}`);
      b.setAttribute('aria-expanded', 'false');
      const head = document.createElement('span');
      head.className = 'tag-head';
      head.appendChild(statusIcon(a.status));
      const nm = document.createElement('b'); nm.textContent = a.name; head.appendChild(nm);
      const rl = document.createElement('span'); rl.className = 'tag-role'; rl.textContent = ' · ' + a.role; head.appendChild(rl);
      const bub = document.createElement('span');
      bub.className = 'tag-bubble';
      const bt = document.createElement('span'); bt.className = 'tag-text'; bt.textContent = bubbleText(a); bub.appendChild(bt);
      b.append(head, bub);
      b.addEventListener('click', () => select(a.id, { from: 'scene' }));
      tagLayer.appendChild(b);
      tagEls.set(a.id, { el: b, w: 0, h: 0, x: null, y: null });
    });
    tagSizeDirty = true;
  }

  function placeTags() {
    const sw = stage.clientWidth, sh = stage.clientHeight;
    if (!sw) return;
    const s = sw / W;
    if (tagSizeDirty) {
      tagEls.forEach((t) => { t.el.style.transform = 'translate(0,0)'; t.w = t.el.offsetWidth; t.h = t.el.offsetHeight; t.x = null; });
      tagSizeDirty = false;
    }
    const rects = [];
    people.forEach((p) => {
      const t = tagEls.get(p.id);
      if (!t) return;
      const [px, py] = iso(p.gx, p.gy);
      const headTop = (p.mode === 'sit' ? py - 32 : py - 34) - (p.look.tall ? 1 : 0);
      const ax = px * s, ay = headTop * s;
      rects.push({ t, ax, ay, x: clamp(ax - t.w / 2, 4, sw - t.w - 4), y: ay - t.h - 6, w: t.w, h: t.h, sel: p.id === selected });
    });
    // push overlapping tags apart: the one further back moves up
    rects.sort((a, b) => a.ay - b.ay);
    for (let it = 0; it < 6; it++) {
      for (let i = 0; i < rects.length; i++) for (let j = i + 1; j < rects.length; j++) {
        const A = rects[i], B = rects[j];
        const m = 4;
        const ox = Math.min(A.x + A.w, B.x + B.w) - Math.max(A.x, B.x) + m;
        const oy = Math.min(A.y + A.h, B.y + B.h) - Math.max(A.y, B.y) + m;
        if (ox > 0 && oy > 0) {
          if (ox < oy * 0.8) {
            const push = ox / 2;
            if (A.x < B.x) { A.x -= push; B.x += push; } else { A.x += push; B.x -= push; }
          } else {
            A.y -= oy;
          }
          A.x = clamp(A.x, 4, sw - A.w - 4); B.x = clamp(B.x, 4, sw - B.w - 4);
        }
      }
      rects.forEach((r) => { if (r.y < 4) r.y = 4; });
    }
    rects.forEach((r) => {
      const t = r.t;
      const x = Math.round(r.x), y = Math.round(r.y);
      const stem = Math.max(0, Math.round(r.ay - (y + r.h)));
      const tail = clamp(Math.round(r.ax - x), 10, r.w - 10);
      if (t.x !== x || t.y !== y || t.stem !== stem || t.tail !== tail) {
        t.el.style.transform = `translate(${x}px, ${y}px)`;
        t.el.style.setProperty('--stem', stem + 'px');
        t.el.style.setProperty('--tail', tail + 'px');
        t.x = x; t.y = y; t.stem = stem; t.tail = tail;
      }
      t.el.classList.toggle('is-selected', r.sel);
    });
  }

  function statusIcon(status) {
    const s = document.createElement('span');
    s.className = 'st st--' + status;
    s.setAttribute('aria-hidden', 'true');
    return s;
  }

  // ---------- Dashboard panel ----------
  const fmtTime = (iso8601) => {
    const d = new Date(iso8601);
    if (isNaN(d)) return { abs: '', rel: '' };
    const abs = d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
    const diff = Math.max(0, (Date.now() - d.getTime()) / 1000);
    let rel;
    if (diff < 60) rel = 'just now';
    else if (diff < 3600) rel = Math.round(diff / 60) + ' min ago';
    else if (diff < 86400) rel = Math.round(diff / 3600) + ' h ago';
    else rel = Math.round(diff / 86400) + ' d ago';
    return { abs, rel };
  };
  const safeUrl = (u) => (typeof u === 'string' && /^https:\/\//i.test(u) ? u : null);

  const portraits = {};
  function portrait(id) {
    if (portraits[id]) return portraits[id];
    const [c, x] = mkCanvas(16, 16);
    x.fillStyle = '#2a2230'; x.fillRect(0, 0, 16, 16);
    personDraw(x, LOOKS[id] || DEFAULT_LOOK, 'stand', 8, 33 + ((LOOKS[id] || {}).tall ? 1 : 0), 0, {});
    try { portraits[id] = c.toDataURL('image/png'); } catch (e) { portraits[id] = ''; }
    return portraits[id];
  }

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined && text !== null) e.textContent = text;
    return e;
  }
  function link(text, url) {
    const u = safeUrl(url);
    if (!u) return el('span', null, text);
    const a = el('a', null, text);
    a.href = u; a.rel = 'noopener'; a.target = '_blank';
    return a;
  }

  function renderBoard() {
    const list = $('#agents');
    list.textContent = '';
    data.agents.forEach((a) => {
      const li = el('li', 'agent');
      li.id = 'agent-' + a.id;
      li.dataset.id = a.id;
      li.dataset.status = a.status;
      const head = el('button', 'agent-head');
      head.type = 'button';
      head.setAttribute('aria-expanded', 'false');
      head.setAttribute('aria-controls', 'more-' + a.id);
      const img = el('img', 'avatar');
      img.src = portrait(a.id); img.alt = ''; img.width = 32; img.height = 32;
      const who = el('span', 'agent-who');
      who.append(el('span', 'agent-name', a.name), el('span', 'agent-role', a.role));
      const badge = el('span', 'badge badge--' + a.status);
      badge.append(statusIcon(a.status), document.createTextNode(STATUS_LABEL[a.status] || a.status));
      head.append(img, who, badge);
      head.addEventListener('click', () => select(selected === a.id ? null : a.id, { from: 'board' }));

      const body = el('div', 'agent-body');
      const task = el('p', 'agent-task');
      task.append(el('span', 'k', 'Now'), document.createTextNode(a.task || (a.status === 'idle' ? 'Nothing assigned' : '')));
      const res = el('p', 'agent-result');
      res.append(el('span', 'k', 'Last'));
      if (a.last_result && a.last_result.text) res.append(link(a.last_result.text, a.last_result.url));
      else res.append(document.createTextNode('Nothing yet'));
      const tm = fmtTime(a.updated_at);
      const when = el('p', 'agent-time');
      const time = el('time', null, tm.rel);
      time.dateTime = a.updated_at || '';
      time.title = tm.abs;
      when.append(el('span', 'k', 'Updated'), time);
      body.append(task, res, when);

      const more = el('div', 'agent-more');
      more.id = 'more-' + a.id;
      more.hidden = true;
      if (a.about) more.append(el('p', null, a.about));
      const abs = el('p', 'mono', 'Updated ' + tm.abs + (a.updated_at ? ' (' + a.updated_at.replace('T', ' ').replace('Z', ' UTC') + ')' : ''));
      more.append(abs);
      if (!DESKS[a.id]) more.append(el('p', null, 'No desk in the office yet.'));
      li.append(head, body, more);
      list.appendChild(li);
    });

    const q = $('#queue');
    q.textContent = '';
    (data.owner_queue || []).forEach((item) => {
      const it = typeof item === 'string' ? { text: item } : item;
      const li = el('li', 'queue-item');
      li.append(statusIcon('waiting'));
      const txt = el('span', 'queue-text');
      txt.append(link(it.text, it.url));
      const from = data.agents.find((a) => a.id === it.from);
      if (from) txt.append(el('span', 'queue-from', ' · from ' + from.name));
      li.append(txt);
      q.appendChild(li);
    });
    $('#queue-count').textContent = String((data.owner_queue || []).length);
    $('#queue-empty').hidden = (data.owner_queue || []).length > 0;

    const c = counts();
    $('#summary').textContent = STATUSES.filter((s) => c[s]).map((s) => `${c[s]} ${s === 'waiting' ? 'waiting' : s}`).join(' · ');
    const tm = fmtTime(data.updated_at);
    const up = $('#updated');
    up.textContent = '';
    const tEl = el('time', null, tm.abs);
    tEl.dateTime = data.updated_at || '';
    up.append(document.createTextNode('Updated '), tEl, document.createTextNode(` (${tm.rel})`));
    $('#source').textContent = dataSource === 'live' ? 'from status.json' : 'from the copy embedded in this page';
    if (selected) select(selected, { from: 'refresh' });
  }

  function select(id, { from } = {}) {
    selected = id && data.agents.some((a) => a.id === id) ? id : null;
    document.querySelectorAll('.agent').forEach((li) => {
      const on = li.dataset.id === selected;
      li.classList.toggle('is-selected', on);
      const h = li.querySelector('.agent-head');
      h.setAttribute('aria-expanded', String(on));
      li.querySelector('.agent-more').hidden = !on;
    });
    tagEls.forEach((t, k) => {
      t.el.setAttribute('aria-expanded', String(k === selected));
      t.el.classList.toggle('is-open', k === selected);
    });
    tagSizeDirty = true;
    if (selected && from === 'scene') {
      const li = document.getElementById('agent-' + selected);
      const compact = stage.classList.contains('is-compact');
      if (li && !compact) li.scrollIntoView({ block: 'nearest', behavior: reduceMQ.matches ? 'auto' : 'smooth' });
    }
    const live = $('#selection');
    if (live && from !== 'refresh') {
      const a = data.agents.find((q) => q.id === selected);
      live.textContent = a ? `${a.name} selected: ${STATUS_LABEL[a.status]}. ${bubbleText(a)}` : '';
    }
    if (!playing) render(simT);
  }

  // Canvas hit test: click a person to select them.
  canvas.addEventListener('click', (e) => {
    const r = canvas.getBoundingClientRect();
    const lx = ((e.clientX - r.left) / r.width) * W, ly = ((e.clientY - r.top) / r.height) * H;
    let best = null, bd = 1e9;
    people.forEach((p) => {
      const [px, py] = iso(p.gx, p.gy);
      const top = py - 34;
      if (lx > px - 9 && lx < px + 9 && ly > top && ly < py + 2) {
        const d = Math.abs(lx - px) + Math.abs(ly - (py - 16));
        if (d < bd) { bd = d; best = p.id; }
      }
    });
    select(best, { from: 'scene' });
  });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && selected) select(null, { from: 'key' }); });

  // ---------- Data ----------
  function normalise(d) {
    if (!d || !Array.isArray(d.agents)) throw new Error('status: agents missing');
    d.agents = d.agents
      .filter((a) => a && typeof a.id === 'string' && typeof a.name === 'string')
      .map((a) => Object.assign({}, a, { status: STATUSES.includes(a.status) ? a.status : 'idle', role: a.role || '' }));
    d.owner_queue = Array.isArray(d.owner_queue) ? d.owner_queue : [];
    return d;
  }
  function fallback() {
    const n = document.getElementById('status-fallback');
    return normalise(JSON.parse(n.textContent));
  }
  async function load() {
    // From file:// a fetch is blocked by the browser (and logs a CORS error), so do not try.
    if (location.protocol === 'file:') return { d: fallback(), src: 'fallback' };
    try {
      const r = await fetch('status.json', { cache: 'no-store' });
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return { d: normalise(await r.json()), src: 'live' };
    } catch (e) {
      return { d: fallback(), src: 'fallback' };
    }
  }
  async function refresh() {
    const { d, src } = await load();
    const changed = !data || JSON.stringify(d) !== JSON.stringify(data);
    data = d; dataSource = src;
    if (changed) { buildPeople(); buildTags(); renderBoard(); }
    else renderBoard();
  }

  // ---------- Controls ----------
  const playBtn = $('#play');
  function setPlaying(on) {
    playing = on;
    playBtn.setAttribute('aria-pressed', String(!on));
    playBtn.textContent = on ? 'Pause animation' : 'Play animation';
    stage.classList.toggle('is-paused', !on);
    if (on) { last = performance.now(); requestAnimationFrame(loop); } else render(simT);
  }
  playBtn.addEventListener('click', () => setPlaying(!playing));

  const themeBtn = $('#theme');
  const root = document.documentElement;
  function applyTheme(mode) {
    if (mode === 'light' || mode === 'dark') root.dataset.theme = mode; else delete root.dataset.theme;
    const isDark = mode ? mode === 'dark' : window.matchMedia('(prefers-color-scheme: dark)').matches;
    themeBtn.textContent = isDark ? 'Light theme' : 'Dark theme';
    themeBtn.setAttribute('aria-label', isDark ? 'Switch to light theme' : 'Switch to dark theme');
  }
  let themePref = params.get('theme');
  if (!themePref) { try { themePref = localStorage.getItem('office-theme'); } catch (e) { themePref = null; } }
  applyTheme(themePref);
  themeBtn.addEventListener('click', () => {
    const isDark = root.dataset.theme ? root.dataset.theme === 'dark' : window.matchMedia('(prefers-color-scheme: dark)').matches;
    themePref = isDark ? 'light' : 'dark';
    applyTheme(themePref);
    try { localStorage.setItem('office-theme', themePref); } catch (e) { /* private mode: fine */ }
  });

  // ---------- Loop ----------
  let last = 0, acc = 0;
  function loop(now) {
    if (!playing) return;
    const dt = Math.min(0.1, (now - last) / 1000);
    last = now;
    acc += dt;
    if (acc >= 1 / FPS) {
      const stepDt = acc; acc = 0;
      step(stepDt);
      render(simT);
    }
    requestAnimationFrame(loop);
  }

  async function start() {
    await refresh();
    // ?at=SECONDS jumps the simulation forward (for reproducible screenshots).
    const at = parseFloat(params.get('at'));
    if (at > 0) { const n = Math.round(at * 20); for (let i = 0; i < n; i++) step(0.05); }
    fitCanvas();
    if ('ResizeObserver' in window) new ResizeObserver(fitCanvas).observe(stage);
    else window.addEventListener('resize', fitCanvas);
    const still = reduceMQ.matches || params.get('still') === '1';
    if (params.get('select')) select(params.get('select'), { from: 'param' });
    setPlaying(!still);
    reduceMQ.addEventListener && reduceMQ.addEventListener('change', (e) => setPlaying(!e.matches));
    if (location.protocol !== 'file:') setInterval(refresh, REFRESH_MS);
    setInterval(() => data && renderBoard(), 30000);
  }
  start();
})();

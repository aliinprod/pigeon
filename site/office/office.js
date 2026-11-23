/* Pigeon agent office, v2.
   An isometric pixel-art office drawn in code on a 440x324 canvas and scaled up with
   nearest-neighbour. status.json drives the people, the wall screen, the whiteboard and
   the HTML dashboard. No libraries; the only request is status.json on the same origin.

   Scene rules (see office/README.md, "Design references"):
   - the room is the hero: no text sits on the art by default, only a tiny status marker
     above each head; names/tasks appear in one tooltip on hover/tap, or as small chips
     when "Show names" is on;
   - status is shown by behaviour first (typing, reading, sipping, walking), marker second;
   - warm lamps and window light against a cool ambient, monitors add cool glow. */
(() => {
  'use strict';

  // ---------- Geometry ----------
  const W = 440, H = 324;
  const TW = 20, TH = 10;            // half tile (2:1 iso)
  const OX = 190, OY = 110;          // screen position of floor corner (0,0)
  const RX = 12, RY = 9;             // room in tiles
  const WALL = 100;
  const DESK_Z = 16;
  const FPS = 30;
  const REFRESH_MS = 60000;
  const STATUSES = ['working', 'done', 'waiting', 'idle'];
  const STATUS_LABEL = { working: 'Working', done: 'Done', waiting: 'Waiting for owner', idle: 'Idle' };
  const SCENE_STATUS = { working: '#7fd6e8', done: '#7bd88f', waiting: '#ff5a1f', idle: '#9aa0aa' };

  const params = new URLSearchParams(location.search);
  const reduceMQ = window.matchMedia('(prefers-reduced-motion: reduce)');
  const iso = (gx, gy, z = 0) => [OX + (gx - gy) * TW, OY + (gx + gy) * TH - z];
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const $ = (s, r = document) => r.querySelector(s);

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
    if (f < 0) { r *= 1 + f; g *= 1 + f; b *= 1 + f; } else { r += (255 - r) * f; g += (255 - g) * f; b += (255 - b) * f; }
    return '#' + [r, g, b].map((v) => Math.round(v).toString(16).padStart(2, '0')).join('');
  }
  function mkCanvas(w, h, read) {
    const c = document.createElement('canvas');
    c.width = Math.max(1, w); c.height = Math.max(1, h);
    const x = c.getContext('2d', read ? { willReadFrequently: true } : undefined);
    x.imageSmoothingEnabled = false;
    return [c, x];
  }

  // Crisp polygon fill: whole-pixel scanlines, no anti-aliasing.
  function poly(ctx, pts, color) {
    ctx.fillStyle = color;
    let minY = Infinity, maxY = -Infinity;
    for (const p of pts) { if (p[1] < minY) minY = p[1]; if (p[1] > maxY) maxY = p[1]; }
    minY = Math.round(minY); maxY = Math.round(maxY);
    const xs = [];
    for (let y = minY; y < maxY; y++) {
      const yc = y + 0.5; xs.length = 0;
      for (let i = 0; i < pts.length; i++) {
        const a = pts[i], b = pts[(i + 1) % pts.length];
        if ((a[1] <= yc && b[1] > yc) || (b[1] <= yc && a[1] > yc)) xs.push(a[0] + ((yc - a[1]) * (b[0] - a[0])) / (b[1] - a[1]));
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
    const dx = x1 - x0, dy = y1 - y0, n = Math.max(Math.abs(dx), Math.abs(dy), 1);
    for (let i = 0; i <= n; i++) ctx.fillRect(Math.round(x0 + (dx * i) / n), Math.round(y0 + (dy * i) / n), 1, 1);
  }
  const isoLine = (ctx, a, b, c) => { const p = iso(...a), q = iso(...b); line(ctx, p[0], p[1], q[0], q[1], c); };

  // Iso box: top face lightest, "l" (face at y+d) mid, "r" (face at x+w) darkest.
  function box(ctx, x, y, w, d, z, h, c) {
    const a = iso(x, y, z + h), b = iso(x + w, y, z + h), cc = iso(x + w, y + d, z + h), dd = iso(x, y + d, z + h);
    const b0 = iso(x + w, y, z), c0 = iso(x + w, y + d, z), d0 = iso(x, y + d, z);
    if (h > 0) { poly(ctx, [dd, cc, c0, d0], c.l); poly(ctx, [cc, b, b0, c0], c.r); }
    poly(ctx, [a, b, cc, dd], c.t);
    if (c.e) { line(ctx, dd[0], dd[1], cc[0] - 1, cc[1], c.e); line(ctx, cc[0], cc[1], b[0] - 1, b[1], c.e); }
  }
  // Hue-shifted ramp: shadows drift cooler, highlights warmer.
  const mat = (base) => ({ t: base, l: mix(shade(base, -0.22), '#3a3060', 0.12), r: mix(shade(base, -0.4), '#2a2450', 0.18), e: mix(shade(base, 0.16), '#ffd9a0', 0.15) });
  function mix(a, b, f) {
    const p = parseInt(a.slice(1), 16), q = parseInt(b.slice(1), 16);
    const ch = (s) => Math.round(((p >> s) & 255) * (1 - f) + ((q >> s) & 255) * f);
    return '#' + [ch(16), ch(8), ch(0)].map((v) => v.toString(16).padStart(2, '0')).join('');
  }

  // Paint a flat image onto an iso plane, one pixel column at a time.
  // 'l': plane y = const (right back wall, x-desk screens), slope +1/2.
  // 'r': plane x = const (left back wall, y-desk screens), slope -1/2.
  function shear(ctx, src, face, gx, gy, ztop, dx = 0, dy = 0) {
    const sw = src.width, sh = src.height;
    const [sx, sy] = iso(gx, gy, ztop);
    const x0 = Math.round(sx) + dx, y0 = Math.round(sy) + dy;
    for (let u = 0; u < sw; u++) {
      const off = face === 'l' ? (u >> 1) : -(u >> 1);
      ctx.drawImage(src, u, 0, 1, sh, x0 + u, y0 + off, 1, sh);
    }
  }
  // Left-wall items are specified from gy=a (back) to gy=b (front); their flat image's
  // left edge is at gy=b.
  const shearL = (ctx, src, a, ztop) => shear(ctx, src, 'l', a, 0, ztop);
  const shearR = (ctx, src, b, ztop) => shear(ctx, src, 'r', 0, b, ztop);

  // ---------- 3x5 pixel font ----------
  const FONT = {
    A: [2, 5, 7, 5, 5], B: [6, 5, 6, 5, 6], C: [3, 4, 4, 4, 3], D: [6, 5, 5, 5, 6], E: [7, 4, 6, 4, 7], F: [7, 4, 6, 4, 4],
    G: [3, 4, 5, 5, 3], H: [5, 5, 7, 5, 5], I: [7, 2, 2, 2, 7], J: [1, 1, 1, 5, 2], K: [5, 5, 6, 5, 5], L: [4, 4, 4, 4, 7],
    M: [5, 7, 7, 5, 5], N: [6, 5, 5, 5, 5], O: [2, 5, 5, 5, 2], P: [6, 5, 6, 4, 4], Q: [2, 5, 5, 6, 3], R: [6, 5, 6, 5, 5],
    S: [3, 4, 2, 1, 6], T: [7, 2, 2, 2, 2], U: [5, 5, 5, 5, 7], V: [5, 5, 5, 5, 2], W: [5, 5, 7, 7, 5], X: [5, 5, 2, 5, 5],
    Y: [5, 5, 2, 2, 2], Z: [7, 1, 2, 4, 7], 0: [7, 5, 5, 5, 7], 1: [2, 6, 2, 2, 7], 2: [6, 1, 2, 4, 7], 3: [6, 1, 2, 1, 6],
    4: [5, 5, 7, 1, 1], 5: [7, 4, 6, 1, 6], 6: [3, 4, 6, 5, 2], 7: [7, 1, 2, 2, 2], 8: [2, 5, 2, 5, 2], 9: [2, 5, 3, 1, 6],
    ' ': [0, 0, 0, 0, 0], '.': [0, 0, 0, 0, 2], ':': [0, 2, 0, 2, 0], '-': [0, 0, 7, 0, 0], '/': [1, 1, 2, 4, 4],
    '#': [5, 7, 5, 7, 5], '!': [2, 2, 2, 0, 2], '?': [6, 1, 2, 0, 2], '+': [0, 2, 7, 2, 0], '(': [1, 2, 2, 2, 1],
    ')': [4, 2, 2, 2, 4], ',': [0, 0, 0, 2, 4], '>': [4, 2, 1, 2, 4], '%': [5, 1, 2, 4, 5], ';': [0, 2, 0, 2, 4],
    "'": [2, 2, 0, 0, 0], '&': [2, 5, 2, 5, 3], '·': [0, 0, 2, 0, 0],
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

  // ---------- Team: looks, desks, routes ----------
  const LOOKS = {
    atlas:  { skin: '#e3b08a', hair: '#2b2530', shirt: '#2f3d63', pants: '#1f2438', shoe: '#141219', extra: 'suit', tall: 1 },
    anvil:  { skin: '#b9784f', hair: '#3a2417', shirt: '#b5562f', pants: '#3b3a3f', shoe: '#1d1a1a', extra: 'beanie', hat: '#41454e' },
    canvas: { skin: '#f1c7a2', hair: '#9a4026', shirt: '#2f9184', pants: '#2b2f45', shoe: '#2a1c1c', extra: 'long' },
    plume:  { skin: '#d9a17a', hair: '#1d1a20', shirt: '#ece6da', pants: '#3a3550', shoe: '#1b1b22', extra: 'feather' },
    warden: { skin: '#8d5a3b', hair: '#151217', shirt: '#55633a', pants: '#262a1e', shoe: '#121212', extra: 'cap', hat: '#22252c' },
    proof:  { skin: '#f0c9a8', hair: '#1a1416', shirt: '#d8dfe6', pants: '#384252', shoe: '#1c1c22', extra: 'glasses' },
    frame:  { skin: '#c48a63', hair: '#5b3a22', shirt: '#dca93a', pants: '#2c2b33', shoe: '#1a1414', extra: 'phones', hat: '#2b2b35' },
  };
  const DEFAULT_LOOK = { skin: '#d8a882', hair: '#3b3035', shirt: '#6a6f7a', pants: '#2c2f38', shoe: '#18181c' };

  // x-desk: long side along x, screen faces +y, person sits at +y facing up-right.
  // y-desk: long side along y, screen faces +x, person sits at +x facing up-left.
  const ROW_A = 3.6, ROW_B = 6.85;
  const DESKS = {
    atlas:  { o: 'x', x: 3.4, y: 1.3, w: 2.4, d: 0.9, seat: [4.6, 2.75], stand: [4.6, ROW_A], entry: [4.6, ROW_A], visit: [5.75, 3.05] },
    anvil:  { o: 'y', x: 0.3, y: 1.7, w: 0.9, d: 1.8, seat: [1.75, 2.6], stand: [2.6, 2.6], entry: [2.6, 2.6], visit: [2.3, 3.35] },
    canvas: { o: 'y', x: 0.3, y: 4.1, w: 0.9, d: 1.8, seat: [1.75, 5.0], stand: [2.6, 5.0], entry: [2.6, 5.0], visit: [2.3, 5.75] },
    warden: { o: 'y', x: 0.3, y: 7.5, w: 0.9, d: 1.4, seat: [1.75, 8.2], stand: [2.6, 8.2], entry: [2.6, 8.2], visit: [2.35, 7.45] },
    plume:  { o: 'x', x: 5.0, y: 4.6, w: 2.0, d: 0.9, seat: [6.0, 6.05], stand: [6.0, ROW_B], entry: [6.0, ROW_B], visit: [7.15, 6.3] },
    proof:  { o: 'x', x: 8.0, y: 4.6, w: 2.0, d: 0.9, seat: [9.0, 6.05], stand: [9.0, ROW_B], entry: [9.0, ROW_B], visit: [10.15, 6.3] },
    frame:  { o: 'x', x: 9.6, y: 7.15, w: 2.2, d: 0.85, seat: [10.7, 8.5], stand: [9.2, 8.5], entry: [9.2, ROW_B], visit: [9.45, 8.75] },
  };
  const SPOTS = {
    coffee:  { entry: [10.4, 1.3], stand: [10.4, 1.3], face: [10.4, 0.3] },
    books:   { entry: [1.0, 1.0], stand: [1.0, 1.0], face: [1.0, 0.0] },
    board:   { entry: [7.9, 1.0], stand: [7.9, 1.0], face: [7.9, 0.0] },
    rack:    { entry: [1.5, ROW_B], stand: [1.5, ROW_B], face: [0.5, 6.8] },
    meeting: { entry: [5.6, ROW_B], stand: [5.6, 7.35], face: [4.6, 7.9] },
  };
  const WANDER = {
    proof: ['anvil', 'coffee', 'meeting', 'anvil'],
    warden: ['rack', 'books', 'rack', 'coffee'],
    anvil: ['coffee', 'books'],
    frame: ['coffee', 'meeting'],
  };
  const NODES = {
    g: [2.6, 1.0], a: [2.6, ROW_A], b: [7.5, ROW_A], c: [11.0, ROW_A], m: [7.5, 1.0], k: [11.0, 1.3],
    d: [2.6, ROW_B], e: [7.5, ROW_B], f: [11.0, ROW_B], h: [2.6, 8.2],
  };
  const EDGES = [['g', 'a'], ['a', 'b'], ['b', 'c'], ['b', 'm'], ['c', 'k'], ['a', 'd'], ['b', 'e'], ['c', 'f'], ['d', 'e'], ['e', 'f'], ['d', 'h']];

  function route(from, to) {
    const pts = Object.assign({}, NODES, { A: from, B: to });
    const adj = {};
    const link = (p, q) => {
      const dd = Math.hypot(pts[p][0] - pts[q][0], pts[p][1] - pts[q][1]);
      (adj[p] = adj[p] || []).push([q, dd]); (adj[q] = adj[q] || []).push([p, dd]);
    };
    EDGES.forEach(([p, q]) => link(p, q));
    for (const t of ['A', 'B']) for (const k of Object.keys(NODES)) {
      if (Math.abs(NODES[k][0] - pts[t][0]) < 0.05 || Math.abs(NODES[k][1] - pts[t][1]) < 0.05) link(t, k);
    }
    if (Math.abs(from[0] - to[0]) < 0.05 || Math.abs(from[1] - to[1]) < 0.05) link('A', 'B');
    const dist = { A: 0 }, prev = {}, done = new Set();
    for (;;) {
      let u = null;
      for (const k in dist) if (!done.has(k) && (u === null || dist[k] < dist[u])) u = k;
      if (u === null || u === 'B') break;
      done.add(u);
      for (const [v, dd] of adj[u] || []) if (dist[v] === undefined || dist[u] + dd < dist[v]) { dist[v] = dist[u] + dd; prev[v] = u; }
    }
    if (dist.B === undefined) return [from, to];
    const out = [];
    for (let k = 'B'; k; k = prev[k]) out.unshift(pts[k]);
    return out;
  }

  // ---------- People ----------
  // Sprites are drawn into a small canvas (anchor = feet), given a 1px dark outline, and cached.
  const SPR_W = 32, SPR_H = 56, AX = 16, AY = 50;
  const OUTLINE = '#1a1224';
  const spriteCache = new Map();

  function sprite(id, look, pose, frame, flip) {
    const key = id + '|' + pose + '|' + frame + '|' + (flip ? 1 : 0);
    let s = spriteCache.get(key);
    if (s) return s;
    const [c, x] = mkCanvas(SPR_W, SPR_H, true);
    drawPerson(x, look, pose, frame, flip);
    outline(x);
    spriteCache.set(key, c);
    return c;
  }
  function outline(x) {
    const img = x.getImageData(0, 0, SPR_W, SPR_H), d = img.data;
    const solid = (i, j) => i >= 0 && j >= 0 && i < SPR_W && j < SPR_H && d[(j * SPR_W + i) * 4 + 3] > 0;
    const pts = [];
    for (let j = 0; j < SPR_H; j++) for (let i = 0; i < SPR_W; i++) {
      if (!solid(i, j) && (solid(i - 1, j) || solid(i + 1, j) || solid(i, j - 1) || solid(i, j + 1))) pts.push([i, j]);
    }
    x.fillStyle = OUTLINE;
    pts.forEach(([i, j]) => x.fillRect(i, j, 1, 1));
  }

  // pose: stand-front, stand-back, walk-front, walk-back (frame 0..3), write-back (0/1),
  //       sit-type (0/1), sit-mug (0/1), sit-read (0/1), sit-wait (0/1)
  function drawPerson(ctx, L, pose, frame, flip) {
    const f = flip ? -1 : 1;
    const R = (dx, dy, w, h, col) => { ctx.fillStyle = col; ctx.fillRect(f === 1 ? AX + dx : AX - dx - w, AY + dy, w, h); };
    const tall = L.tall ? 1 : 0;
    const shirtD = mix(shade(L.shirt, -0.3), '#2a2450', 0.15), shirtL = shade(L.shirt, 0.12);
    const skinD = mix(shade(L.skin, -0.18), '#6a3a50', 0.15);
    const hairL = shade(L.hair, 0.2);

    if (pose.startsWith('sit')) {
      // Seated, seen from behind, turned toward the desk (up-right when not flipped).
      const kind = pose.slice(4);
      const look = kind === 'wait' && frame === 1;      // glances toward the room
      const hy = -40 - tall + (kind === 'read' ? 1 : 0);
      // arms + hands
      if (kind === 'type') {
        R(5, -25, 2, 6, shirtD); R(-7, -25, 2, 6, shirtD);
        R(6, -21 - (frame ? 1 : 0), 3, 2, L.skin); R(-6, -20 - (frame ? 0 : 1), 3, 2, skinD);
      } else if (kind === 'mug') {
        R(-7, -25, 2, 7, shirtD);
        if (frame) { R(5, -30, 2, 5, shirtD); R(5, -33, 3, 3, '#f1ebe0'); R(8, -32, 1, 2, '#f1ebe0'); }
        else { R(5, -25, 2, 6, shirtD); R(6, -20, 3, 3, '#f1ebe0'); }
      } else if (kind === 'read') {
        R(5, -25, 2, 5, shirtD); R(-7, -25, 2, 5, shirtD);
        R(5, -24, 4, 5, frame ? '#8c2a35' : '#2f5a7a'); R(5, -24, 4, 1, '#f1ebe0');
      } else {
        R(5, -25, 2, 7, shirtD); R(-7, -25, 2, 7, shirtD);
      }
      // torso
      R(-5, -27 - tall, 11, 12 + tall, L.shirt);
      R(-5, -27 - tall, 2, 12 + tall, shirtD);
      R(-3, -27 - tall, 7, 1, shirtL);
      if (L.extra === 'suit') R(-1, -27 - tall, 3, 1, '#ece6da');
      // neck + head
      R(-1, hy + 11, 3, 1, skinD);
      R(-5, hy, 11, 11, L.hair);
      R(-4, hy, 9, 1, hairL);
      if (look) { R(-5, hy + 4, 2, 4, L.skin); R(-5, hy + 5, 1, 1, '#1a1418'); }
      else { R(5, hy + 4, 1, 3, L.skin); R(4, hy + 5, 1, 1, skinD); }
      extraBack(R, L, hy);
      return;
    }

    const back = pose.endsWith('back');
    const walking = pose.startsWith('walk');
    const writing = pose.startsWith('write');
    const ph = walking ? frame % 4 : 0;
    const lift = [0, 1, 0, 1][ph], side = [0, 0, 1, 1][ph];
    const hy = -37 - tall;
    // legs + shoes
    const lL = side === 0 ? lift : 0, rL = side === 1 ? lift : 0;
    R(-4, -11 - tall, 4, 9 + tall - lL, L.pants);
    R(1, -11 - tall, 4, 9 + tall - rL, shade(L.pants, -0.18));
    R(-5, -2 - lL, 5, 2, L.shoe);
    R(1, -2 - rL, 5, 2, L.shoe);
    // torso
    R(-5, -24 - tall, 11, 13, L.shirt);
    R(-5, -24 - tall, 2, 13, shirtD);
    R(-3, -24 - tall, 7, 1, shirtL);
    R(-5, -12 - tall, 11, 1, shade(L.pants, -0.3));
    // arms
    const sw = walking ? (side ? 1 : -1) * (lift ? 1 : 0) : 0;
    if (writing) {
      R(-7, -23 - tall, 2, 9, shirtD); R(-7, -14 - tall, 2, 2, skinD);
      R(6, -33 - tall + (frame ? 1 : 0), 2, 10, shirtD); R(6, -35 - tall + (frame ? 1 : 0), 2, 2, L.skin);
    } else {
      R(-7, -23 - tall + sw, 2, 9, shirtD);
      R(6, -23 - tall - sw, 2, 9, back ? shirtD : L.shirt);
      R(-7, -14 - tall + sw, 2, 2, skinD);
      R(6, -14 - tall - sw, 2, 2, L.skin);
    }
    if (!back && L.extra === 'suit') {
      R(-1, -24 - tall, 3, 2, '#ece6da'); R(0, -22 - tall, 1, 7, '#8c2a35');
      R(-5, -24 - tall, 3, 12, shade(L.shirt, -0.12)); R(3, -24 - tall, 3, 12, shade(L.shirt, -0.12));
    }
    if (!back && L.extra === 'cap') R(2, -21 - tall, 2, 2, '#d9a83a');      // badge
    // neck + head
    R(-1, hy + 11, 3, 2, skinD);
    if (back) {
      R(-5, hy, 11, 11, L.hair);
      R(-4, hy, 9, 1, hairL);
      R(-6, hy + 5, 1, 2, L.skin); R(6, hy + 5, 1, 2, L.skin);
      extraBack(R, L, hy);
    } else {
      R(-5, hy, 11, 11, L.skin);
      R(-5, hy + 8, 11, 3, skinD);
      R(-5, hy + 8, 11, 1, L.skin);
      R(-5, hy, 11, 3, L.hair);
      R(-5, hy + 3, 2, 3, L.hair); R(5, hy + 3, 1, 2, L.hair);
      R(-4, hy, 9, 1, hairL);
      R(-2, hy + 5, 1, 2, '#1a1418'); R(2, hy + 5, 1, 2, '#1a1418');
      R(-3, hy + 7, 1, 1, '#e58f7a'); R(3, hy + 7, 1, 1, '#e58f7a');
      R(0, hy + 8, 2, 1, mix(L.skin, '#6a2a30', 0.45));
      extraFront(R, L, hy);
    }
  }
  function extraBack(R, L, hy) {
    switch (L.extra) {
      case 'beanie': R(-6, hy - 2, 13, 5, L.hat); R(-6, hy + 2, 13, 1, shade(L.hat, -0.3)); R(-1, hy - 3, 3, 1, L.hat); break;
      case 'cap': R(-6, hy - 2, 13, 5, L.hat); R(-6, hy + 2, 13, 1, shade(L.hat, -0.35)); break;
      case 'long': R(-6, hy + 2, 13, 10, L.hair); R(-5, hy + 12, 11, 1, L.hair); R(-4, hy, 9, 1, shade(L.hair, 0.2)); break;
      case 'feather': R(-2, hy - 3, 5, 4, L.hair); R(2, hy - 6, 1, 3, '#f6f1e7'); R(3, hy - 9, 1, 4, '#f6f1e7'); R(4, hy - 10, 1, 3, '#d8d0c2'); break;
      case 'phones': R(-6, hy - 1, 13, 2, L.hat); R(-7, hy + 3, 3, 5, L.hat); R(5, hy + 3, 3, 5, L.hat); break;
      case 'glasses': for (let i = -5; i < 6; i += 2) R(i, hy - 1, 1, 1, L.hair); R(6, hy + 5, 1, 1, '#9aa6b2'); break;
      case 'suit': R(-5, hy, 11, 3, shade(L.hair, 0.12)); break;
      default: break;
    }
  }
  function extraFront(R, L, hy) {
    switch (L.extra) {
      case 'beanie': R(-6, hy - 2, 13, 5, L.hat); R(-6, hy + 2, 13, 1, shade(L.hat, -0.3)); R(-1, hy - 3, 3, 1, L.hat); R(-3, hy + 9, 7, 2, L.hair); break;
      case 'cap': R(-6, hy - 2, 13, 4, L.hat); R(-7, hy + 2, 15, 1, shade(L.hat, -0.35)); break;
      case 'long': R(-6, hy + 1, 2, 13, L.hair); R(5, hy + 1, 2, 13, L.hair); break;
      case 'feather': R(-2, hy - 3, 5, 3, L.hair); R(4, hy - 6, 1, 4, '#f6f1e7'); R(5, hy - 9, 1, 4, '#f6f1e7'); break;
      case 'phones': R(-6, hy - 1, 13, 2, L.hat); R(-7, hy + 3, 3, 5, L.hat); R(5, hy + 3, 3, 5, L.hat); break;
      case 'glasses': R(-3, hy + 5, 3, 2, '#3a4652'); R(1, hy + 5, 3, 2, '#3a4652'); R(-2, hy + 5, 1, 1, '#b9c6d2'); R(2, hy + 5, 1, 1, '#b9c6d2'); for (let i = -5; i < 6; i += 2) R(i, hy - 1, 1, 1, L.hair); break;
      default: break;
    }
  }

  // ---------- Furniture ----------
  const WOOD = '#a8734a', WOOD_D = '#7e5234', METAL = '#3a3946';
  const CHAIR = { seat: '#3d3550', back: '#4a4063', hi: '#62567f', base: '#1d1a24' };

  function chairSeat(ctx, sx, sy, o) {
    const f = o === 'x' ? 1 : -1;
    const R = (dx, dy, w, h, c) => { ctx.fillStyle = c; ctx.fillRect(f === 1 ? sx + dx : sx - dx - w, sy + dy, w, h); };
    R(-6, -1, 13, 1, CHAIR.base); R(-4, -2, 9, 1, CHAIR.base);
    R(0, -10, 1, 8, '#2b2733');
    R(-6, -14, 13, 4, CHAIR.seat); R(-6, -14, 13, 1, CHAIR.hi);
  }
  function chairBack(ctx, sx, sy, o) {
    const f = o === 'x' ? 1 : -1;
    const R = (dx, dy, w, h, c) => { ctx.fillStyle = c; ctx.fillRect(f === 1 ? sx + dx : sx - dx - w, sy + dy, w, h); };
    R(-7, -21, 13, 8, CHAIR.back);
    R(-7, -21, 13, 1, CHAIR.hi);
    R(-7, -21, 1, 8, shade(CHAIR.back, -0.25));
    R(-7, -14, 13, 1, shade(CHAIR.back, -0.3));
  }

  function lamp(ctx, gx, gy, z) {
    const [bx, by] = iso(gx, gy, z);
    ctx.fillStyle = '#2b2622'; ctx.fillRect(bx - 3, by - 1, 6, 2);
    line(ctx, bx, by - 1, bx - 1, by - 12, '#4a423b');
    line(ctx, bx - 1, by - 12, bx + 4, by - 16, '#4a423b');
    ctx.fillStyle = '#2f5a4a'; ctx.fillRect(bx + 1, by - 19, 8, 4);
    ctx.fillStyle = '#4b8a70'; ctx.fillRect(bx + 2, by - 20, 6, 1);
    ctx.fillStyle = '#ffe2a6'; ctx.fillRect(bx + 2, by - 15, 6, 1);
  }
  function mug(ctx, gx, gy, z, col = '#ece4d6') { box(ctx, gx, gy, 0.2, 0.2, z, 5, { t: '#3a2620', l: col, r: shade(col, -0.2) }); }
  function paper(ctx, gx, gy, z, w = 0.45, d = 0.32) { box(ctx, gx, gy, w, d, z, 1, { t: '#f1ebdf', l: '#cfc6b5', r: '#b8ae9c' }); }
  function kb(ctx, gx, gy, z, o) {
    if (o === 'x') box(ctx, gx, gy, 0.85, 0.2, z, 1, { t: '#d8d0bf', l: '#a69c88', r: '#8f8674' });
    else box(ctx, gx, gy, 0.2, 0.85, z, 1, { t: '#d8d0bf', l: '#a69c88', r: '#8f8674' });
  }

  // A monitor: stand + body; screen geometry is remembered for the per-frame overlay.
  function monitor(ctx, gx, gy, z, o, big, screens, key) {
    const body = { t: '#d2c7b0', l: '#b6aa92', r: '#988c75', e: '#e6dcc6' };
    const w = big ? 1.15 : 0.95, h = big ? 18 : 15, dep = 0.38;
    if (o === 'x') {
      box(ctx, gx + w / 2 - 0.15, gy + 0.05, 0.3, 0.25, z, 3, body);
      box(ctx, gx, gy, w, dep, z + 3, h, body);
      screens.push({ key, face: 'l', gx, gy: gy + dep, ztop: z + 3 + h, fw: Math.round(w * TW) - 4, fh: h - 5 });
    } else {
      box(ctx, gx + 0.05, gy + w / 2 - 0.15, 0.25, 0.3, z, 3, body);
      box(ctx, gx, gy, dep, w, z + 3, h, body);
      screens.push({ key, face: 'r', gx: gx + dep, gy: gy + w, ztop: z + 3 + h, fw: Math.round(w * TW) - 4, fh: h - 5 });
    }
  }

  function deskBase(ctx, D) {
    const { x, y, w, d, o } = D;
    const leg = { t: '#4a3122', l: '#3d281b', r: '#2f1e14' };
    const z = DESK_Z;
    box(ctx, x + 0.08, y + 0.08, 0.12, 0.12, 0, z - 2, leg);
    box(ctx, x + w - 0.2, y + 0.08, 0.12, 0.12, 0, z - 2, leg);
    box(ctx, x + 0.08, y + d - 0.2, 0.12, 0.12, 0, z - 2, leg);
    box(ctx, x + w - 0.2, y + d - 0.2, 0.12, 0.12, 0, z - 2, leg);
    if (o === 'x') box(ctx, x + w - 0.7, y + 0.1, 0.55, d - 0.2, 0, z - 2, mat(WOOD_D));
    else box(ctx, x + 0.1, y + 0.1, w - 0.2, 0.6, 0, z - 2, mat(WOOD_D));
    box(ctx, x, y, w, d, z - 2, 2, mat(WOOD));
  }

  // Each desk tells who sits there.
  function deskKit(ctx, id, D, screens) {
    const { x, y, w, d, o } = D, z = DESK_Z;
    if (o === 'x') {
      lamp(ctx, x + w - 0.3, y + 0.25, z);
      if (id === 'frame') {
        monitor(ctx, x + 0.15, y + 0.08, z, 'x', true, screens, 'frame2');
        monitor(ctx, x + 1.25, y + 0.08, z, 'x', true, screens, 'frame');
        box(ctx, x + 0.02, y + 0.55, 0.22, 0.22, z, 9, { t: '#2a2a33', l: '#33333d', r: '#24242c' });   // speaker
        box(ctx, x + 1.95, y + 0.55, 0.2, 0.2, z, 9, { t: '#2a2a33', l: '#33333d', r: '#24242c' });
        kb(ctx, x + 0.8, y + 0.62, z, 'x');
        // clapperboard
        box(ctx, x + 0.25, y + 0.62, 0.42, 0.18, z, 1, { t: '#1d1d22', l: '#2a2a30', r: '#141418' });
        const [cx, cy] = iso(x + 0.3, y + 0.66, z + 1);
        for (let i = 0; i < 4; i++) { ctx.fillStyle = i % 2 ? '#f1ebdf' : '#1d1d22'; ctx.fillRect(cx + i * 2, cy - 1, 2, 1); }
        return;
      }
      monitor(ctx, x + w / 2 - 0.35, y + 0.08, z, 'x', id === 'atlas', screens, id);
      kb(ctx, x + w / 2 - 0.3, y + 0.62, z, 'x');
      box(ctx, x + w / 2 + 0.65, y + 0.62, 0.12, 0.16, z, 1, { t: '#c9c0ad', l: '#a49a86', r: '#8f8674' }); // mouse
      if (id === 'atlas') {
        paper(ctx, x + 0.15, y + 0.15, z); paper(ctx, x + 0.17, y + 0.12, z + 1); paper(ctx, x + 0.14, y + 0.14, z + 2);
        mug(ctx, x + 0.25, y + 0.6, z, '#8c2a35');
        box(ctx, x + 0.65, y + 0.55, 0.25, 0.32, z, 1, { t: '#1d1d22', l: '#2a2a30', r: '#141418' });   // phone
      } else if (id === 'plume') {
        paper(ctx, x + 0.12, y + 0.45, z, 0.5, 0.36);
        const [px, py] = iso(x + 0.25, y + 0.55, z + 1);
        ['#ff5a1f', '#0b0b0b', '#8a8f98', '#ffffff'].forEach((c, i) => { ctx.fillStyle = c; ctx.fillRect(px + 2 + i * 3, py + 1, 2, 2); });
        mug(ctx, x + 0.2, y + 0.15, z, '#2f5a7a');
        const [qx, qy] = iso(x + 0.3, y + 0.25, z + 5);
        line(ctx, qx, qy, qx + 3, qy - 8, '#f6f1e7'); line(ctx, qx + 1, qy, qx + 4, qy - 7, '#d8d0c2');      // feather quill
      } else if (id === 'proof') {
        box(ctx, x + 0.12, y + 0.4, 0.45, 0.45, z, 1, { t: '#b07a4a', l: '#8a5a36', r: '#6e462a' });   // clipboard
        paper(ctx, x + 0.16, y + 0.44, z + 1, 0.37, 0.37);
        const [px, py] = iso(x + 0.2, y + 0.5, z + 2);
        for (let i = 0; i < 3; i++) { ctx.fillStyle = '#3f9a55'; ctx.fillRect(px + 4, py + 1 + i * 2, 2, 1); ctx.fillStyle = '#8a8f98'; ctx.fillRect(px + 7, py + 1 + i * 2, 5, 1); }
        mug(ctx, x + 0.2, y + 0.12, z);
        const [mx, my] = iso(x + 1.75, y + 0.7, z + 1);          // magnifier
        ctx.fillStyle = '#9fb2c0'; ctx.fillRect(mx, my - 3, 4, 3); ctx.fillStyle = '#4a3122'; ctx.fillRect(mx + 3, my, 3, 1);
      }
    } else {
      lamp(ctx, x + 0.25, y + 0.25, z);
      if (id === 'anvil') {
        monitor(ctx, x + 0.08, y + 0.35, z, 'y', true, screens, 'anvil');
        monitor(ctx, x + 0.12, y + 1.45, z, 'y', false, screens, 'anvil2');
        kb(ctx, x + 0.62, y + 0.6, z, 'y');
        // a tiny anvil paperweight
        const [ax, ay] = iso(x + 0.7, y + 1.55, z);
        ctx.fillStyle = '#4b4e58'; ctx.fillRect(ax - 3, ay - 4, 8, 2); ctx.fillRect(ax - 1, ay - 2, 4, 2); ctx.fillRect(ax - 2, ay, 6, 1);
        ctx.fillStyle = '#6b6f7a'; ctx.fillRect(ax - 3, ay - 4, 8, 1);
      } else if (id === 'canvas') {
        monitor(ctx, x + 0.08, y + 0.35, z, 'y', true, screens, 'canvas');
        box(ctx, x + 0.5, y + 1.25, 0.38, 0.5, z, 1, { t: '#26262e', l: '#1c1c22', r: '#141418' });      // drawing tablet
        const [tx, ty] = iso(x + 0.6, y + 1.35, z + 1);
        line(ctx, tx, ty, tx + 5, ty - 3, '#d8d0bf');
        const [sx, sy] = iso(x + 0.55, y + 1.5, z + 2);
        ['#2f9184', '#ff5a1f', '#e8d27f', '#c39bff'].forEach((c, i) => { ctx.fillStyle = c; ctx.fillRect(sx - 2 + i * 2, sy + 2, 2, 1); });
        mug(ctx, x + 0.62, y + 0.15, z, '#e8d27f');
      } else if (id === 'warden') {
        monitor(ctx, x + 0.08, y + 0.2, z, 'y', false, screens, 'warden');
        kb(ctx, x + 0.62, y + 0.35, z, 'y');
        // shield sticker on a small safe box
        box(ctx, x + 0.2, y + 1.0, 0.5, 0.35, z, 8, { t: '#4a4e58', l: '#3a3d46', r: '#2c2e36' });
        const [bx, by] = iso(x + 0.7, y + 1.35, z + 6);
        ctx.fillStyle = '#d9a83a'; ctx.fillRect(bx - 4, by - 3, 3, 3); ctx.fillRect(bx - 3, by, 1, 1);
      }
    }
  }

  function plant(ctx, gx, gy, big, seed) {
    const r = rng(seed);
    const h = big ? 12 : 7;
    box(ctx, gx, gy, 0.45, 0.45, 0, h, { t: '#4a2c20', l: '#b8643e', r: '#8f4a2d', e: '#d68457' });
    const [px, py] = iso(gx + 0.22, gy + 0.22, h);
    const n = big ? 16 : 8, spread = big ? 9 : 5, height = big ? 30 : 14;
    for (let i = 0; i < n; i++) {
      const a = (i / n) * Math.PI * 2 + r();
      const len = 0.4 + r() * 0.6;
      const lx = Math.round(px + Math.cos(a) * spread * len), ly = Math.round(py - 3 - (0.3 + r() * 0.7) * height);
      line(ctx, px, py - 2, lx, ly + 3, '#2f5a33');
      ctx.fillStyle = i % 3 ? '#4f9448' : '#3d7a3c'; ctx.fillRect(lx - 2, ly, 5, 3);
      ctx.fillStyle = '#74bb63'; ctx.fillRect(lx - 1, ly, 3, 1);
    }
  }

  // ---------- Wall flats ----------
  function bookshelfFront(w, h) {
    const [c, x] = mkCanvas(w, h);
    const r = rng(5);
    x.fillStyle = '#4b301f'; x.fillRect(0, 0, w, h);
    const shelves = 4, sh = Math.floor((h - 2) / shelves);
    const cols = ['#8c2a35', '#2f5a7a', '#d9a83a', '#3f7a64', '#c9bfb0', '#6a4a8a', '#b5603b', '#1f3b4d', '#e8e2d6'];
    for (let s = 0; s < shelves; s++) {
      const y0 = 1 + s * sh;
      x.fillStyle = '#25160d'; x.fillRect(1, y0, w - 2, sh - 1);
      let bx = 2;
      while (bx < w - 3) {
        if (r() < 0.1) { // a small plant or a box
          x.fillStyle = r() < 0.5 ? '#4f9448' : '#b5603b'; x.fillRect(bx, y0 + sh - 6, 4, 5); bx += 6; continue;
        }
        const bw = 1 + Math.floor(r() * 2), bh = sh - 3 - Math.floor(r() * 4);
        x.fillStyle = cols[Math.floor(r() * cols.length)];
        x.fillRect(bx, y0 + sh - 1 - bh, bw, bh);
        bx += bw + (r() < 0.25 ? 1 : 0);
      }
      x.fillStyle = '#7a5236'; x.fillRect(0, y0 + sh - 1, w, 1);
    }
    return c;
  }
  function cabinetFront(w, h, doors, col) {
    const [c, x] = mkCanvas(w, h);
    x.fillStyle = col; x.fillRect(0, 0, w, h);
    const dw = Math.floor(w / doors);
    for (let i = 0; i < doors; i++) {
      x.fillStyle = shade(col, -0.18); x.fillRect(i * dw + 1, 1, dw - 2, h - 2);
      x.fillStyle = shade(col, 0.08); x.fillRect(i * dw + 2, 2, dw - 4, h - 4);
      x.fillStyle = '#c9c0ad'; x.fillRect(i * dw + (i % 2 ? 2 : dw - 4), Math.floor(h / 2) - 2, 1, 4);
    }
    return c;
  }
  function windowFlat(c, w, h, t, idx) {
    const g = c.createLinearGradient(0, 0, 0, h);
    g.addColorStop(0, '#231d4d'); g.addColorStop(0.45, '#6e3a6c'); g.addColorStop(0.78, '#df7848'); g.addColorStop(1, '#f4b465');
    c.fillStyle = g; c.fillRect(0, 0, w, h);
    const sr = rng(11 + idx);
    for (let i = 0; i < 10; i++) { const sx = Math.floor(sr() * w), sy = Math.floor(sr() * h * 0.35); if (Math.sin(t * 2 + i) > -0.6) { c.fillStyle = '#f6eed8'; c.fillRect(sx, sy, 1, 1); } }
    if (idx === 0) { c.fillStyle = '#ffd27a'; c.fillRect(w - 13, h - 18, 6, 5); c.fillRect(w - 12, h - 19, 4, 7); }
    const r = rng(77 + idx);
    let x0 = 0;
    while (x0 < w) {
      const bw = 3 + Math.floor(r() * 6), bh = 8 + Math.floor(r() * 20);
      c.fillStyle = r() < 0.5 ? '#1e1530' : '#271b3a';
      c.fillRect(x0, h - bh, bw, bh);
      for (let wy = h - bh + 2; wy < h - 1; wy += 3) for (let wx = x0 + 1; wx < x0 + bw - 1; wx += 2) {
        if (r() < 0.28) { c.fillStyle = Math.sin(t * 0.5 + wx * 3 + wy) > -0.8 ? '#ffcf73' : '#5a3d3d'; c.fillRect(wx, wy, 1, 1); }
      }
      x0 += bw;
    }
    // a pigeon crosses now and then
    const p = ((t + idx * 7) % 15) / 3.4;
    if (p < 1) {
      const bx = Math.round(-4 + p * (w + 8)), by = Math.round(12 + Math.sin(p * 9) * 2), flap = Math.floor(t * 8) % 2;
      c.fillStyle = '#1a1424';
      c.fillRect(bx, by, 4, 1); c.fillRect(bx + 4, by - 1, 1, 1);
      if (flap) { c.fillRect(bx, by - 1, 2, 1); c.fillRect(bx + 1, by - 2, 1, 1); } else c.fillRect(bx + 1, by + 1, 2, 1);
    }
    c.fillStyle = '#6b4a35';
    c.fillRect(0, 0, w, 2); c.fillRect(0, h - 4, w, 4); c.fillRect(0, 0, 2, h); c.fillRect(w - 2, 0, 2, h);
    c.fillRect(Math.floor(w / 2) - 1, 0, 2, h); c.fillRect(0, Math.floor(h * 0.42), w, 2);
    c.fillStyle = '#8a6448'; c.fillRect(0, h - 4, w, 1);
    // a small plant on the sill
    if (idx === 1) { c.fillStyle = '#b8643e'; c.fillRect(4, h - 8, 4, 4); c.fillStyle = '#4f9448'; c.fillRect(3, h - 12, 6, 4); }
  }

  // The big wall screen: the in-scene dashboard (names, status, task ticker).
  function screenFlat(c, w, h, agents, queueLen, t, still) {
    c.fillStyle = '#17151d'; c.fillRect(0, 0, w, h);
    c.fillStyle = '#0a0f14'; c.fillRect(2, 2, w - 4, h - 4);
    ptext(c, 'TEAM', 5, 4, '#e8f0f2');
    const lab = 'OWNER ' + queueLen, lw = lab.length * 4;
    ptext(c, lab, w - 5 - lw, 4, '#ff8a5c');
    if (queueLen && Math.floor(t * 1.5) % 2 === 0) { c.fillStyle = '#ff5a1f'; c.fillRect(w - 9 - lw, 5, 3, 3); }
    c.fillStyle = '#26323d'; c.fillRect(4, 11, w - 8, 1);
    const tx = 34, tw = w - tx - 4;
    agents.slice(0, 7).forEach((a, i) => {
      const y = 14 + i * 6;
      let col = SCENE_STATUS[a.status] || '#9aa0aa';
      if (a.status === 'working' && Math.floor(t * 3 + i) % 3 === 0) col = shade(col, 0.4);
      c.fillStyle = col; c.fillRect(5, y + 1, 3, 3);
      ptext(c, a.name.slice(0, 6), 10, y, '#cfd8dc');
      const text = (a.task || (a.status === 'idle' ? 'IDLE' : STATUS_LABEL[a.status] || '')).toUpperCase();
      const len = text.length * 4;
      c.save(); c.beginPath(); c.rect(tx, y, tw, 6); c.clip();
      let off = 0;
      if (!still && len > tw) { const span = len - tw + 16; off = Math.floor(((t * 10 + i * 13) % (span + 30)) - 15); off = clamp(off, 0, span - 16); }
      ptext(c, text, tx - off, y, a.status === 'idle' ? '#6b7680' : '#9fb4bf');
      c.restore();
    });
    const sl = Math.floor(t * 18) % h;
    c.fillStyle = 'rgba(255,255,255,0.05)'; c.fillRect(2, sl, w - 4, 1);
  }

  // Atlas's whiteboard: a tiny kanban, one sticky note per bot.
  function whiteboardFlat(c, w, h, agents, t) {
    c.fillStyle = '#9aa3ad'; c.fillRect(0, 0, w, h);
    c.fillStyle = '#f1efe9'; c.fillRect(2, 2, w - 4, h - 4);
    ptext(c, 'PLAN', 4, 4, '#2b2530');
    const cols = [['NOW', 'working'], ['WAIT', 'waiting'], ['DONE', 'done'], ['FREE', 'idle']];
    const cw = Math.floor((w - 6) / cols.length);
    cols.forEach(([lab, st], i) => {
      const cx = 3 + i * cw;
      c.fillStyle = SCENE_STATUS[st] === '#9aa0aa' ? '#8a8f98' : shade(SCENE_STATUS[st], -0.2); c.fillRect(cx + 1, 12, cw - 3, 3);
      if (i) { c.fillStyle = '#c9c6bd'; c.fillRect(cx - 1, 11, 1, h - 16); }
      agents.filter((a) => a.status === st).forEach((a, k) => {
        const ny = 19 + k * 8;
        if (ny > h - 8) return;
        const L = LOOKS[a.id] || DEFAULT_LOOK;
        c.fillStyle = L.shirt; c.fillRect(cx + 1, ny, cw - 3, 7);
        c.fillStyle = 'rgba(0,0,0,0.25)'; c.fillRect(cx + 1, ny + 6, cw - 3, 1);
        ptext(c, a.name[0], cx + 2, ny + 1, lum(L.shirt) > 150 ? '#1a1418' : '#ffffff');
      });
    });
    c.fillStyle = '#6b7280'; c.fillRect(6, h - 3, 10, 1); c.fillStyle = '#c2410c'; c.fillRect(18, h - 3, 6, 1);
  }
  function lum(hex) { const n = parseInt(hex.slice(1), 16); return 0.3 * ((n >> 16) & 255) + 0.59 * ((n >> 8) & 255) + 0.11 * (n & 255); }

  function posterFlat() {
    const [c, x] = mkCanvas(22, 28);
    x.fillStyle = '#d9cdb8'; x.fillRect(0, 0, 22, 28);
    x.fillStyle = '#0b0b0b'; x.fillRect(1, 1, 20, 26);
    x.fillStyle = '#f4efe6';   // our own geometric bird in flight
    x.fillRect(5, 12, 9, 3); x.fillRect(13, 10, 3, 3); x.fillRect(15, 11, 2, 1);
    x.fillRect(6, 9, 4, 3); x.fillRect(8, 6, 3, 3); x.fillRect(3, 14, 3, 2);
    x.fillStyle = '#ff5a1f'; x.fillRect(14, 11, 1, 1);
    ptext(x, 'HOME', 3, 19, '#f4efe6');
    return c;
  }
  function shieldFlat() {
    const [c, x] = mkCanvas(16, 20);
    x.fillStyle = '#d9cdb8'; x.fillRect(0, 0, 16, 20);
    x.fillStyle = '#20232a'; x.fillRect(1, 1, 14, 18);
    x.fillStyle = '#55633a'; x.fillRect(4, 4, 8, 7); x.fillRect(5, 11, 6, 2); x.fillRect(6, 13, 4, 1); x.fillRect(7, 14, 2, 1);
    x.fillStyle = '#d9a83a'; x.fillRect(7, 6, 2, 4);
    return c;
  }
  function clockFlat(c) {
    c.clearRect(0, 0, 16, 16);
    for (let yy = 0; yy < 16; yy++) for (let xx = 0; xx < 16; xx++) {
      const dd = Math.hypot(xx + 0.5 - 8, yy + 0.5 - 8);
      if (dd <= 7) { c.fillStyle = dd > 5.6 ? '#6b4a35' : '#efe9dc'; c.fillRect(xx, yy, 1, 1); }
    }
    const now = new Date();
    const hr = (((now.getHours() % 12) + now.getMinutes() / 60) / 12) * Math.PI * 2, mn = (now.getMinutes() / 60) * Math.PI * 2;
    line(c, 7, 7, Math.round(7 + Math.sin(hr) * 3), Math.round(7 - Math.cos(hr) * 3), '#2b2530');
    line(c, 7, 7, Math.round(7 + Math.sin(mn) * 5), Math.round(7 - Math.cos(mn) * 5), '#8c2a35');
  }
  function rackFlat(c, w, h, t) {
    c.fillStyle = '#23252d'; c.fillRect(0, 0, w, h);
    for (let u = 0; u < 7; u++) {
      const y = 3 + u * 7;
      c.fillStyle = '#30333d'; c.fillRect(2, y, w - 4, 6);
      c.fillStyle = '#1a1c22'; c.fillRect(3, y + 2, w - 12, 1); c.fillRect(3, y + 4, w - 12, 1);
      for (let k = 0; k < 3; k++) {
        const on = Math.sin(t * (3 + u) + k * 2.1 + u) > 0.1;
        c.fillStyle = on ? (k === 2 && u === 4 ? '#ffb020' : '#56e07a') : '#1f3a26';
        c.fillRect(w - 8 + k * 2, y + 2, 1, 1);
      }
    }
  }

  // ---------- Layout of walls (grid coordinates) ----------
  const WIN = [{ a: 1.6, b: 3.2, z0: 30, z1: 84 }, { a: 4.0, b: 5.6, z0: 30, z1: 84 }];  // left wall
  const SCREEN = { a: 1.95, b: 6.35, z0: 40, z1: 94 };      // right wall
  const BOARD = { a: 6.85, b: 9.05, z0: 36, z1: 84 };       // right wall
  const CLOCK = { a: 11.15, z1: 90 };
  const RACK = { x: 0.12, y: 6.15, w: 0.78, d: 1.05, h: 60 };

  // ---------- Static layer ----------
  function drawStatic(ctx) {
    ctx.fillStyle = '#0d0a12'; ctx.fillRect(0, 0, W, H);
    // walls
    poly(ctx, [iso(0, 0, WALL), iso(0, RY, WALL), iso(0, RY, 0), iso(0, 0, 0)], '#3b2d45');
    poly(ctx, [iso(0, 0, WALL), iso(RX, 0, WALL), iso(RX, 0, 0), iso(0, 0, 0)], '#5a4256');
    for (let gy = 0.5; gy < RY; gy += 0.5) isoLine(ctx, [0, gy, WALL], [0, gy, 32], gy % 1 ? '#372a40' : '#352838');
    for (let gx = 0.5; gx < RX; gx += 0.5) isoLine(ctx, [gx, 0, WALL], [gx, 0, 32], gx % 1 ? '#563f52' : '#523b4e');
    // wainscot + rail + baseboard
    poly(ctx, [iso(0, 0, 30), iso(0, RY, 30), iso(0, RY, 0), iso(0, 0, 0)], '#2d2134');
    poly(ctx, [iso(0, 0, 30), iso(RX, 0, 30), iso(RX, 0, 0), iso(0, 0, 0)], '#3c2b3c');
    for (let gy = 0.75; gy < RY; gy += 1.5) poly(ctx, [iso(0, gy, 26), iso(0, gy + 1.1, 26), iso(0, gy + 1.1, 6), iso(0, gy, 6)], '#30243a');
    for (let gx = 0.75; gx < RX; gx += 1.5) poly(ctx, [iso(gx, 0, 26), iso(gx + 1.1, 0, 26), iso(gx + 1.1, 0, 6), iso(gx, 0, 6)], '#413044');
    isoLine(ctx, [0, RY, 31], [0, 0, 31], '#7a5644'); isoLine(ctx, [0, 0, 31], [RX, 0, 31], '#8a624e');
    isoLine(ctx, [0, RY, 30], [0, 0, 30], '#4f3a33'); isoLine(ctx, [0, 0, 30], [RX, 0, 30], '#5e463c');
    isoLine(ctx, [0, RY, 2], [0, 0, 2], '#22171f'); isoLine(ctx, [0, 0, 2], [RX, 0, 2], '#2a1c24');
    // wall caps (thickness)
    poly(ctx, [iso(0, 0, WALL + 3), iso(0, RY, WALL + 3), iso(-0.3, RY, WALL), iso(-0.3, 0, WALL)], '#5b4656');
    poly(ctx, [iso(-0.3, -0.3, WALL + 3), iso(RX, -0.3, WALL + 3), iso(RX, 0, WALL), iso(0, 0, WALL + 3)], '#6a5364');
    isoLine(ctx, [0, 0, WALL + 3], [0, 0, 0], '#271c2a');

    // floor: long planks along x, staggered seams
    const r = rng(42);
    const woods = ['#7a4f35', '#83573b', '#744a31', '#8a5d40', '#6f4630'];
    for (let row = 0; row < RY * 2; row++) {
      const gy0 = row * 0.5, gy1 = gy0 + 0.5;
      let gx0 = -r() * 1.5;
      while (gx0 < RX) {
        const gx1 = Math.min(RX, gx0 + 1.6 + r() * 1.8);
        const a = Math.max(0, gx0);
        const col = woods[Math.floor(r() * woods.length)];
        poly(ctx, [iso(a, gy0), iso(gx1, gy0), iso(gx1, gy1), iso(a, gy1)], col);
        isoLine(ctx, [a, gy0], [gx1, gy0], mix(shade(col, -0.25), '#2a1830', 0.2));
        if (gx1 < RX) isoLine(ctx, [gx1, gy0], [gx1, gy1], shade(col, -0.3));
        for (let k = 0; k < 2; k++) if (r() < 0.6) { const p = iso(a + (gx1 - a) * r(), gy0 + 0.15 + r() * 0.2); ctx.fillStyle = shade(col, 0.08); ctx.fillRect(p[0], p[1], 3, 1); }
        gx0 = gx1;
      }
    }
    // room edge
    poly(ctx, [iso(0, RY), iso(RX, RY), iso(RX, RY, -8), iso(0, RY, -8)], '#3a2618');
    poly(ctx, [iso(RX, 0), iso(RX, RY), iso(RX, RY, -8), iso(RX, 0, -8)], '#2a1a10');

    // rugs
    poly(ctx, [iso(2.3, 3.2), iso(11.8, 3.2), iso(11.8, 4.0), iso(2.3, 4.0)], '#5a2f38');
    poly(ctx, [iso(2.45, 3.3), iso(11.65, 3.3), iso(11.65, 3.9), iso(2.45, 3.9)], '#6e3a42');
    for (let gx = 2.8; gx < 11.5; gx += 0.5) { const p = iso(gx, 3.6); ctx.fillStyle = (gx * 2) % 2 ? '#c9925a' : '#8f5a48'; ctx.fillRect(p[0], p[1], 2, 1); }
    const ring = (cx, cy, rad, col) => poly(ctx, Array.from({ length: 28 }, (_, i) => iso(cx + Math.cos((i / 28) * Math.PI * 2) * rad, cy + Math.sin((i / 28) * Math.PI * 2) * rad)), col);
    ring(4.6, 7.9, 1.35, '#24474a'); ring(4.6, 7.9, 1.2, '#2e5a5c'); ring(4.6, 7.9, 0.85, '#356668');
    poly(ctx, [iso(4.7, 4.3), iso(10.4, 4.3), iso(10.4, 6.6), iso(4.7, 6.6)], '#3b3447');
    poly(ctx, [iso(4.85, 4.4), iso(10.25, 4.4), iso(10.25, 6.5), iso(4.85, 6.5)], '#453c54');

    // contact shadows for furniture (cool, soft-edged by two passes)
    const shadow = (x, y, w, d, a) => {
      poly(ctx, [iso(x - 0.05, y - 0.05), iso(x + w + 0.25, y - 0.05), iso(x + w + 0.25, y + d + 0.12), iso(x - 0.05, y + d + 0.12)], `rgba(18,8,32,${a * 0.6})`);
      poly(ctx, [iso(x + 0.05, y + 0.05), iso(x + w + 0.12, y + 0.05), iso(x + w + 0.12, y + d + 0.05), iso(x + 0.05, y + d + 0.05)], `rgba(18,8,32,${a})`);
    };
    for (const id in DESKS) { const D = DESKS[id]; shadow(D.x, D.y, D.w, D.d, 0.32); }
    shadow(9.3, 0, 2.65, 0.7, 0.35); shadow(0.2, 0, 1.7, 0.5, 0.35); shadow(RACK.x, RACK.y, RACK.w, RACK.d, 0.35);
    // cables: desk to floor box
    for (const id in DESKS) {
      const D = DESKS[id];
      const sx = D.o === 'x' ? D.x + D.w - 0.15 : D.x + 0.2, sy = D.o === 'x' ? D.y + 0.15 : D.y + D.d - 0.1;
      const ex = D.o === 'x' ? D.x + D.w + 0.25 : D.x + 0.25, ey = D.o === 'x' ? D.y + 0.25 : D.y + D.d + 0.2;
      isoLine(ctx, [sx, sy], [ex, sy + 0.05], '#1d1620'); isoLine(ctx, [ex, sy + 0.05], [ex, ey], '#1d1620');
      box(ctx, ex - 0.1, ey - 0.05, 0.3, 0.12, 0, 2, { t: '#d8d0bf', l: '#a69c88', r: '#8f8674' });
    }

    // wall pieces
    // bookshelf (right wall corner)
    box(ctx, 0.2, 0, 1.7, 0.5, 0, 74, mat('#5e3d27'));
    shear(ctx, bookshelfFront(Math.round(1.7 * TW) - 2, 70), 'l', 0.2, 0.5, 72, 1, 0);
    // kitchen: counter, cabinets, fridge, upper cabinet
    box(ctx, 9.3, 0, 1.85, 0.68, 0, 22, mat('#6d4a33'));
    shear(ctx, cabinetFront(Math.round(1.85 * TW) - 2, 18, 2, '#7a5238'), 'l', 9.3, 0.68, 20, 1, 0);
    box(ctx, 9.3, 0, 1.85, 0.7, 22, 2, { t: '#c9c0ad', l: '#a49a86', r: '#8f8674', e: '#e6dcc6' });
    box(ctx, 9.45, 0.08, 0.5, 0.45, 24, 15, { t: '#2a2a2e', l: '#38383f', r: '#232327', e: '#55555c' });   // coffee machine
    box(ctx, 9.55, 0.4, 0.25, 0.12, 24, 4, { t: '#1a1a1e', l: '#1a1a1e', r: '#121215' });
    box(ctx, 10.2, 0.12, 0.3, 0.3, 24, 9, { t: '#b9c2cc', l: '#9aa4ae', r: '#7d8691' });                   // kettle
    mug(ctx, 10.65, 0.3, 24); mug(ctx, 10.85, 0.18, 24, '#2f5a7a');
    box(ctx, 11.2, 0, 0.75, 0.72, 0, 52, { t: '#e9e6df', l: '#cfcac0', r: '#aea89d', e: '#ffffff' });       // fridge
    isoLine(ctx, [11.3, 0.72, 34], [11.3, 0.72, 44], '#8a8f98');
    isoLine(ctx, [11.2, 0.72, 30], [11.95, 0.72, 30], '#b8b2a7');
    const up = cabinetFront(Math.round(1.6 * TW), 20, 2, '#7a5238');
    box(ctx, 9.4, 0, 1.6, 0.4, 60, 22, mat('#6d4a33'));
    shear(ctx, up, 'l', 9.4, 0.4, 82);
    // server rack (left wall)
    box(ctx, RACK.x, RACK.y, RACK.w, RACK.d, 0, RACK.h, { t: '#2e3038', l: '#262830', r: '#1d1f25', e: '#4a4e58' });
    // posters
    shearR(ctx, posterFlat(), 1.35, 72);
    shearR(ctx, shieldFlat(), 7.55, 84);
    // string lights cable along the right wall
    for (let gx = 0.3; gx < RX - 0.2; gx += 0.05) {
      const z = WALL - 4 - Math.abs(Math.sin(gx * 1.3)) * 4;
      const p = iso(gx, 0, z); ctx.fillStyle = '#2a1f24'; ctx.fillRect(p[0], p[1], 1, 1);
    }
  }

  // Furniture + chairs as pre-rendered sprites (drawn once, then just blitted).
  const items = [];     // {depth, spr, overlay?}
  const screens = [];   // monitor screen geometry
  function bake(fn) {
    const [c, x] = mkCanvas(W, H, true);
    fn(x);
    const d = x.getImageData(0, 0, W, H).data;
    let x0 = W, y0 = H, x1 = -1, y1 = -1;
    for (let y = 0; y < H; y++) for (let xx = 0; xx < W; xx++) if (d[(y * W + xx) * 4 + 3]) {
      if (xx < x0) x0 = xx; if (xx > x1) x1 = xx; if (y < y0) y0 = y; if (y > y1) y1 = y;
    }
    if (x1 < 0) return null;
    const [o, ox] = mkCanvas(x1 - x0 + 1, y1 - y0 + 1);
    ox.drawImage(c, x0, y0, o.width, o.height, 0, 0, o.width, o.height);
    return { c: o, x: x0, y: y0 };
  }
  function buildItems() {
    for (const id in DESKS) {
      const D = DESKS[id];
      const mine = [];
      const spr = bake((x) => { deskBase(x, D); deskKit(x, id, D, mine); });
      mine.forEach((s) => screens.push(s));
      items.push({ depth: D.x + D.w / 2 + D.y + D.d / 2, spr, screens: mine });
      const [cx, cy] = iso(D.seat[0], D.seat[1]).map(Math.round);
      D.chairSeat = bake((x) => chairSeat(x, cx, cy, D.o));
      D.chairBack = bake((x) => chairBack(x, cx, cy, D.o));
      D.sx = cx; D.sy = cy;
    }
    // meeting corner: round table + three chairs + laptop
    items.push({
      depth: 4.6 + 7.9,
      spr: bake((x) => {
        const t = (cx, cy, rad, z, col) => poly(x, Array.from({ length: 24 }, (_, i) => iso(cx + Math.cos((i / 24) * Math.PI * 2) * rad, cy + Math.sin((i / 24) * Math.PI * 2) * rad, z)), col);
        box(x, 4.5, 7.8, 0.2, 0.2, 0, 14, { t: '#3d281b', l: '#3d281b', r: '#2f1e14' });
        t(4.6, 7.9, 0.4, 1, '#2f1e14');
        t(4.6, 7.9, 0.72, 14, '#6e462a'); t(4.6, 7.9, 0.7, 15, '#b07a4a'); t(4.6, 7.9, 0.62, 15, '#bd8656');
        box(x, 4.35, 7.6, 0.45, 0.3, 15, 1, { t: '#9aa4ae', l: '#7d8691', r: '#646c76' });
        box(x, 4.35, 7.6, 0.05, 0.3, 16, 7, { t: '#2a2a33', l: '#1e2a30', r: '#2a2a33' });
        paper(x, 4.75, 8.05, 15, 0.35, 0.25); mug(x, 4.9, 7.65, 15, '#8c2a35');
      }),
    });
    const stool = (gx, gy) => items.push({ depth: gx + gy, spr: bake((x) => { box(x, gx - 0.22, gy - 0.22, 0.44, 0.44, 9, 3, mat('#3d3550')); box(x, gx - 0.04, gy - 0.04, 0.08, 0.08, 0, 9, { t: '#2b2733', l: '#2b2733', r: '#1d1a24' }); }) });
    stool(3.6, 7.4); stool(4.1, 8.95); stool(5.45, 8.75);
    // plants
    [[2.0, 0.15, true, 1], [11.5, 2.4, true, 2], [11.55, 5.3, false, 3], [0.25, 3.65, false, 4], [6.9, 8.4, true, 5], [8.4, 0.25, false, 6]].forEach(([gx, gy, big, s]) =>
      items.push({ depth: gx + gy + 0.45, spr: bake((x) => plant(x, gx, gy, big, s)) }));
    // bins
    [[3.3, 2.4], [7.55, 5.2], [1.3, 6.0]].forEach(([gx, gy]) => items.push({ depth: gx + gy, spr: bake((x) => box(x, gx, gy, 0.25, 0.25, 0, 7, { t: '#22222a', l: '#55555f', r: '#44444d' })) }));
  }

  // ---------- State ----------
  let data = null, dataSource = 'fallback';
  let people = [];
  let selected = null, hovered = null;
  let simT = 0;
  let playing = true;
  let showNames = params.get('names') === '1';
  const R0 = rng(2026);

  const counts = () => { const c = {}; (data ? data.agents : []).forEach((a) => { c[a.status] = (c[a.status] || 0) + 1; }); return c; };

  function buildPeople() {
    const prev = new Map(people.map((p) => [p.id, p]));
    people = data.agents.filter((a) => DESKS[a.id]).map((a, i) => {
      const old = prev.get(a.id), D = DESKS[a.id];
      const p = old || { id: a.id, gx: D.seat[0], gy: D.seat[1], mode: 'sit', phase: i };
      p.agent = a; p.desk = D; p.look = LOOKS[a.id] || DEFAULT_LOOK;
      if (!old) p.next = firstMove(a);
      return p;
    });
  }
  function firstMove(a) {
    if (a.id === 'atlas' && a.status === 'working') return 0.8;
    if (a.id === 'proof' && a.status !== 'working') return 0.3;
    if (WANDER[a.id] && a.status !== 'working' && a.status !== 'waiting') return 6 + R0() * 10;
    return Infinity;
  }
  function targetOf(key) {
    if (SPOTS[key]) return { key, entry: SPOTS[key].entry, stand: SPOTS[key].stand, face: SPOTS[key].face };
    const D = DESKS[key];
    return D ? { key, entry: D.entry, via: D.stand, stand: D.visit, face: D.seat } : null;
  }
  function planTrip(p, key) {
    const tg = targetOf(key);
    if (!tg) return;
    const D = p.desk;
    const mid = route(D.entry, tg.entry).slice(1, -1);
    const via = tg.via && (tg.via[0] !== tg.entry[0] || tg.via[1] !== tg.entry[1]) ? [tg.via] : [];
    const there = [D.seat, D.stand, D.entry].concat(mid, [tg.entry], via, [tg.stand]);
    const back = [tg.stand].concat(via.slice().reverse(), [tg.entry], route(tg.entry, D.entry).slice(1, -1), [D.entry, D.stand, D.seat]);
    const hold = key === 'board' ? 4 : p.id === 'atlas' ? 3.2 : 4 + R0() * 3;
    p.legs = [{ path: there, then: 'stand', hold, tg }, { path: back, then: 'sit' }];
    startLeg(p);
  }
  function startLeg(p) {
    const leg = p.legs.shift();
    p.path = leg.path.map((q) => q.slice()); p.seg = 0; p.mode = 'walk'; p.leg = leg;
  }
  function busyKey(key, self) { return people.some((q) => q !== self && q.leg && q.leg.tg && q.leg.tg.key === key); }

  function step(dt) {
    simT += dt;
    const speed = 1.9;
    for (const p of people) {
      const a = p.agent;
      if (p.mode === 'sit') {
        if (simT < p.next) continue;
        let key = null;
        if (a.id === 'atlas' && a.status === 'working') {
          const list = people.filter((q) => q.id !== 'atlas').map((q) => q.id).concat(['board']);
          p.cursor = ((p.cursor === undefined ? -1 : p.cursor) + 1) % list.length;
          key = list[p.cursor];
          const tq = people.find((q) => q.id === key);
          if (tq && tq.mode !== 'sit') key = 'board';
        } else if (WANDER[a.id] && a.status !== 'working' && a.status !== 'waiting') {
          const list = WANDER[a.id];
          p.cursor = ((p.cursor === undefined ? -1 : p.cursor) + 1) % list.length;
          key = list[p.cursor];
          const tq = people.find((q) => q.id === key);
          if (busyKey(key, p) || (tq && tq.mode !== 'sit')) key = null;
        }
        if (key) planTrip(p, key); else p.next = simT + 5 + R0() * 6;
      } else if (p.mode === 'walk') {
        let left = speed * dt;
        while (left > 0 && p.seg < p.path.length - 1) {
          const [tx, ty] = p.path[p.seg + 1];
          const dx = tx - p.gx, dy = ty - p.gy, dist = Math.hypot(dx, dy);
          if (dist > 1e-6) p.dir = [dx, dy];
          if (dist <= left) { p.gx = tx; p.gy = ty; p.seg++; left -= dist; } else { p.gx += (dx / dist) * left; p.gy += (dy / dist) * left; left = 0; }
        }
        if (p.seg >= p.path.length - 1) {
          if (p.leg.then === 'stand') { p.mode = 'stand'; p.wait = p.leg.hold; p.tg = p.leg.tg; }
          else { p.mode = 'sit'; p.leg = null; p.tg = null; p.gx = p.desk.seat[0]; p.gy = p.desk.seat[1]; p.next = simT + nextGap(a); }
        }
      } else if (p.mode === 'stand') {
        p.wait -= dt;
        if (p.wait <= 0) startLeg(p);
      }
    }
  }
  function nextGap(a) { return a.id === 'atlas' ? 3 + R0() * 4 : a.id === 'proof' ? 8 + R0() * 7 : 12 + R0() * 14; }

  // Which sprite a person shows right now.
  function poseOf(p, t) {
    const st = p.agent.status;
    if (p.mode === 'sit') {
      const flip = p.desk.o === 'y';
      if (st === 'working') return ['sit-type', Math.floor(t * 7 + p.phase) % 2, flip];
      if (st === 'done') return ['sit-mug', (t + p.phase * 1.7) % 6 < 1.2 ? 1 : 0, flip];
      if (st === 'waiting') return ['sit-wait', (t + p.phase) % 5 < 1.4 ? 1 : 0, flip];
      return ['sit-read', (t + p.phase) % 7 < 0.4 ? 1 : 0, flip];
    }
    if (p.mode === 'walk') {
      const [dx, dy] = p.dir || [0, 1];
      const vx = dx - dy, vy = dx + dy;
      return [vy < -0.01 ? 'walk-back' : 'walk-front', Math.floor(t * 8) % 4, vx < 0];
    }
    const tg = p.tg;
    const [px, py] = iso(p.gx, p.gy);
    let back = true, flip = false;
    if (tg && tg.face) { const [fx, fy] = iso(tg.face[0], tg.face[1]); flip = fx < px; back = fy <= py; }
    if (tg && tg.key === 'board') return ['write-back', Math.floor(t * 3) % 2, false];
    return [back ? 'stand-back' : 'stand-front', 0, flip];
  }
  function headTop(p) {
    const [, py] = iso(p.gx, p.gy);
    const tall = p.look.tall ? 1 : 0;
    return Math.round(py - (p.mode === 'sit' ? 41 : 38) - tall - (p.look.extra === 'feather' ? 3 : 0));
  }

  // ---------- Render ----------
  const canvas = $('#scene');
  const out = canvas.getContext('2d');
  const [buf, bx] = mkCanvas(W, H);
  const [stat, sx] = mkCanvas(W, H);
  let ready = false;
  const flats = {
    win0: mkCanvas(Math.round((WIN[0].b - WIN[0].a) * TW), WIN[0].z1 - WIN[0].z0),
    win1: mkCanvas(Math.round((WIN[1].b - WIN[1].a) * TW), WIN[1].z1 - WIN[1].z0),
    screen: mkCanvas(Math.round((SCREEN.b - SCREEN.a) * TW), SCREEN.z1 - SCREEN.z0),
    board: mkCanvas(Math.round((BOARD.b - BOARD.a) * TW), BOARD.z1 - BOARD.z0),
    clock: mkCanvas(16, 16),
    rack: mkCanvas(Math.round(RACK.d * TW) - 2, RACK.h - 8),
  };
  const monBufs = {};
  const DUST = Array.from({ length: 56 }, (_, i) => { const r = rng(300 + i); return { w: i % 2, u: r(), v: r(), s: 0.2 + r() * 0.5, ph: r() * 6.28 }; });
  const LIGHT = [0.05, 0.014];
  const BEAMS = WIN.map((wd) => {
    const corners = [[wd.a, wd.z0], [wd.b, wd.z0], [wd.b, wd.z1], [wd.a, wd.z1]];
    const wallPts = corners.map(([gy, z]) => iso(0, gy, z));
    const floorPts = corners.map(([gy, z]) => iso(z * LIGHT[0], gy + z * LIGHT[1], 0));
    return { wallPts, floorPts, hull: hull(wallPts.concat(floorPts)) };
  });
  function hull(pts) {
    const p = pts.slice().sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    const cr = (o, a, b) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    const lo = [], up = [];
    for (const q of p) { while (lo.length >= 2 && cr(lo[lo.length - 2], lo[lo.length - 1], q) <= 0) lo.pop(); lo.push(q); }
    for (let i = p.length - 1; i >= 0; i--) { const q = p[i]; while (up.length >= 2 && cr(up[up.length - 2], up[up.length - 1], q) <= 0) up.pop(); up.push(q); }
    return lo.slice(0, -1).concat(up.slice(0, -1));
  }
  const pathPoly = (ctx, pts) => { ctx.beginPath(); pts.forEach((q, i) => (i ? ctx.lineTo(q[0], q[1]) : ctx.moveTo(q[0], q[1]))); ctx.closePath(); };
  const PENDANTS = [[4.6, 7.9, 74], [10.3, 1.1, 78]];

  function paintScreen(key, c, w, h, t, still) {
    const id = key.replace(/2$/, '');
    const a = data && data.agents.find((q) => q.id === id);
    const st = a ? a.status : 'idle';
    c.fillStyle = '#0b1a1c'; c.fillRect(0, 0, w, h);
    if (key === 'frame2') {      // timeline
      c.fillStyle = '#141820'; c.fillRect(0, 0, w, h);
      const cols = ['#c39bff', '#7fd6e8', '#e8d27f', '#e89f7f'];
      for (let r = 0; r < 3; r++) for (let k = 0; k < 4; k++) { c.fillStyle = cols[(r + k) % 4]; c.fillRect(1 + k * 5 + r, 2 + r * 3, 4, 2); }
      const ph = still ? 6 : Math.floor(t * 3) % w;
      c.fillStyle = '#ff5a1f'; c.fillRect(ph, 0, 1, h);
      return;
    }
    if (key === 'frame') {       // preview of a dusk frame
      const g = c.createLinearGradient(0, 0, 0, h); g.addColorStop(0, '#2b2457'); g.addColorStop(1, '#f2b061');
      c.fillStyle = g; c.fillRect(0, 0, w, h);
      c.fillStyle = '#1e1530'; c.fillRect(0, h - 3, w, 3); c.fillRect(3, h - 6, 3, 3); c.fillRect(10, h - 5, 4, 2);
      if (st === 'done') { c.fillStyle = '#7bd88f'; c.fillRect(w - 4, 1, 3, 3); }
      return;
    }
    if (st === 'working') {
      const scroll = still ? 0 : Math.floor(t * 4 + id.length);
      const pal = id === 'anvil' ? ['#9fe39a', '#6fbf6a', '#c7f0c4'] : id === 'atlas' ? ['#e8eef2', '#9fb4bf', '#cfd8dc'] : ['#7fd6e8', '#e8d27f', '#c39bff', '#9fe39a', '#e89f7f'];
      for (let r = 0; r < Math.ceil(h / 2) + 1; r++) {
        const n = scroll + r, ind = id === 'atlas' ? 0 : (n * 7) % 3, len = 3 + ((n * 13) % (w - 5));
        c.fillStyle = pal[(n * 3) % pal.length];
        c.fillRect(1 + ind, r * 2, Math.min(len, w - 2 - ind), 1);
      }
      if (still || Math.floor(t * 2) % 2) { c.fillStyle = '#ffffff'; c.fillRect(1, h - 2, 2, 1); }
    } else if (st === 'done') {
      c.fillStyle = '#123018'; c.fillRect(0, 0, w, h);
      c.fillStyle = '#7bd88f';
      const cx = Math.floor(w / 2) - 4, cy = Math.floor(h / 2) - 2;
      [[0, 2], [1, 3], [2, 4], [3, 3], [4, 2], [5, 1], [6, 0]].forEach(([dx, dy]) => c.fillRect(cx + dx, cy + dy, 2, 1));
    } else if (st === 'waiting') {
      const on = still || Math.floor(t * 1.5) % 2 === 0;
      c.fillStyle = on ? '#ff5a1f' : '#7a2e14';
      const cx = Math.floor(w / 2) - 1; c.fillRect(cx, 2, 2, 4); c.fillRect(cx, 7, 2, 1);
    } else {
      c.fillStyle = '#0d1416'; c.fillRect(0, 0, w, h);
      const px = Math.floor((Math.sin(t * 0.7 + w) * 0.5 + 0.5) * (w - 3)), py = Math.floor((Math.cos(t * 0.5) * 0.5 + 0.5) * (h - 2));
      c.fillStyle = '#3b5a60'; c.fillRect(px, py, 3, 1);
    }
  }
  function drawScreens(ctx, list, t, still) {
    list.forEach((s) => {
      let mb = monBufs[s.key];
      if (!mb) mb = monBufs[s.key] = mkCanvas(s.fw, s.fh);
      paintScreen(s.key, mb[1], s.fw, s.fh, t, still);
      const [x0, y0] = iso(s.gx, s.gy, s.ztop).map(Math.round);
      for (let u = 0; u < s.fw; u++) {
        const off = s.face === 'l' ? ((u + 2) >> 1) : -((u + 2) >> 1);
        ctx.drawImage(mb[0], u, 0, 1, s.fh, x0 + 2 + u, y0 + 2 + off, 1, s.fh);
      }
    });
  }

  function marker(ctx, p, t, still) {
    const st = p.agent.status;
    const [px] = iso(p.gx, p.gy).map(Math.round);
    const bob = still ? 0 : Math.round(Math.sin(t * 2.4 + p.phase) * 1);
    const y = headTop(p) - 9 + bob;
    const sel = p.id === selected || p.id === hovered;
    const R = (dx, dy, w, h, c) => { ctx.fillStyle = c; ctx.fillRect(px + dx, y + dy, w, h); };
    if (st === 'working') {
      R(-6, 0, 13, 7, OUTLINE); R(-5, 1, 11, 5, '#f4f7f8');
      for (let k = 0; k < 3; k++) { const on = still ? k === 1 : Math.floor(t * 4) % 3 === k; R(-4 + k * 4, 3 - (on ? 1 : 0), 2, 2, on ? '#2a9db5' : '#9fb4bf'); }
      R(-1, 7, 3, 1, OUTLINE); R(0, 6, 1, 1, '#f4f7f8');
    } else if (st === 'done') {
      R(-4, 0, 9, 9, OUTLINE); R(-3, 1, 7, 7, '#4cbf6a');
      [[-2, 4], [-1, 5], [0, 4], [1, 3], [2, 2]].forEach(([dx, dy]) => R(dx, dy, 1, 1, '#ffffff'));
    } else if (st === 'waiting') {
      const pulse = still ? 0 : Math.floor(t * 2) % 2;
      R(-4, -pulse, 9, 10, OUTLINE); R(-3, 1 - pulse, 7, 8, '#ff5a1f');
      R(0, 2 - pulse, 1, 4, '#ffffff'); R(0, 7 - pulse, 1, 1, '#ffffff');
    } else {
      R(-3, 1, 7, 7, OUTLINE); R(-2, 2, 5, 5, '#6b7280'); R(-1, 3, 3, 3, '#cfd3d9');
    }
    if (sel) { R(-7, 11, 15, 1, '#ffffff'); }
  }

  function render(t) {
    const still = !playing;
    if (!ready) { drawStatic(sx); buildItems(); ready = true; }
    const ctx = bx;
    ctx.globalCompositeOperation = 'source-over'; ctx.globalAlpha = 1;
    ctx.drawImage(stat, 0, 0);
    const agents = data ? data.agents : [];

    // wall flats
    WIN.forEach((wd, i) => { const [c, x] = flats['win' + i]; windowFlat(x, c.width, c.height, t, i); shearR(ctx, c, wd.b, wd.z1); });
    { const [c, x] = flats.screen; screenFlat(x, c.width, c.height, agents, data ? data.owner_queue.length : 0, t, still); shearL(ctx, c, SCREEN.a, SCREEN.z1); }
    { const [c, x] = flats.board; whiteboardFlat(x, c.width, c.height, agents, t); shearL(ctx, c, BOARD.a, BOARD.z1); }
    { const [c, x] = flats.clock; if (!c.at || Math.abs(t - c.at) > 20) { clockFlat(x); c.at = t; } shearL(ctx, c, CLOCK.a, CLOCK.z1); }
    { const [c, x] = flats.rack; rackFlat(x, c.width, c.height, still ? 1 : t); shear(ctx, c, 'r', RACK.x + RACK.w, RACK.y + RACK.d, RACK.h - 4, 1, 0); }
    // coffee machine light
    { const [cx, cy] = iso(9.62, 0.53, 34).map(Math.round); ctx.fillStyle = still || Math.floor(t) % 4 ? '#ff6a3a' : '#7a2e14'; ctx.fillRect(cx, cy, 1, 1); }

    // sorted scene
    const list = items.map((it) => ({ depth: it.depth, draw: () => { ctx.drawImage(it.spr.c, it.spr.x, it.spr.y); if (it.screens) drawScreens(ctx, it.screens, t, still); } }));
    for (const id in DESKS) {
      const D = DESKS[id];
      const p = people.find((q) => q.id === id);
      list.push({
        depth: D.seat[0] + D.seat[1],
        draw: () => {
          ctx.drawImage(D.chairSeat.c, D.chairSeat.x, D.chairSeat.y);
          if (p && p.mode === 'sit') {
            const [pose, fr, flip] = poseOf(p, t);
            ctx.drawImage(sprite(p.id, p.look, pose, still ? 0 : fr, flip), D.sx - AX, D.sy - 1 - AY);
          }
          ctx.drawImage(D.chairBack.c, D.chairBack.x, D.chairBack.y);
        },
      });
    }
    for (const p of people) {
      if (p.mode === 'sit') continue;
      const [px, py] = iso(p.gx, p.gy).map(Math.round);
      list.push({
        depth: p.gx + p.gy + 0.05,
        draw: () => {
          ctx.fillStyle = 'rgba(18,8,32,0.35)';
          ctx.fillRect(px - 6, py - 1, 13, 2); ctx.fillRect(px - 4, py - 2, 9, 4);
          const [pose, fr, flip] = poseOf(p, t);
          ctx.drawImage(sprite(p.id, p.look, pose, still ? 0 : fr, flip), px - AX, py - AY);
        },
      });
    }
    list.sort((a, b) => a.depth - b.depth).forEach((it) => it.draw());

    // pendant lamps (hang from above the frame)
    PENDANTS.forEach(([gx, gy, z]) => {
      const [lx, ly] = iso(gx, gy, z).map(Math.round);
      ctx.fillStyle = '#16121c'; ctx.fillRect(lx, 0, 1, ly - 6);
      ctx.fillStyle = '#2a2633'; ctx.fillRect(lx - 4, ly - 6, 9, 2); ctx.fillRect(lx - 6, ly - 4, 13, 3);
      ctx.fillStyle = '#ffe7b0'; ctx.fillRect(lx - 4, ly - 1, 9, 1);
    });

    // ---- light ----
    ctx.globalCompositeOperation = 'multiply';
    ctx.fillStyle = '#b8a8d2'; ctx.fillRect(0, 0, W, H);                  // cool evening ambient
    const fade = ctx.createLinearGradient(0, H, W * 0.4, H * 0.4);         // darker toward the open front edge
    fade.addColorStop(0, 'rgba(90,70,120,1)'); fade.addColorStop(1, 'rgba(255,255,255,1)');
    ctx.fillStyle = fade; ctx.fillRect(0, 0, W, H);
    ctx.globalCompositeOperation = 'lighter';
    const glow = (x, y, r, rgb, a, sy = 1) => {
      ctx.save(); ctx.translate(x, y); ctx.scale(1, sy);
      const g = ctx.createRadialGradient(0, 0, 0, 0, 0, r);
      g.addColorStop(0, `rgba(${rgb},${a})`); g.addColorStop(1, `rgba(${rgb},0)`);
      ctx.fillStyle = g; ctx.fillRect(-r, -r, r * 2, r * 2); ctx.restore();
    };
    const flick = (k) => (still ? 0 : Math.sin(t * 3.1 + k) * 0.02 + (Math.sin(t * 17 + k * 3) > 0.985 ? -0.04 : 0));
    BEAMS.forEach((b, i) => {
      pathPoly(ctx, b.floorPts); ctx.fillStyle = 'rgba(255,150,70,0.16)'; ctx.fill();
      pathPoly(ctx, b.hull); ctx.fillStyle = `rgba(255,160,80,${0.07 + (still ? 0 : 0.012 * Math.sin(t * 0.8 + i))})`; ctx.fill();
      const [cx, cy] = iso(0, (WIN[i].a + WIN[i].b) / 2, 56); glow(cx + 4, cy, 40, '255,150,90', 0.14);
    });
    for (const id in DESKS) {
      const D = DESKS[id];
      const lp = D.o === 'x' ? iso(D.x + D.w - 0.3, D.y + 0.25, DESK_Z) : iso(D.x + 0.25, D.y + 0.25, DESK_Z);
      glow(lp[0] + 5, lp[1] - 4, 44, '255,190,110', 0.24 + flick(D.x), 0.6);
      glow(lp[0] + 5, lp[1] - 14, 10, '255,220,160', 0.35 + flick(D.x));
      const a = agents.find((q) => q.id === id);
      const col = { working: '120,210,235', done: '120,220,150', waiting: '255,110,60', idle: '80,110,130' }[a ? a.status : 'idle'];
      const ga = a && a.status === 'waiting' ? 0.1 : 0.2;
      const sp = D.o === 'x' ? iso(D.x + D.w / 2, D.y + 0.9, DESK_Z + 10) : iso(D.x + 0.9, D.y + D.d / 2, DESK_Z + 10);
      glow(sp[0], sp[1], 26, col, ga + (still ? 0 : 0.03 * Math.sin(t * 9 + D.x)));
    }
    PENDANTS.forEach(([gx, gy, z], i) => {
      const [lx, ly] = iso(gx, gy, z);
      glow(lx, ly, 16, '255,215,150', 0.45 + flick(i * 5));
      const [fx, fy] = iso(gx, gy, 14);
      glow(fx, fy, 42, '255,190,120', 0.2 + flick(i * 5), 0.5);
    });
    { const [cx, cy] = iso((SCREEN.a + SCREEN.b) / 2, 0.2, (SCREEN.z0 + SCREEN.z1) / 2); glow(cx, cy, 70, '110,180,220', 0.12, 0.8); }
    { const [cx, cy] = iso(RACK.x + RACK.w, RACK.y + RACK.d / 2, 30); glow(cx, cy, 20, '90,230,130', 0.1); }
    for (let gx = 0.6; gx < RX - 0.3; gx += 0.75) {
      const z = WALL - 5 - Math.abs(Math.sin(gx * 1.3)) * 4;
      const [lx, ly] = iso(gx, 0, z).map(Math.round);
      const on = still || Math.sin(t * 1.5 + gx * 4) > -0.6;
      ctx.fillStyle = on ? '#ffd88a' : '#7a5a3a'; ctx.fillRect(lx, ly, 2, 2);
      if (on) glow(lx + 1, ly + 1, 6, '255,200,120', 0.3);
    }
    DUST.forEach((d) => {
      const b = BEAMS[d.w];
      const tt = still ? 0 : t;
      const u = (d.u + tt * 0.01 * d.s) % 1, v = (d.v + Math.sin(tt * 0.3 * d.s + d.ph) * 0.04 + 1) % 1;
      const wp = b.wallPts, fp = b.floorPts;
      const top = [wp[3][0] + (wp[2][0] - wp[3][0]) * u, wp[3][1] + (wp[2][1] - wp[3][1]) * u];
      const bot = [fp[0][0] + (fp[1][0] - fp[0][0]) * u, fp[0][1] + (fp[1][1] - fp[0][1]) * u];
      const k = 0.15 + v * 0.8;
      const x = Math.round(top[0] + (bot[0] - top[0]) * k + v * 10), y = Math.round(top[1] + (bot[1] - top[1]) * k);
      ctx.fillStyle = `rgba(255,225,170,${0.25 + 0.35 * (0.5 + 0.5 * Math.sin(tt * 2 * d.s + d.ph))})`;
      ctx.fillRect(x, y, 1, 1);
    });
    ctx.globalCompositeOperation = 'source-over';

    // selection ring + status markers stay crisp on top of the light
    const selP = people.find((q) => q.id === selected);
    if (selP) {
      const [px, py] = iso(selP.gx, selP.gy).map(Math.round);
      ctx.fillStyle = (still || Math.floor(t * 3) % 2) ? '#ffffff' : '#ffd8c8';
      for (let k = 0; k < 48; k++) { const an = (k / 48) * Math.PI * 2; if (Math.sin(an) < -0.25) continue; ctx.fillRect(Math.round(px + Math.cos(an) * 11), Math.round(py + Math.sin(an) * 5), 1, 1); }
    }
    people.forEach((p) => marker(ctx, p, t, still));

    const vg = ctx.createRadialGradient(W / 2, H / 2, H * 0.4, W / 2, H / 2, H);
    vg.addColorStop(0, 'rgba(8,5,14,0)'); vg.addColorStop(1, 'rgba(8,5,14,0.35)');
    ctx.fillStyle = vg; ctx.fillRect(0, 0, W, H);

    out.imageSmoothingEnabled = false;
    out.drawImage(buf, 0, 0, canvas.width, canvas.height);
    placeOverlay();
  }

  // ---------- Overlay: hit areas, name chips, one tooltip ----------
  const stage = $('#stage');
  const hitLayer = $('#hits');
  const tip = $('#tip');
  const hits = new Map();

  function fitCanvas() {
    const cssW = stage.clientWidth || W;
    const k = Math.max(1, Math.ceil((cssW / W) * (window.devicePixelRatio || 1)));
    if (canvas.width !== W * k) { canvas.width = W * k; canvas.height = H * k; }
    if (!playing) render(simT); else placeOverlay();
  }

  function statusIcon(status) {
    const s = document.createElement('span');
    s.className = 'st st--' + status;
    s.setAttribute('aria-hidden', 'true');
    return s;
  }
  function taskText(a) {
    if (a.task) return a.task;
    if (a.status === 'idle') return 'Nothing assigned' + (a.last_result && a.last_result.text ? '. Last: ' + a.last_result.text : '');
    return STATUS_LABEL[a.status] || '';
  }
  function buildHits() {
    hitLayer.textContent = '';
    hits.clear();
    data.agents.forEach((a) => {
      if (!DESKS[a.id]) return;
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'hit';
      b.dataset.id = a.id;
      b.setAttribute('aria-label', `${a.name}, ${a.role}. ${STATUS_LABEL[a.status] || a.status}. ${taskText(a)}`);
      b.setAttribute('aria-pressed', 'false');
      const chip = document.createElement('span');
      chip.className = 'chip';
      chip.setAttribute('aria-hidden', 'true');
      chip.textContent = a.name;
      b.appendChild(chip);
      b.addEventListener('mouseenter', () => { hovered = a.id; updateTip(); });
      b.addEventListener('mouseleave', () => { if (hovered === a.id) hovered = null; updateTip(); });
      b.addEventListener('focus', () => { hovered = a.id; updateTip(); });
      b.addEventListener('blur', () => { if (hovered === a.id) hovered = null; updateTip(); });
      b.addEventListener('click', (e) => { e.stopPropagation(); select(selected === a.id ? null : a.id, { from: 'scene' }); });
      hitLayer.appendChild(b);
      hits.set(a.id, { el: b, key: '' });
    });
  }
  function scale() { return (stage.clientWidth || W) / W; }

  function placeOverlay() {
    const s = scale();
    people.forEach((p) => {
      const h = hits.get(p.id);
      if (!h) return;
      const [px, py] = iso(p.gx, p.gy);
      const top = headTop(p) - 11;
      const w = Math.max(30, 20 * s), hh = Math.max(40, (py - top) * s);
      const x = px * s - w / 2, y = py * s - hh;
      const key = `${x | 0},${y | 0},${w | 0},${hh | 0}`;
      if (h.key !== key) {
        h.el.style.transform = `translate(${x.toFixed(1)}px, ${y.toFixed(1)}px)`;
        h.el.style.width = w.toFixed(1) + 'px';
        h.el.style.height = hh.toFixed(1) + 'px';
        h.key = key;
      }
      h.el.classList.toggle('is-selected', p.id === selected);
    });
    positionTip();
  }

  let tipFor = null;
  function updateTip() {
    const id = hovered || selected;
    if (!id || !data) { tip.hidden = true; tipFor = null; return; }
    const a = data.agents.find((q) => q.id === id);
    if (!a) { tip.hidden = true; tipFor = null; return; }
    if (tipFor !== id) {
      tip.textContent = '';
      const head = document.createElement('p'); head.className = 'tip-head';
      const nm = document.createElement('b'); nm.textContent = a.name;
      const rl = document.createElement('span'); rl.className = 'tip-role'; rl.textContent = a.role;
      head.append(statusIcon(a.status), nm, rl);
      const st = document.createElement('p'); st.className = 'tip-status'; st.textContent = STATUS_LABEL[a.status] || a.status;
      const tk = document.createElement('p'); tk.className = 'tip-task'; tk.textContent = taskText(a);
      tip.append(head, st, tk);
      tip.dataset.status = a.status;
      tipFor = id;
    }
    tip.hidden = false;
    positionTip();
  }
  function positionTip() {
    if (tip.hidden || !tipFor) return;
    const p = people.find((q) => q.id === tipFor);
    if (!p) return;
    const s = scale(), sw = stage.clientWidth, sh = stage.clientHeight;
    const [px, py] = iso(p.gx, p.gy);
    const tw = tip.offsetWidth, th = tip.offsetHeight;
    let x = clamp(px * s - tw / 2, 6, sw - tw - 6);
    let y = (headTop(p) - 13) * s - th;
    let below = false;
    if (y < 6) { y = py * s + 8; below = true; }
    if (y + th > sh - 4) y = Math.max(6, sh - th - 4);
    tip.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
    tip.style.setProperty('--tail', clamp(Math.round(px * s - x), 12, tw - 12) + 'px');
    tip.classList.toggle('is-below', below);
  }
  stage.addEventListener('click', (e) => { if (e.target === canvas) select(null, { from: 'scene' }); });

  // ---------- Dashboard panel ----------
  const fmtTime = (s) => {
    const d = new Date(s);
    if (isNaN(d)) return { abs: '', rel: '' };
    const abs = d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
    const diff = Math.max(0, (Date.now() - d.getTime()) / 1000);
    const rel = diff < 60 ? 'just now' : diff < 3600 ? Math.round(diff / 60) + ' min ago' : diff < 86400 ? Math.round(diff / 3600) + ' h ago' : Math.round(diff / 86400) + ' d ago';
    return { abs, rel };
  };
  const safeUrl = (u) => (typeof u === 'string' && /^https:\/\//i.test(u) ? u : null);
  const portraits = {};
  function portrait(id) {
    if (portraits[id] !== undefined) return portraits[id];
    const [c, x] = mkCanvas(20, 20);
    x.fillStyle = '#2a2230'; x.fillRect(0, 0, 20, 20);
    const spr = sprite(id, LOOKS[id] || DEFAULT_LOOK, 'stand-front', 0, false);
    x.drawImage(spr, AX - 10, AY - 41 - ((LOOKS[id] || {}).tall ? 1 : 0), 20, 20, 0, 0, 20, 20);
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
      li.id = 'agent-' + a.id; li.dataset.id = a.id; li.dataset.status = a.status;
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
      if (a.last_result && a.last_result.text) res.append(link(a.last_result.text, a.last_result.url)); else res.append(document.createTextNode('Nothing yet'));
      const tm = fmtTime(a.updated_at);
      const when = el('p', 'agent-time');
      const time = el('time', null, tm.rel);
      time.dateTime = a.updated_at || ''; time.title = tm.abs;
      when.append(el('span', 'k', 'Updated'), time);
      body.append(task, res, when);
      const more = el('div', 'agent-more');
      more.id = 'more-' + a.id; more.hidden = true;
      if (a.about) more.append(el('p', null, a.about));
      more.append(el('p', 'mono', 'Updated ' + tm.abs + (a.updated_at ? ' (' + a.updated_at.replace('T', ' ').replace('Z', ' UTC') + ')' : '')));
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
    $('#summary').textContent = STATUSES.filter((s) => c[s]).map((s) => `${c[s]} ${s}`).join(' · ');
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
      li.querySelector('.agent-head').setAttribute('aria-expanded', String(on));
      li.querySelector('.agent-more').hidden = !on;
    });
    hits.forEach((h, k) => h.el.setAttribute('aria-pressed', String(k === selected)));
    tipFor = null;
    updateTip();
    if (selected && from === 'scene' && window.matchMedia('(min-width: 1101px)').matches) {
      const li = document.getElementById('agent-' + selected);
      if (li) li.scrollIntoView({ block: 'nearest', behavior: reduceMQ.matches ? 'auto' : 'smooth' });
    }
    const live = $('#selection');
    if (live && from !== 'refresh') {
      const a = data.agents.find((q) => q.id === selected);
      live.textContent = a ? `${a.name} selected: ${STATUS_LABEL[a.status]}. ${taskText(a)}` : '';
    }
    if (!playing) render(simT);
  }
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
  const fallback = () => normalise(JSON.parse(document.getElementById('status-fallback').textContent));
  async function load() {
    if (location.protocol === 'file:') return { d: fallback(), src: 'fallback' };
    try {
      const r = await fetch('status.json', { cache: 'no-store', credentials: 'same-origin' });
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
    if (changed) { buildPeople(); buildHits(); tipFor = null; }
    renderBoard();
    updateTip();
  }

  // ---------- Controls ----------
  const playBtn = $('#play');
  let last = 0, acc = 0;
  function setPlaying(on) {
    playing = on;
    playBtn.setAttribute('aria-pressed', String(!on));
    playBtn.textContent = on ? 'Pause animation' : 'Play animation';
    stage.classList.toggle('is-paused', !on);
    if (on) { last = performance.now(); requestAnimationFrame(loop); } else render(simT);
  }
  playBtn.addEventListener('click', () => setPlaying(!playing));

  const namesBtn = $('#names');
  function setNames(on) {
    showNames = on;
    namesBtn.setAttribute('aria-pressed', String(on));
    stage.classList.toggle('show-names', on);
    try { localStorage.setItem('office-names', on ? '1' : '0'); } catch (e) { /* fine */ }
  }
  if (!params.has('names')) { try { showNames = localStorage.getItem('office-names') === '1'; } catch (e) { /* fine */ } }
  namesBtn.addEventListener('click', () => setNames(!showNames));
  setNames(showNames);

  const themeBtn = $('#theme');
  const root = document.documentElement;
  const sysDark = () => window.matchMedia('(prefers-color-scheme: dark)').matches;
  function applyTheme(mode) {
    if (mode === 'light' || mode === 'dark') root.dataset.theme = mode; else delete root.dataset.theme;
    const isDark = mode ? mode === 'dark' : sysDark();
    themeBtn.textContent = isDark ? 'Light theme' : 'Dark theme';
    themeBtn.setAttribute('aria-label', isDark ? 'Switch to light theme' : 'Switch to dark theme');
  }
  let themePref = params.get('theme');
  if (!themePref) { try { themePref = localStorage.getItem('office-theme'); } catch (e) { themePref = null; } }
  applyTheme(themePref);
  themeBtn.addEventListener('click', () => {
    const isDark = root.dataset.theme ? root.dataset.theme === 'dark' : sysDark();
    themePref = isDark ? 'light' : 'dark';
    applyTheme(themePref);
    try { localStorage.setItem('office-theme', themePref); } catch (e) { /* fine */ }
  });

  function loop(now) {
    if (!playing) return;
    const dt = Math.min(0.1, (now - last) / 1000);
    last = now; acc += dt;
    if (acc >= 1 / FPS) { step(acc); acc = 0; render(simT); }
    requestAnimationFrame(loop);
  }

  async function start() {
    await refresh();
    const at = parseFloat(params.get('at'));
    if (at > 0) { const n = Math.round(at * 20); for (let i = 0; i < n; i++) step(0.05); }
    fitCanvas();
    if ('ResizeObserver' in window) new ResizeObserver(fitCanvas).observe(stage); else window.addEventListener('resize', fitCanvas);
    const still = reduceMQ.matches || params.get('still') === '1';
    if (params.get('select')) select(params.get('select'), { from: 'param' });
    setPlaying(!still);
    if (reduceMQ.addEventListener) reduceMQ.addEventListener('change', (e) => setPlaying(!e.matches));
    if (location.protocol !== 'file:') setInterval(refresh, REFRESH_MS);
    setInterval(() => data && renderBoard(), 30000);
  }
  start();
})();

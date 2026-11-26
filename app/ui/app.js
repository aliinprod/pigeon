// @ts-check
// Pigeon app UI. Plain JS, no framework, no build step, no network: it only
// talks to the Rust backend through Tauri IPC (`window.__TAURI__.core.invoke`).
// Everything that came from another computer (names, paths, errors) is
// escaped before it is put into HTML. No inline styles or handlers (CSP).
'use strict';

(() => {
  /** @type {(cmd: string, args?: object) => Promise<any>} */
  const invoke = (cmd, args) => /** @type {any} */ (window).__TAURI__.core.invoke(cmd, args);

  // ---------- Icons (same paths as app/design) ----------
  const P = {
    folder: '<path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l2 2h9A1.5 1.5 0 0 1 21 9.5v8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"/>',
    file: '<path d="M6 3h8l4 4v13a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z"/><path d="M14 3v4h4"/>',
    search: '<circle cx="11" cy="11" r="6.5"/><path d="m16 16 4.5 4.5"/>',
    copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3"/>',
    check: '<path d="m5 12.5 4.5 4.5L19 7.5"/>',
    down: '<path d="M12 4v12m-5-5 5 5 5-5M5 20h14"/>',
    up: '<path d="M12 20V8m-5 5 5-5 5 5M5 4h14"/>',
    chev: '<path d="m9 6 6 6-6 6"/>',
    laptop: '<rect x="4" y="5" width="16" height="11" rx="1.5"/><path d="M2 19h20"/>',
    pc: '<rect x="3" y="4" width="18" height="12" rx="1.5"/><path d="M9 20h6M12 16v4"/>',
    gear: '<circle cx="12" cy="12" r="3"/><path d="M12 2.5v3M12 18.5v3M4.2 6.2l2.1 2.1M17.7 15.7l2.1 2.1M2.5 12h3M18.5 12h3M4.2 17.8l2.1-2.1M17.7 8.3l2.1-2.1"/>',
    shield: '<path d="M12 3 5 6v5c0 4.5 3 8 7 10 4-2 7-5.5 7-10V6z"/>',
    refresh: '<path d="M20 11a8 8 0 0 0-14.5-4.5M4 4v3h3M4 13a8 8 0 0 0 14.5 4.5M20 20v-3h-3"/>',
    x: '<path d="M6 6l12 12M18 6 6 18"/>',
    info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>',
    devices: '<rect x="3" y="5" width="13" height="9" rx="1.2"/><path d="M1.5 17h16"/><rect x="17" y="8" width="5" height="11" rx="1"/>',
    clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
    alert: '<path d="M12 3 2 20h20z"/><path d="M12 10v4M12 17h.01"/>',
    open: '<path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    share: '<path d="M12 15V3m-4 4 4-4 4 4"/><path d="M5 12v7a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-7"/>',
    qr: '<rect x="4" y="4" width="6" height="6"/><rect x="14" y="4" width="6" height="6"/><rect x="4" y="14" width="6" height="6"/><path d="M14 14h2v2h-2zM18 18h2v2h-2zM14 18h2"/>'
  };
  const ic = (/** @type {keyof typeof P} */ n, cls = '') => `<svg class="ico ${cls}" viewBox="0 0 24 24" aria-hidden="true">${P[n] || ''}</svg>`;
  const LOGO = (cls = '') => `<img class="logo ${cls}" src="logo.webp" alt="">`;
  const esc = (/** @type {any} */ s) => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c] || c));
  /** @param {string} label @param {{k?:string,go?:string,act?:string,i?:keyof typeof P,data?:Record<string,any>,dis?:boolean,aria?:string}} o */
  const B = (label, o = {}) => {
    const data = Object.entries(o.data || {}).map(([k, v]) => ` data-${k}="${esc(v)}"`).join('');
    return `<button class="btn ${o.k || ''}"${o.go ? ` data-go="${o.go}"` : ''}${o.act ? ` data-act="${o.act}"` : ''}${data}${o.dis ? ' disabled' : ''}${o.aria ? ` aria-label="${esc(o.aria)}"` : ''}>${o.i ? ic(o.i) : ''}${label ? `<span class="lbl">${esc(label)}</span>` : ''}</button>`;
  };
  const flow = (/** @type {string} */ from, /** @type {string} */ to) => `<div class="flow"><span class="n">${esc(from)}</span><span class="ln"></span><span class="n">${esc(to)}</span></div>`;
  const pill = (/** @type {boolean|null|undefined} */ on) => on === true ? '<span class="pill"><span class="dot"></span>Online</span>'
    : on === false ? '<span class="pill off"><span class="dot"></span>Offline</span>' : '<span class="pill off"><span class="dot"></span>Not checked</span>';
  const fp = (/** @type {string} */ f) => { const u = String(f || '').toUpperCase(); return `${u.slice(0, 4)} ${u.slice(4, 8)}`; };
  const size = (/** @type {number} */ n) => n < 1000 ? `${n} B` : n < 1e6 ? `${(n / 1e3).toFixed(n < 1e4 ? 1 : 0)} KB` : n < 1e9 ? `${(n / 1e6).toFixed(n < 1e7 ? 1 : 0)} MB` : `${(n / 1e9).toFixed(2)} GB`;
  const when = (/** @type {number} */ secs) => {
    if (!secs) return '';
    const d = new Date(secs * 1000), now = new Date();
    const t = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    if (d.toDateString() === now.toDateString()) return `Today, ${t}`;
    return `${d.toLocaleDateString([], { day: 'numeric', month: 'short', year: 'numeric' })}, ${t}`;
  };
  const left = (/** @type {number} */ until) => {
    const s = Math.max(0, until - Math.floor(Date.now() / 1000));
    return s >= 3600 ? `${Math.floor(s / 3600)} h ${Math.floor((s % 3600) / 60)} m` : s >= 60 ? `${Math.floor(s / 60)} min` : `${s} s`;
  };
  const tries = (/** @type {number} */ n) => `${n} ${n === 1 ? 'try' : 'tries'}`;

  // ---------- State ----------
  /** @type {any} */ let S = null;           // last backend status
  /** @type {string|null} */ let fatal = null;
  /** @type {string[]} */ let WORDS = [];
  const st = {
    screen: 'loading',
    /** @type {any} */ browse: { computer: '', path: '', entries: null, roots: [], error: '', q: '', sel: '', loading: false },
    /** @type {any} */ getPair: { link: '', info: null, name: '', error: '', outcome: null, busy: false, code: ['', '', '', ''] },
    inviteName: '',
    /** @type {any} */ dialog: null,
    /** @type {string} */ toast: '', toastAttn: false
  };
  const thisX = () => !S ? 'this computer' : S.me.os === 'windows' ? 'this PC' : S.me.os === 'macos' ? 'this Mac' : 'this computer';
  const ThisX = () => thisX().replace(/^t/, 'T');
  const meIco = () => S && S.me.os === 'macos' ? 'laptop' : 'pc';
  const computer = (/** @type {string} */ id) => S && S.computers.find((/** @type {any} */ c) => c.id === id);

  // ---------- Theme (per-viewer convenience; works without storage) ----------
  const getTheme = () => { try { return localStorage.getItem('pigeon-theme') || 'system'; } catch (_) { return 'system'; } };
  const applyTheme = (/** @type {string} */ t) => {
    if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t; else delete document.documentElement.dataset.theme;
  };
  const setTheme = (/** @type {string} */ t) => { try { localStorage.setItem('pigeon-theme', t); } catch (_) {} applyTheme(t); };
  applyTheme(getTheme());

  // ---------- Sidebar ----------
  const NAV = { browse: 'get', transfers: 'xfer', folders: 'share', devices: 'dev', 'pair-choose': 'pair', invite: 'pair', 'g-link': 'pair', 'g-code': 'pair', 'g-paired': 'pair', settings: 'set' };
  function sidebar() {
    if (!S) return '';
    const k = /** @type {any} */ (NAV)[st.screen] || '';
    const active = S.transfers.filter((/** @type {any} */ t) => t.state === 'active').length;
    const comps = S.computers.map((/** @type {any} */ c) => `<button class="nav" data-act="browse" data-id="${esc(c.id)}" aria-current="${k === 'get' && st.browse.computer === c.id}">${ic('laptop')}<span class="nm">${esc(c.name)}</span><span class="st"><span class="dot ${c.online === false ? 'off' : c.online == null ? 'unknown' : ''}"></span></span></button>`).join('');
    return `<div class="me"><div class="me-ico">${ic(meIco())}</div><div class="grow"><div class="h3 trunc">${esc(S.me.name)}</div><div class="small muted">${ThisX()} · ${S.online ? 'online' : 'offline'}</div></div></div>
      <div class="sec">${ic('down')}Get from</div>
      ${comps || `<div class="hint">No computers yet</div>`}
      <button class="nav" data-go="transfers" aria-current="${k === 'xfer'}">${ic('down')}<span class="nm">Transfers</span>${active ? `<span class="count">${active}</span>` : ''}</button>
      <div class="sec">${ic('share')}Share</div>
      <button class="nav" data-go="folders" aria-current="${k === 'share'}">${ic('folder')}<span class="nm">My shared folders</span><span class="st">${S.folders.length || ''}</span></button>
      <div class="sec">Devices</div>
      <button class="nav" data-go="devices" aria-current="${k === 'dev'}">${ic('devices')}<span class="nm">Devices</span><span class="st">${peers().length || ''}</span></button>
      <button class="nav" data-go="pair-choose" aria-current="${k === 'pair'}">${ic('plus')}<span class="nm">Pair a device</span></button>
      <div class="spacer"></div>
      <button class="nav" data-go="settings" aria-current="${k === 'set'}">${ic('gear')}<span class="nm">Settings</span></button>`;
  }

  // ---------- Screens ----------
  const main = (/** @type {string} */ inner, center = false) => ({ html: `<div class="scroll">${inner}</div>`, center });

  function welcome() {
    return main(`<div class="pair-card wide">${LOGO('hero')}
      <h1 class="h1 mt8">What do you want to do on ${thisX()}?</h1>
      <p class="muted lead">You can do both, and change it any time. Nothing is shared until you pick folders.</p>
      <div class="choices">
        <button class="choice" data-act="welcome" data-to="folders"><span class="kicker">${ic('share')}Share</span><div class="h2">Share folders from ${thisX()}</div>
          <p class="small muted">Pick folders. Your other computers can get files from them, read-only, after you pair them.</p></button>
        <button class="choice" data-act="welcome" data-to="g-link"><span class="kicker">${ic('down')}Get</span><div class="h2">Get files from another computer</div>
          <p class="small muted">Browse and download from a computer you own that runs Pigeon and shares folders.</p></button>
      </div>
      <div class="row end mt16">${B('Skip for now', { act: 'welcome', data: { to: 'devices' } })}</div></div>`, true);
  }

  function folders() {
    const devs = S.devices;
    const rows = S.folders.map((/** @type {any} */ f) => `<div class="li">${ic('folder', 'lg')}<div class="grow"><div class="h3">${esc(f.name)}</div><div class="small muted mono trunc">${esc(f.path)}${f.available ? '' : ' · <span class="attn-text">not available</span>'}</div></div><span class="tag">Read-only</span>${B('Stop sharing', { k: 'sm', act: 'rm-folder', data: { name: f.name } })}</div>`).join('');
    return main(`<div class="m-head"><div class="grow"><div class="kicker">${ic('share')}Share</div><div class="h1 mt4">My shared folders</div></div>${B('Add folder…', { i: 'plus', act: 'add-folder', k: S.folders.length ? '' : 'primary' })}</div>
      <div class="m-content">
        <div class="list">${rows || `<div class="empty">Nothing is shared yet. Add a folder: paired devices can then get files from it, read-only.</div>`}</div>
        <div class="label mt20">Who can get these</div>
        <div class="list mt8">${devs.map((/** @type {any} */ d) => `<div class="li">${ic('pc')}<span class="grow"><b>${esc(d.name)}</b> <span class="muted small">can get from ${thisX()} · <span class="fp">${fp(d.fingerprint)}</span></span></span>${B('Revoke', { k: 'sm danger', act: 'revoke', data: { id: d.id, name: d.name } })}</div>`).join('')}
          <div class="li">${B('Let another device get files', { k: 'sm ghost', i: 'plus', go: 'invite' })}</div></div>
        <div class="banner mt16">${ic('shield')}<p>Devices see these folders and nothing else. Links that point outside a shared folder are skipped. Pigeon never writes to them.</p></div>
      </div>`);
  }

  function pairChoose() {
    return main(`<div class="m-head"><div class="grow"><div class="h1">Pair a device</div><div class="small muted">Which way should files go?</div></div></div>
      <div class="m-content mid">
        <div class="choices">
          <button class="choice" data-go="invite">${flow(S.me.name, 'Other device')}<div><div class="h2">Let a device get files from ${thisX()}</div>
            <p class="small muted mt4">You get a link and a 4-word code here. The other device pastes the link and types the code.</p></div><span class="kicker">${ic('share')}Share</span></button>
          <button class="choice" data-act="new-get-pair">${flow('Other device', S.me.name)}<div><div class="h2">Get files from another device</div>
            <p class="small muted mt4">Start on that device: Pair a device → Let a device get files. Then paste its link here and type its code.</p></div><span class="kicker">${ic('down')}Get</span></button>
        </div>
        <div class="banner mt16">${ic('info')}<p><b>Each pairing works one way.</b> The invite is always made on the computer that shares. To get files both ways, pair twice, once from each side.</p></div>
      </div>`);
  }

  function invite() {
    const inv = S.invite;
    if (!inv) {
      return main(`<div class="m-head"><div class="grow"><div class="kicker">${ic('share')}Share · invite</div><div class="h1 mt4">Let a device get files from ${thisX()}</div></div></div>
        <div class="m-content narrow">
          <div class="card pad mb14">${flow(S.me.name, 'The device that pairs')}<div class="small muted mt8">It will be able to get files from ${thisX()}'s shared folders, read-only. This doesn't let ${thisX()} get files from it.</div></div>
          ${S.folders.length ? '' : `<div class="banner attn mb14">${ic('alert')}<p><b>Nothing is shared yet.</b> The device can pair now, but it sees an empty list until you add a folder.</p></div>`}
          <label class="small h3" for="dev-name">What is the other device called? <span class="muted">(optional)</span></label>
          <div class="row mt4"><input class="field narrow" id="dev-name" maxlength="64" placeholder="e.g. Home PC" value="${esc(st.inviteName)}" autocomplete="off">${B('Make invite', { k: 'primary', act: 'make-invite' })}</div>
          <p class="small muted mt8">Single use, 3 tries, valid 12 hours. You can cancel it any time.</p>
        </div>`);
    }
    if (inv.state === 'paired') {
      const d = inv.device;
      return main(`<div class="pair-card">
        <div class="big-check">${ic('check', 'lg')}</div>
        <h1 class="h1">${esc(d.name)} can now get files from ${thisX()}</h1>
        <p class="muted lead">Read-only, from your shared folders. You can revoke it any time in Devices.</p>
        <div class="card pad12 mb12">${flow(S.me.name, d.name)}
          <div class="row mt12"><span class="label grow">${esc(d.name)}'s fingerprint</span><span class="fp">${fp(d.fingerprint)}</span></div>
          <div class="small muted">${esc(d.name)} shows the same code for itself.</div></div>
        <div class="banner">${ic('info')}<p><b>Want to get files from ${esc(d.name)} too?</b> Pairing works one way. Start on ${esc(d.name)}: <i>Pair a device → Let a device get files</i>, then paste its link here.</p></div>
        <div class="row between wrap mt16">${B('Paste a link', { act: 'dismiss-invite', data: { to: 'g-link' } })}${B('Done', { k: 'primary', act: 'dismiss-invite', data: { to: 'devices' } })}</div>
      </div>`, true);
    }
    if (inv.state === 'burned' || inv.state === 'expired') {
      const burned = inv.state === 'burned';
      return main(`<div class="pair-card">
        <div class="big-check ${burned ? 'big-x' : ''}">${ic(burned ? 'x' : 'clock', 'lg')}</div>
        <h1 class="h1">${burned ? 'This invite is used up' : 'This invite expired'}</h1>
        <p class="muted lead">${burned ? '3 wrong codes were typed, so it stopped working. Nothing was shared. If you didn’t type them, someone else has your link: make a new invite and keep the code on screen.' : 'Invites last 12 hours. Nobody paired with it. Make a new one when you’re ready.'}</p>
        <div class="row">${B('Make a new invite', { k: 'primary', act: 'dismiss-invite', data: { to: 'invite' } })}${B('Close', { act: 'dismiss-invite', data: { to: 'devices' } })}</div></div>`, true);
    }
    // Waiting.
    const wrong = inv.attempts_left < 3;
    const finishing = !inv.code.length;
    return main(`<div class="m-head"><div class="grow"><div class="kicker">${ic('share')}Share · invite${inv.device_name ? ` for ${esc(inv.device_name)}` : ''}</div><div class="h1 mt4">Let a device get files from ${thisX()}</div></div>
        <span class="small muted mono" id="expires">Expires in ${left(inv.expires_at)}</span></div>
      <div class="m-content narrow">
        <div class="card pad mb14">${flow(S.me.name, inv.device_name || 'The device that pairs')}<div class="small muted mt8">It will be able to get files from ${thisX()}'s shared folders, read-only. This doesn't let ${thisX()} get files from it.</div></div>
        ${wrong && !finishing ? `<div class="banner attn mb14">${ic('alert')}<p><b>Someone typed a wrong code. ${tries(inv.attempts_left)} left.</b> If that wasn't you, cancel this invite.</p></div>` : ''}
        <div class="row top"><span class="step-n">1</span><div class="grow">
          <div class="h3">Send this link to the other device</div>
          <div class="small muted mt4 mb12">Email or chat to yourself is fine. The link alone can't pair.</div>
          <div class="linkbox"><code id="link">${esc(inv.link)}</code>${B('Copy', { i: 'copy', act: 'copy-link' })}</div></div></div>
        <div class="row top mt16"><span class="step-n">2</span><div class="grow">
          <div class="h3">On that device, type this code</div>
          <div class="small muted mt4 mb12">Don't send it. Read it from here, or take a photo with your phone.</div>
          ${finishing ? '<div class="row small muted"><span class="spinner"></span>Finishing the pairing…</div>'
            : `<div class="code-words" id="code">${inv.code.map((/** @type {string} */ w, /** @type {number} */ i) => `<div><b>${esc(w)}</b><small>${i + 1}</small></div>`).join('')}</div>`}</div></div>
        <div class="row wrap small muted mt12"><span class="row"><span class="spinner"></span>Waiting for a device…</span><span>Single use</span><span>${tries(inv.attempts_left)}${wrong ? ' left' : ''}</span></div>
        <div class="row wrap mt16">${B('Show QR', { i: 'qr', dis: true })}<span class="small muted grow">QR comes later</span>${B('Cancel invite', { k: wrong ? 'danger' : '', act: 'cancel-invite' })}</div>
      </div>`);
  }

  function devicesScreen() {
    const list = peers();
    const cards = list.map(p => {
      const up = p.dev ? `<div class="dir"><span class="arrow">${ic('up')}</span><span class="grow"><b>${esc(p.dev.name)}</b> can get files from ${thisX()} <span class="small muted">· since ${esc(when(p.dev.paired_at))}</span></span>${B('Revoke', { k: 'sm danger', act: 'revoke', data: { id: p.id, name: p.dev.name } })}</div>`
        : `<div class="dir no"><span class="arrow">${ic('up')}</span><span class="grow">${esc(p.name)} can't get files from ${thisX()}</span>${B('Let it get files', { k: 'sm', go: 'invite' })}</div>`;
      const down = p.comp ? `<div class="dir"><span class="arrow">${ic('down')}</span><span class="grow"><b>${ThisX()}</b> can get files from ${esc(p.comp.name)} <span class="small muted">· since ${esc(when(p.comp.paired_at))}</span></span>${B('Browse', { k: 'sm', act: 'browse', data: { id: p.id } })}${B('Forget', { k: 'sm danger', act: 'forget', data: { id: p.id, name: p.comp.name } })}</div>`
        : `<div class="dir no"><span class="arrow">${ic('down')}</span><span class="grow">${ThisX()} can't get files from ${esc(p.name)}. <span class="small">To add it, start on ${esc(p.name)}: Pair a device → Let a device get files.</span></span>${B('Paste a link', { k: 'sm', act: 'new-get-pair' })}</div>`;
      return `<div class="card mb12"><div class="row wrap card-head pad12">${ic('pc', 'lg')}<div class="grow"><div class="h3">${esc(p.name)}</div><div class="small muted"><span class="fp">${fp(p.fingerprint)}</span></div></div>${p.comp ? pill(p.comp.online) : ''}</div>${up}${down}</div>`;
    }).join('');
    return main(`<div class="m-head"><div class="grow"><div class="h1">Devices</div><div class="small muted">Each pairing works one way. Both directions are listed separately.</div></div>${B('Pair a device', { k: 'primary', i: 'plus', go: 'pair-choose' })}</div>
      <div class="m-content">${cards || `<div class="list"><div class="empty">No devices yet. Pair one to share or get files.</div></div>`}
        <p class="small muted">Fingerprint = first 8 characters of a device key. Compare it with what that device shows for itself. This computer is <span class="fp">${fp(S.me.fingerprint)}</span>.</p></div>`);
  }

  /** Devices and computers merged by key: one card, two directions. */
  function peers() {
    if (!S) return [];
    /** @type {Map<string, any>} */ const m = new Map();
    for (const d of S.devices) m.set(d.id, { id: d.id, name: d.name, fingerprint: d.fingerprint, dev: d, comp: null });
    for (const c of S.computers) {
      const p = m.get(c.id);
      if (p) { p.comp = c; p.name = c.name; } else m.set(c.id, { id: c.id, name: c.name, fingerprint: c.fingerprint, dev: null, comp: c });
    }
    return [...m.values()];
  }

  const getShell = (/** @type {string} */ inner, /** @type {number} */ step) => main(`<div class="pair-card"><div class="kicker">${ic('down')}Get · pair</div>
    ${step ? `<div class="stepper"><i class="on"></i><i class="${step > 1 ? 'on' : ''}"></i></div><div class="small muted mt4">Step ${step} of 2</div>` : ''}${inner}</div>`, true);

  function gLink() {
    const g = st.getPair, info = g.info;
    const card = info ? (info.expired
      ? `<div class="banner attn mt12">${ic('clock')}<p><b>This invite expired.</b> On that computer, click Pair a device to make a new one.</p></div>`
      : `<div class="card pad mt12"><div class="row">${ic('check')}<div class="grow small"><b>Invite from computer <span class="fp">${fp(info.fingerprint)}</span></b><div class="muted">Expires in ${left(info.expires_at)} · single use</div></div></div>
        <div class="mt8">${flow(g.name || 'That computer', S.me.name)}</div><div class="small muted mt8">${ThisX()} will be able to get files from it. Not the other way.</div>
        ${info.already_paired ? `<div class="small muted mt8">${ThisX()} is already paired with this computer; pairing again renews it.</div>` : ''}
        <label class="small h3 mt12" for="cname">What do you call that computer?</label>
        <input class="field narrow mt4" id="cname" maxlength="64" placeholder="e.g. Work Mac" value="${esc(g.name)}" autocomplete="off"></div>`)
      : g.error ? `<div class="banner attn mt12">${ic('alert')}<p>${esc(g.error)}</p></div>` : '';
    return getShell(`<h1 class="h1 mt12">Get files from another device</h1>
      <p class="muted lead">On that device, open Pigeon → <b>Pair a device</b> → <b>Let a device get files</b>. Paste the link it gives you.</p>
      <label class="small h3" for="lnk">Invite link</label>
      <div class="row mt4"><input class="field mono" id="lnk" value="${esc(g.link)}" placeholder="pigeon-invite-…" autocomplete="off" spellcheck="false"></div>
      ${card}
      <div class="row end mt16">${B('Next', { k: 'primary lg', act: 'to-code', dis: !info || info.expired })}</div>`, 1);
  }

  function gCode() {
    const g = st.getPair, o = g.outcome;
    const name = g.name || `computer ${fp(g.info ? g.info.fingerprint : '')}`;
    const wrong = o && o.result === 'wrong_code';
    const banner = !o ? '' : wrong ? `<div class="banner attn mb14">${ic('alert')}<p><b>That code didn't match. ${tries(o.attempts_left)} left.</b> Check each word on ${esc(name)}'s screen. After ${o.attempts_left === 1 ? 'one more wrong code' : `${o.attempts_left} more wrong codes`}, the invite stops working.</p></div>`
      : o.result === 'typo' ? `<div class="banner mb14">${ic('info')}<p>${esc(o.message)}. That didn't use a try.</p></div>`
      : `<div class="banner attn mb14">${ic('alert')}<p>${esc(o.message || 'Pairing failed.')}</p></div>`;
    return getShell(`<h1 class="h1 mt12">Type the code from ${esc(name)}</h1>
      <p class="muted lead">4 words, shown on ${esc(name)} next to the link. Not case-sensitive.</p>
      ${banner}
      <div class="wordin" id="wordin">${[0, 1, 2, 3].map(i => `<div class="wbox ${wrong ? 'bad' : ''}"><input data-w="${i}" autocomplete="off" autocapitalize="off" spellcheck="false" aria-label="Word ${i + 1}"><span class="ghost"></span><span class="idx">${i + 1}</span></div>`).join('')}</div>
      <div class="small muted mt8">Tip: paste the whole code, or press <span class="kbd">Tab</span> to accept a word.</div>
      <div class="row between mt16">${B('Back', { go: 'g-link' })}${g.busy ? '<span class="row small muted"><span class="spinner"></span>Pairing…</span>' : B('Pair', { k: 'primary lg', act: 'pair' })}</div>`, 2);
  }

  function gPaired() {
    const o = st.getPair.outcome;
    if (!o || o.result !== 'paired') return gLink();
    const c = o.computer;
    return getShell(`<div class="big-check mt12">${ic('check', 'lg')}</div>
      <h1 class="h1">${ThisX()} can now get files from ${esc(c.name)}</h1>
      <p class="muted lead">Check these match what ${esc(c.name)} shows. If they don't, click Forget in Devices and pair again.</p>
      <div class="fp-pair"><div class="card">${ic('laptop')}<div class="label mt4">${esc(c.name)}</div><span class="fp">${fp(c.fingerprint)}</span></div>
        <div class="card">${ic(meIco())}<div class="label mt4">${ThisX()}</div><span class="fp">${fp(o.my_fingerprint)}</span></div></div>
      <div class="banner mt12">${ic('info')}<p><b>Should ${esc(c.name)} get files from ${thisX()} too?</b> That's a second pairing, and it starts here: ${thisX()} makes the invite, ${esc(c.name)} types the code.</p></div>
      <div class="row between wrap mt16">${B(`Let ${c.name} get from ${thisX()}`, { go: 'invite', i: 'share' })}${B(`Browse ${c.name}`, { k: 'primary', act: 'browse', data: { id: c.id } })}</div>`, 0);
  }

  function gProblem(/** @type {string} */ kind) {
    const C = {
      used_up: ['x', 'This invite is used up', '3 wrong codes were typed, so it stopped working. Make a new invite on the other computer and type the code exactly as shown.'],
      expired: ['clock', 'This invite expired', 'Invites last 12 hours. On the other computer, click Pair a device to make a new one.'],
      already_used: ['alert', 'This invite was already used', 'Each invite pairs one device, once. If you didn’t pair with it, someone else may have: check Devices on that computer. It may also have been cancelled or replaced, or Pigeon isn’t running there. Make a new invite on that computer.']
    }[kind] || ['alert', 'Pairing failed', ''];
    return getShell(`<div class="big-check ${kind !== 'expired' ? 'big-x' : ''} mt12">${ic(/** @type {any} */ (C[0]), 'lg')}</div>
      <h1 class="h1">${C[1]}</h1><p class="muted lead">${C[2]}</p>
      <div class="row">${B('Paste a new link', { k: 'primary', act: 'new-get-pair' })}</div>`, 0);
  }

  function browse() {
    const b = st.browse, c = computer(b.computer);
    if (!c) return main(`<div class="center-msg muted">That computer is not paired any more.</div>`);
    const name = c.name;
    const parts = b.path ? b.path.split('/') : [];
    let crumbs = parts.length ? `<button data-act="open" data-path="">${esc(name)}</button>` : `<span class="cur">${esc(name)}</span>`;
    parts.forEach((/** @type {string} */ p, /** @type {number} */ i) => {
      const path = parts.slice(0, i + 1).join('/');
      crumbs += ic('chev') + (i === parts.length - 1 ? `<span class="cur">${esc(p)}</span>` : `<button data-act="open" data-path="${esc(path)}">${esc(p)}</button>`);
    });
    const q = b.q.trim().toLowerCase();
    const rows = (b.entries || []).filter((/** @type {any} */ e) => !q || e.name.toLowerCase().includes(q))
      .sort((/** @type {any} */ x, /** @type {any} */ y) => (y.is_dir - x.is_dir) || x.name.localeCompare(y.name));
    const table = rows.length ? `<table class="files"><thead><tr><th>Name</th><th class="num">Size</th><th class="hide-sm">Modified</th><th></th></tr></thead><tbody>
      ${rows.map((/** @type {any} */ e) => e.is_dir
        ? `<tr class="f folder" data-act="open" data-path="${esc(e.path)}"><td class="name"><div class="row">${ic('folder')}<span class="trunc">${esc(e.name)}</span></div></td><td class="num"></td><td class="hide-sm muted small">${esc(when(e.mtime))}</td><td class="act">${ic('chev')}</td></tr>`
        : `<tr class="f ${b.sel === e.path ? 'sel' : ''}" data-act="sel" data-path="${esc(e.path)}"><td class="name"><div class="row">${ic('file')}<span class="trunc">${esc(e.name)}</span></div></td><td class="num">${size(e.size)}</td><td class="hide-sm muted small">${esc(when(e.mtime))}</td><td class="act">${B('Get', { k: 'sm getbtn', i: 'down', act: 'get', aria: `Get ${e.name}`, data: { path: e.path } })}</td></tr>`).join('')}
      </tbody></table>`
      : b.loading ? `<div class="row small muted"><span class="spinner"></span>Asking ${esc(name)}…</div>`
      : b.error ? ''
      : q ? `<p class="muted center-msg">No names in this folder match “${esc(b.q)}”.</p>`
      : `<p class="muted center-msg">${b.path ? 'This folder is empty.' : `${esc(name)} doesn't share any folders yet.`}</p>`;
    const offline = c.online === false;
    return main(`<div class="m-head tight"><div class="kicker">${ic('down')}Get from ${esc(name)} ${b.roots.length ? ic('chev') : ''} ${b.roots.map((/** @type {string} */ r) => `<button class="chip" data-act="open" data-path="${esc(r)}" aria-current="${parts[0] === r}">${esc(r)}</button>`).join(' ')}</div></div>
      <div class="m-head tight2"><div class="crumbs">${crumbs}</div><div class="grow"></div>
        <div class="search">${ic('search')}<input class="field" id="q" placeholder="Search this folder" value="${esc(b.q)}" aria-label="Search file names in this folder"></div><span id="online-pill">${pill(c.online)}</span>${B('', { k: 'sm ghost', i: 'refresh', act: 'open', data: { path: b.path }, aria: 'Refresh' })}</div>
      ${b.error ? `<div class="m-content"><div class="banner attn">${ic('alert')}<p><b>${offline ? `Can't reach ${esc(name)}.` : 'That didn’t work.'}</b> ${esc(b.error)}${offline ? ' It may be asleep, shut down, or off the internet. Try again in a moment.' : ''}</p></div></div>` : ''}
      <div class="m-content">${table}</div>`,
    ).html + `<div class="m-status">${ic('shield')}<span>Read-only. Files stay on ${esc(name)}.</span><span class="grow"></span><span class="path-line">Saving to <span class="mono" title="${esc(S.download_dir)}">${esc(S.download_dir)}</span></span></div>`;
  }

  // W-19: "verified" only when the backend says the copy's hash was checked
  // against the one the other computer sent (always, for a finished copy).
  function verifiedBadge(/** @type {any} */ x) {
    if (x.verified === true) return `<span class="verified" title="SHA-256 ${esc(x.sha256)}">${ic('check')}SHA-256 verified</span>`;
    return `<span title="SHA-256 of this copy: ${esc(x.sha256)}">SHA-256 of this copy</span>`;
  }

  // W-17: files that could run code get Show in folder only.
  const RISKY = 'This file type can run code. Pigeon won’t open it; use Show in folder.';

  function xferRow(/** @type {any} */ x) {
    const route = x.route === 'relay' ? '<span class="route relay" title="Encrypted end to end; the relay can’t read it. Slower, not less safe.">Relay · slower</span>' : x.route === 'direct' ? '<span class="route">Direct</span>' : '';
    if (x.state === 'done') {
      return `<div class="xfer"><div class="fico">${ic('file')}</div>
        <div class="row wrap"><span class="h3 trunc">${esc(x.file_name)}</span><span class="small muted mono">${size(x.size)}</span></div>
        <div class="acts">${x.risky ? '' : B('Open', { k: 'sm', i: 'open', act: 'open-file', data: { id: x.id } })}${B('Show in folder', { k: 'sm', i: 'folder', act: 'show-file', data: { id: x.id } })}</div>
        <div class="meta">${x.risky ? `<span class="attn-text risky">${ic('shield')}${RISKY}</span>` : ''}${verifiedBadge(x)}${route}<span>from ${esc(x.computer_name)}</span><span>Done ${esc(when(x.finished_at))}</span>${x.resumed_from ? `<span>resumed at ${size(x.resumed_from)}</span>` : ''}
          ${x.file_name !== x.wanted_name ? `<span>Saved as a new copy: <span class="mono">${esc(x.wanted_name)}</span> already existed</span>` : ''}<span class="mono sha">${esc(String(x.sha256).slice(0, 16))}…</span></div></div>`;
    }
    if (x.state === 'failed') {
      return `<div class="xfer err"><div class="fico">${ic('alert')}</div>
        <div class="h3 trunc">${esc(x.file_name)}</div><div class="acts">${B('Resume', { k: 'sm', i: 'refresh', act: 'resume', data: { id: x.id } })}</div>
        <div class="meta"><span class="attn-text">Didn't finish.</span><span>${esc(x.error)}</span>${x.done ? `<span>Resumes from <span class="mono">${size(x.done)}</span>; nothing is downloaded twice.</span>` : ''}</div></div>`;
    }
    const pct = x.size ? Math.floor(x.done / x.size * 100) : 0;
    const eta = x.mb_per_sec > 0 && x.size ? Math.round((x.size - x.done) / 1e6 / x.mb_per_sec) : null;
    return `<div class="xfer"><div class="fico">${ic('file')}</div>
      <div class="row"><span class="h3 trunc">${esc(x.file_name)}</span><span class="small muted mono">${x.size ? `${pct}%` : ''}</span></div>
      <div class="acts"></div>
      <div class="bar wide"><i data-pct="${pct}"></i></div>
      <div class="meta wide">${x.size ? `<span class="mono">${size(x.done)} of ${size(x.size)} · ${x.mb_per_sec.toFixed(1)} MB/s${eta != null ? ` · ${eta >= 60 ? `${Math.round(eta / 60)} min` : `${eta} s`} left` : ''}</span>` : `<span class="row"><span class="spinner"></span>Connecting to ${esc(x.computer_name)}…</span>`}</div></div>`;
  }

  function transfers() {
    const active = S.transfers.filter((/** @type {any} */ x) => x.state !== 'done');
    const done = S.transfers.filter((/** @type {any} */ x) => x.state === 'done');
    return main(`<div class="m-head"><div class="grow"><div class="kicker">${ic('down')}Get</div><div class="h1 mt4">Transfers</div></div>${B('Clear finished', { k: 'sm ghost', act: 'clear' })}</div>
      <div class="m-content">
        ${active.length ? `<div class="label">In progress</div>${active.map(xferRow).join('')}` : ''}
        ${done.length ? `<div class="label mt20">Finished</div>${done.map(xferRow).join('')}` : ''}
        ${!S.transfers.length ? `<div class="list"><div class="empty">Nothing yet. Browse a computer and click Get.</div></div>` : ''}
      </div>`).html + `<div class="m-status">${ic('folder')}<span class="path-line">Saving to <span class="mono" title="${esc(S.download_dir)}">${esc(S.download_dir)}</span></span><span class="grow"></span><span>Never overwrites</span></div>`;
  }

  function settings() {
    const t = getTheme();
    return main(`<div class="m-head"><div class="h1">Settings</div></div>
      <div class="m-content"><div class="kv">
        <div class="k">This computer's name</div><div class="row"><input class="field narrow" id="myname" maxlength="64" value="${esc(S.me.name)}">${B('Rename', { k: 'sm', act: 'rename' })}</div>
        <div class="k">Fingerprint</div><div><span class="fp">${fp(S.me.fingerprint)}</span> <span class="small muted">· other devices show this for ${esc(S.me.name)}. One key for sharing and getting.</span></div>
        <div class="sep"></div>
        <div class="k">Download folder</div><div class="row wrap"><code class="mono small card code-path">${esc(S.download_dir)}</code></div>
        <div class="k"></div><div class="small muted">Pigeon never overwrites. If <span class="mono">report.pdf</span> is already there, the new copy is <span class="mono">report (1).pdf</span>.</div>
        <div class="sep"></div>
        <div class="k">Relay</div><div class="small">Default (n0) <span class="muted">· sees encrypted bytes only · ${S.online ? 'connected' : 'not connected'}</span></div>
        <div class="k">Appearance</div><div><div class="seg">${['system', 'light', 'dark'].map(v => `<button data-act="theme" data-v="${v}" aria-pressed="${t === v}">${v[0].toUpperCase() + v.slice(1)}</button>`).join('')}</div></div>
      </div></div>`);
  }

  // ---------- Render ----------
  const $ = (/** @type {string} */ s) => /** @type {HTMLElement} */ (document.querySelector(s));
  function screenHtml() {
    if (fatal) return main(`<div class="pair-card"><div class="big-check big-x">${ic('alert', 'lg')}</div><h1 class="h1">Pigeon couldn't start</h1><p class="muted lead">${esc(fatal)}</p></div>`, true);
    if (!S) return main('<div class="center-msg muted">Starting Pigeon…</div>', true);
    switch (st.screen) {
      case 'welcome': return welcome();
      case 'folders': return folders();
      case 'pair-choose': return pairChoose();
      case 'invite': return invite();
      case 'devices': return devicesScreen();
      case 'g-link': return gLink();
      case 'g-code': return gCode();
      case 'g-paired': return gPaired();
      case 'g-problem': return gProblem(st.getPair.problem);
      case 'browse': return browse();
      case 'transfers': return transfers();
      case 'settings': return settings();
      default: return devicesScreen();
    }
  }
  function render() {
    $('#side').innerHTML = sidebar();
    const r = screenHtml();
    const m = $('#main');
    // Browse and Transfers return `main(...).html` plus a status bar below it.
    if (typeof r === 'string') { m.innerHTML = r; m.className = 'main'; }
    else { m.innerHTML = r.html; m.className = r.center ? 'main center' : 'main'; }
    m.querySelectorAll("[data-pct]").forEach(el => { /** @type {HTMLElement} */ (el).style.width = `${/** @type {HTMLElement} */ (el).dataset.pct}%`; });
    renderOverlay();
    if (st.screen === 'g-code') wireCode();
  }
  function renderOverlay() {
    const d = st.dialog;
    $('#overlay').innerHTML = (d ? `<div class="dim" data-act="close-dialog"></div><div class="dialog" role="alertdialog" aria-label="${esc(d.title)}">${LOGO()}
      <div class="h3">${esc(d.title)}</div><p class="small mt8">${esc(d.body)}</p>
      <div class="btns">${B('Cancel', { act: 'close-dialog' })}${B(d.ok, { k: 'danger-fill', act: 'confirm-dialog' })}</div></div>` : '')
      + (st.toast ? `<div class="toast ${st.toastAttn ? 'attn' : ''}" role="status">${ic(st.toastAttn ? 'alert' : 'info')}<span>${esc(st.toast)}</span></div>` : '');
  }
  /** Screens that only show backend state are redrawn on every poll; screens with text fields are not. */
  const LIVE = new Set(['folders', 'devices', 'transfers']);
  function liveUpdate() {
    $('#side').innerHTML = sidebar();
    if (LIVE.has(st.screen) || (st.screen === 'invite' && S && S.invite)) render();
    else if (st.screen === 'browse') { const c = computer(st.browse.computer); const p = document.getElementById('online-pill'); if (c && p) p.innerHTML = pill(c.online); }
  }
  let toastTimer = 0;
  function toast(/** @type {string} */ msg, attn = false) {
    st.toast = msg; st.toastAttn = attn; renderOverlay();
    clearTimeout(toastTimer); toastTimer = window.setTimeout(() => { st.toast = ''; renderOverlay(); }, 4500);
  }
  const fail = (/** @type {any} */ e) => toast(String(e), true);

  function go(/** @type {string} */ s) {
    st.screen = s; render();
    const m = document.querySelector('#main .scroll'); if (m) m.scrollTop = 0;
  }

  async function refresh() {
    try { S = await invoke('status'); fatal = null; }
    catch (e) { if (!S) fatal = String(e); }
  }

  // ---------- Browse ----------
  async function openPath(/** @type {string} */ path) {
    const b = st.browse;
    b.path = path; b.loading = true; b.error = ''; b.entries = null; b.q = ''; b.sel = '';
    if (st.screen !== 'browse') st.screen = 'browse';
    render();
    const id = b.computer;
    try {
      const entries = await invoke('list_dir', { computer: id, path });
      if (b.computer !== id || b.path !== path) return;
      b.entries = entries;
      if (!path) b.roots = entries.filter((/** @type {any} */ e) => e.is_dir).map((/** @type {any} */ e) => e.name);
    } catch (e) {
      if (b.computer !== id) return;
      b.error = String(e);
    }
    b.loading = false;
    await refresh();
    render();
  }
  async function openComputer(/** @type {string} */ id) {
    st.browse = { computer: id, path: '', entries: null, roots: [], error: '', q: '', sel: '', loading: false };
    await openPath('');
  }

  // ---------- Code boxes (real EFF word list from the backend) ----------
  function wireCode() {
    const root = $('#wordin');
    const ins = /** @type {HTMLInputElement[]} */ ([...root.querySelectorAll('input')]);
    const code = st.getPair.code;
    ins.forEach((inp, i) => { inp.value = code[i] || ''; });
    const isWord = (/** @type {string} */ w) => WORDS.includes(w);
    const matches = (/** @type {string} */ v) => v.length < 2 ? [] : WORDS.filter(w => w.startsWith(v) && w !== v).slice(0, 4);
    const paint = (/** @type {number} */ i, /** @type {boolean} */ showList) => {
      const inp = ins[i], v = inp.value.trim().toLowerCase(), box = /** @type {HTMLElement} */ (inp.parentElement), m = matches(v);
      root.querySelectorAll('.sugg').forEach(n => n.remove());
      /** @type {HTMLElement} */ (box.querySelector('.ghost')).textContent = (m[0] && document.activeElement === inp) ? ' '.repeat(v.length) + m[0].slice(v.length) : '';
      box.classList.toggle('unknown', !!v && !isWord(v) && !m.length);
      if (showList && m.length && document.activeElement === inp) {
        const d = document.createElement('div'); d.className = 'sugg';
        for (const w of m) { const b = document.createElement('button'); b.type = 'button'; b.tabIndex = -1; b.dataset.word = w; b.textContent = w; d.appendChild(b); }
        d.addEventListener('mousedown', e => { const b = /** @type {HTMLElement} */ (e.target).closest('[data-word]'); if (!b) return; e.preventDefault(); accept(i, /** @type {string} */ (/** @type {HTMLElement} */ (b).dataset.word)); });
        box.appendChild(d);
      }
    };
    const accept = (/** @type {number} */ i, /** @type {string} */ w) => { ins[i].value = w; code[i] = w; paint(i, false); if (ins[i + 1]) ins[i + 1].focus(); else root.querySelectorAll('.sugg').forEach(n => n.remove()); };
    const fill = (/** @type {number} */ start, /** @type {string[]} */ words) => {
      words.forEach((w, k) => { if (ins[start + k]) { ins[start + k].value = w; code[start + k] = w; } });
      (ins[Math.min(3, start + words.length)] || ins[3]).focus(); ins.forEach((_, k) => paint(k, false));
    };
    ins.forEach((inp, i) => {
      inp.addEventListener('focus', () => paint(i, true));
      inp.addEventListener('blur', () => setTimeout(() => paint(i, false), 0));
      inp.addEventListener('input', () => {
        /** @type {HTMLElement} */ (inp.parentElement).classList.remove('bad');
        const raw = inp.value.toLowerCase();
        if (/[\s\-·.,]/.test(raw)) {
          const parts = raw.split(/[\s\-·.,]+/).filter(Boolean);
          if (parts.length > 1) { fill(i, parts); return; }
          if (parts[0]) { const m = matches(parts[0]); accept(i, isWord(parts[0]) ? parts[0] : (m[0] || parts[0])); return; }
          inp.value = ''; return;
        }
        code[i] = inp.value.trim().toLowerCase(); paint(i, true);
      });
      inp.addEventListener('paste', e => {
        const parts = ((e.clipboardData && e.clipboardData.getData('text')) || '').toLowerCase().split(/[^a-z]+/).filter(Boolean);
        if (parts.length > 1) { e.preventDefault(); fill(i, parts); }
      });
      inp.addEventListener('keydown', e => {
        const m = matches(inp.value.trim().toLowerCase());
        if (((e.key === 'Tab' && !e.shiftKey) || e.key === 'Enter') && m[0]) { e.preventDefault(); accept(i, m[0]); }
        else if (e.key === 'Enter') { e.preventDefault(); doPair(); }
        else if (e.key === 'Backspace' && !inp.value && ins[i - 1]) { e.preventDefault(); ins[i - 1].focus(); }
      });
    });
    const first = code.findIndex(w => !isWord(w));
    const el = ins[first >= 0 ? first : 3]; el.focus(); el.setSelectionRange(el.value.length, el.value.length);
  }

  async function doPair() {
    const g = st.getPair;
    if (g.busy) return;
    // The code goes from these boxes straight to the backend and is cleared here.
    const typed = g.code.map((/** @type {string} */ w) => w.trim().toLowerCase()).join('-');
    g.code = ['', '', '', ''];
    g.busy = true; render();
    try {
      const o = await invoke('pair_with', { link: g.link.trim(), code: typed, name: g.name.trim() || null });
      g.outcome = o; g.busy = false;
      if (o.result === 'paired') { await refresh(); go('g-paired'); }
      else if (o.result === 'wrong_code' || o.result === 'typo' || o.result === 'failed') {
        if (o.result === 'typo') g.code = typed.split('-');
        render();
      } else { g.problem = o.result; go('g-problem'); }
    } catch (e) { g.busy = false; g.outcome = { result: 'failed', message: String(e) }; render(); }
  }

  let linkTimer = 0;
  async function checkLink() {
    const g = st.getPair, link = g.link.trim();
    if (!link) { g.info = null; g.error = ''; render(); return; }
    try { g.info = await invoke('check_link', { link }); g.error = ''; }
    catch (e) { g.info = null; g.error = String(e); }
    const focus = document.activeElement && document.activeElement.id;
    render();
    const el = focus && document.getElementById(focus);
    if (el instanceof HTMLInputElement) { el.focus(); el.setSelectionRange(el.value.length, el.value.length); }
  }

  // ---------- Events ----------
  document.addEventListener('input', e => {
    const t = /** @type {HTMLInputElement} */ (e.target);
    if (t.id === 'lnk') { st.getPair.link = t.value; clearTimeout(linkTimer); linkTimer = window.setTimeout(checkLink, 250); }
    else if (t.id === 'cname') st.getPair.name = t.value;
    else if (t.id === 'dev-name') st.inviteName = t.value;
    else if (t.id === 'q') {
      st.browse.q = t.value; const pos = t.selectionStart; render();
      const q = /** @type {HTMLInputElement} */ (document.getElementById('q')); q.focus(); q.setSelectionRange(pos, pos);
    }
  });
  document.addEventListener('keydown', e => {
    const t = /** @type {HTMLElement} */ (e.target);
    if (e.key === 'Enter' && t.id === 'dev-name') act('make-invite', t);
    if (e.key === 'Enter' && t.id === 'myname') act('rename', t);
    if (e.key === 'Escape' && st.dialog) { st.dialog = null; renderOverlay(); }
  });
  document.addEventListener('click', e => {
    const t = /** @type {HTMLElement|null} */ (/** @type {HTMLElement} */ (e.target).closest('[data-go],[data-act]'));
    if (!t || /** @type {HTMLButtonElement} */ (t).disabled) return;
    if (t.dataset.go) { go(t.dataset.go); return; }
    e.stopPropagation();
    act(/** @type {string} */ (t.dataset.act), t);
  });

  async function act(/** @type {string} */ a, /** @type {HTMLElement} */ t) {
    const d = t.dataset;
    try {
      switch (a) {
        case 'welcome': try { localStorage.setItem('pigeon-welcomed', '1'); } catch (_) {}
          if (d.to === 'g-link') { act('new-get-pair', t); return; } go(/** @type {string} */ (d.to)); return;
        case 'add-folder': { const f = await invoke('add_folder'); if (f) toast(`Sharing ${f.name} (read-only)`); await refresh(); render(); return; }
        case 'rm-folder': st.dialog = { title: `Stop sharing ${d.name}?`, body: 'Paired devices can’t see it any more. A download in progress finishes. Nothing on this computer is deleted.', ok: 'Stop sharing', run: () => invoke('remove_folder', { name: d.name }) }; renderOverlay(); return;
        case 'revoke': st.dialog = { title: `Stop ${d.name} getting files from ${thisX()}?`, body: `It's disconnected now and can't get files until you pair again. A transfer in progress stops. Whether ${thisX()} can get files from it is separate.`, ok: 'Revoke', run: () => invoke('revoke_device', { id: d.id }) }; renderOverlay(); return;
        case 'forget': st.dialog = { title: `Forget ${d.name}?`, body: `${ThisX()} stops getting files from it. Files you already got stay in your download folder. Whether it can get files from ${thisX()} is separate.`, ok: 'Forget', run: () => invoke('forget_computer', { id: d.id }) }; renderOverlay(); return;
        case 'close-dialog': st.dialog = null; renderOverlay(); return;
        case 'confirm-dialog': { const run = st.dialog && st.dialog.run; st.dialog = null; renderOverlay(); if (run) await run(); await refresh(); if (st.screen === 'browse' && !computer(st.browse.computer)) st.screen = 'devices'; render(); return; }
        case 'make-invite': await invoke('create_invite', { deviceName: st.inviteName.trim() || null }); await refresh(); go('invite'); return;
        case 'cancel-invite': await invoke('cancel_invite'); await invoke('dismiss_invite'); await refresh(); toast('Invite cancelled. It can’t be used any more.'); go('invite'); return;
        case 'dismiss-invite': await invoke('dismiss_invite'); await refresh(); if (d.to === 'g-link') { act('new-get-pair', t); return; } go(/** @type {string} */ (d.to)); return;
        case 'copy-link': { const link = S && S.invite && S.invite.link; if (link) { await navigator.clipboard.writeText(link); const l = t.querySelector('.lbl'); if (l) l.textContent = 'Copied'; } return; }
        case 'new-get-pair': st.getPair = { link: '', info: null, name: '', error: '', outcome: null, busy: false, code: ['', '', '', ''] }; go('g-link'); return;
        case 'to-code': st.getPair.outcome = null; go('g-code'); return;
        case 'pair': doPair(); return;
        case 'browse': await openComputer(/** @type {string} */ (d.id)); return;
        case 'open': await openPath(d.path || ''); return;
        case 'sel': st.browse.sel = d.path; document.querySelectorAll('tr.sel').forEach(r => r.classList.remove('sel')); t.classList.add('sel'); return;
        case 'get': await invoke('get_file', { computer: st.browse.computer, path: d.path }); await refresh(); go('transfers'); return;
        case 'resume': await invoke('resume_transfer', { id: Number(d.id) }); await refresh(); render(); return;
        case 'clear': await invoke('clear_finished'); await refresh(); render(); return;
        case 'open-file': await invoke('open_file', { id: Number(d.id) }); return;
        case 'show-file': await invoke('show_in_folder', { id: Number(d.id) }); return;
        case 'rename': { const v = /** @type {HTMLInputElement} */ (document.getElementById('myname')).value; const n = await invoke('rename_computer', { name: v }); toast(`This computer is now called ${n}`); await refresh(); render(); return; }
        case 'theme': setTheme(/** @type {string} */ (d.v)); render(); return;
      }
    } catch (e) { fail(e); }
  }

  // ---------- Start ----------
  async function start() {
    try { WORDS = await invoke('wordlist'); } catch (_) { WORDS = []; }
    await refresh();
    let welcomed = false; try { welcomed = !!localStorage.getItem('pigeon-welcomed'); } catch (_) {}
    if (S) {
      const fresh = !S.folders.length && !S.computers.length && !S.devices.length;
      st.screen = fresh && !welcomed ? 'welcome' : S.invite ? 'invite' : S.computers.length ? 'devices' : S.folders.length ? 'folders' : 'devices';
      // Deep link for screenshots and tests: #screen (read-only navigation).
      const h = location.hash.slice(1);
      if (h) st.screen = h;
    }
    render();
    setInterval(async () => {
      if (document.hidden) return;
      const had = S && S.invite && S.invite.state;
      await refresh();
      const now = S && S.invite && S.invite.state;
      if (had === 'waiting' && now === 'paired') toast(`${S.invite.device.name} can now get files from ${thisX()}`);
      liveUpdate();
      const ex = document.getElementById('expires'); if (ex && S && S.invite && S.invite.expires_at) ex.textContent = `Expires in ${left(S.invite.expires_at)}`;
    }, 700);
  }
  start();
})();

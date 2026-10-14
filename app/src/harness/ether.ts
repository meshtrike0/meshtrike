// Simulated radio for the browser harness: every phone runs the real Rust
// core (WASM); the ether moves Bluetooth frames between phones that are in
// range, like simulator/src/lib.rs does for the headless simulator.

import { CoreClient, PickedImage, PlatformHooks, parseReply, qrToLuma } from '../core/client';
import { CoreEvent, QrMatrix } from '../core/types';
import { loadCore, WasmCore } from './wasm';

const STEP_MS = 50;
const DISCOVERY_MS = 500;
const CONNECT_MS = 100;

export interface Jammer {
  x: number;
  y: number;
  r: number;
}

type Cmd =
  | { cmd: 'advertise'; uuids: string[] }
  | { cmd: 'stop_advertise' }
  | { cmd: 'scan'; uuids: string[] }
  | { cmd: 'stop_scan' }
  | { cmd: 'connect'; peer: number }
  | { cmd: 'frame'; peer: number; bytes: number[] }
  | { cmd: 'disconnect'; peer: number }
  | { cmd: 'lora'; bytes: number[] }
  | { cmd: 'net_config'; enabled: boolean; config: string }
  | { cmd: 'net_send'; address: string; bytes: number[] };

export class Phone implements CoreClient {
  advertising: string[] = [];
  scanning: string[] = [];
  powered = true;
  /// Internet route: the core asked for it (Settings switch) / this phone has
  /// connectivity (harness "Net" toggle) / the route is up.
  netWanted = false;
  netConnectivity = true;
  netUp = false;
  /** Real-mixnet mode: this phone's socket to its local helper (mac_helper --port). */
  netWs: WebSocket | null = null;
  netConfigJson = '{}';
  framesSent = 0;
  bytesSent = 0;
  cameraTarget: number | null = null;
  screenQr: QrMatrix | null = null;
  private listeners = new Set<(e: CoreEvent) => void>();
  hooks: PlatformHooks;

  constructor(
    readonly ether: Ether,
    readonly id: number,
    public name: string,
    public x: number,
    public y: number,
    readonly core: WasmCore,
    public lora: boolean,
  ) {
    this.hooks = {
      platformLabel: 'Browser harness (simulated radio, software keystore)',
      captureCamera: async () => {
        const t = this.ether.phones.find((p) => p.id === this.cameraTarget);
        if (!t || !t.screenQr || !t.powered) return null;
        return qrToLuma(t.screenQr, 3, 4);
      },
      cameraTargets: () =>
        this.ether.phones.filter((p) => p.id !== this.id && p.screenQr).map((p) => ({ id: String(p.id), label: `${p.name}'s screen` })),
      pointCamera: (id) => {
        this.cameraTarget = id ? Number(id) : null;
        this.ether.changed();
      },
      showQr: (m) => {
        this.screenQr = m;
        this.ether.changed();
      },
      pickImage: pickImageFromBrowser,
      pickFile: pickFileFromBrowser,
      saveFile: async (name, bytes) => {
        const blob = new Blob([bytes.slice().buffer]);
        const a = document.createElement('a');
        a.href = URL.createObjectURL(blob);
        a.download = name;
        a.click();
        setTimeout(() => URL.revokeObjectURL(a.href), 1000);
      },
      copyText: (text) => {
        navigator.clipboard?.writeText(text).catch(() => {});
        // 60-second clipboard expiry (12.3).
        setTimeout(() => navigator.clipboard?.writeText('').catch(() => {}), 60_000);
      },
    };
  }

  async call<T>(method: string, args: Record<string, unknown> = {}): Promise<T> {
    // Yield so React can paint before slow calls (Argon2id, PoW).
    await new Promise((r) => setTimeout(r, 0));
    const reply = this.core.call(method, JSON.stringify(args));
    this.pumpEvents();
    return parseReply<T>(reply);
  }

  subscribe(fn: (e: CoreEvent) => void) {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }

  pumpEvents() {
    const evs = JSON.parse(this.core.drain_events()) as CoreEvent[];
    for (const e of evs) for (const l of this.listeners) l(e);
  }

  rawCall(method: string, args: Record<string, unknown> = {}) {
    return parseReply<unknown>(this.core.call(method, JSON.stringify(args)));
  }
}

export class Ether {
  phones: Phone[] = [];
  rangeM = 30;
  loraRangeM = 5000;
  latencyMs = 20;
  /** Simulated mixnet delay. */
  netLatencyMs = 900;
  private netInFlight: { at: number; to: number; bytes: Uint8Array }[] = [];
  /** A Nym gateway holds envelopes for its offline clients. */
  netMailbox = new Map<number, Uint8Array[]>();
  netStats = { sent: 0 };
  /**
   * Off: a simulated mixnet with a gateway mailbox. On: each phone talks to a
   * local helper running the real Nym client (plugins/link/native mac_helper
   * --port 47910+phone id), so messages cross the real mixnet.
   */
  realNet = false;
  loss = 0;
  jammers: Jammer[] = [];
  fastKdf = true;
  paused = false;
  private nextId = 1;
  private connected = new Set<string>();
  private pending: { a: number; b: number; at: number }[] = [];
  private inFlight: { at: number; from: number; to: number; bytes: Uint8Array }[] = [];
  private lastDiscovery = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private watchers = new Set<() => void>();
  private simNow = 0;
  stats = { frames: 0, bytes: 0 };

  onChange(fn: () => void) {
    this.watchers.add(fn);
    return () => {
      this.watchers.delete(fn);
    };
  }

  changed() {
    for (const w of this.watchers) w();
  }

  async addPhone(name: string, x: number, y: number, lora = false): Promise<Phone> {
    const b = await loadCore();
    const core = new b.WasmCore(JSON.stringify({ fast_kdf: this.fastKdf, lora }));
    const p = new Phone(this, this.nextId++, name, x, y, core, lora);
    this.phones.push(p);
    this.changed();
    return p;
  }

  removePhone(id: number) {
    const p = this.byId(id);
    if (!p) return;
    for (const o of this.phones) if (o.id !== id) this.breakLink(o.id, id);
    this.phones = this.phones.filter((q) => q.id !== id);
    p.core.free();
    this.changed();
  }

  byId(id: number) {
    return this.phones.find((p) => p.id === id);
  }

  start() {
    if (!this.timer) this.timer = setInterval(() => this.step(), STEP_MS);
  }

  stop() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  /** Fast-forward every phone's clock (delayed send, expiry, disappearing). */
  advance(ms: number) {
    for (const p of this.phones) p.core.advance_clock(ms);
    this.step();
  }

  private key(a: number, b: number) {
    return a < b ? `${a}-${b}` : `${b}-${a}`;
  }

  isConnected(a: number, b: number) {
    return this.connected.has(this.key(a, b));
  }

  links(): [number, number][] {
    return [...this.connected].map((k) => k.split('-').map(Number) as [number, number]);
  }

  dist(a: Phone, b: Phone) {
    return Math.hypot(a.x - b.x, a.y - b.y);
  }

  jammed(p: Phone) {
    return this.jammers.some((j) => Math.hypot(p.x - j.x, p.y - j.y) <= j.r);
  }

  inRange(a: Phone, b: Phone) {
    return a.powered && b.powered && this.dist(a, b) <= this.rangeM && !this.jammed(a) && !this.jammed(b);
  }

  private breakLink(a: number, b: number) {
    if (this.connected.delete(this.key(a, b))) {
      this.byId(a)?.rawCall('on_disconnected', { peer: b });
      this.byId(b)?.rawCall('on_disconnected', { peer: a });
    }
  }

  step() {
    if (this.paused) return;
    this.simNow += STEP_MS;
    const now = this.simNow;
    // Links out of range break.
    for (const k of [...this.connected]) {
      const [a, b] = k.split('-').map(Number);
      const pa = this.byId(a);
      const pb = this.byId(b);
      if (!pa || !pb || !this.inRange(pa, pb)) this.breakLink(a, b);
    }
    // Discovery.
    if (now - this.lastDiscovery >= DISCOVERY_MS) {
      this.lastDiscovery = now;
      for (const a of this.phones) {
        if (!a.scanning.length) continue;
        for (const b of this.phones) {
          if (a === b || !this.inRange(a, b) || this.isConnected(a.id, b.id)) continue;
          if (b.advertising.some((u) => a.scanning.includes(u))) {
            a.rawCall('on_peer_found', { peer: b.id, uuids: b.advertising, rssi: Math.round(-40 - this.dist(a, b)), transport: 'Ble' });
          }
        }
      }
    }
    // Connections.
    const ready = this.pending.filter((c) => c.at <= now);
    this.pending = this.pending.filter((c) => c.at > now);
    for (const c of ready) {
      const a = this.byId(c.a);
      const b = this.byId(c.b);
      if (!a || !b || this.isConnected(c.a, c.b)) continue;
      if (this.inRange(a, b) && b.advertising.length) {
        this.connected.add(this.key(c.a, c.b));
        b.rawCall('on_connected', { peer: a.id, mtu: 512, outgoing: false });
        a.rawCall('on_connected', { peer: b.id, mtu: 512, outgoing: true });
      } else {
        a.rawCall('on_disconnected', { peer: c.b });
      }
    }
    // Radio commands.
    for (const p of this.phones) {
      const cmds = JSON.parse(p.core.drain_commands()) as Cmd[];
      for (const c of cmds) this.exec(p, c, now);
    }
    // Deliver frames.
    const due = this.inFlight.filter((f) => f.at <= now);
    this.inFlight = this.inFlight.filter((f) => f.at > now);
    for (const f of due) {
      if (this.isConnected(f.from, f.to)) this.byId(f.to)?.core.on_frame(f.from, f.bytes);
    }
    // Simulated mixnet: deliver, or leave it with the recipient's gateway.
    const netDue = this.netInFlight.filter((f) => f.at <= now);
    this.netInFlight = this.netInFlight.filter((f) => f.at > now);
    for (const f of netDue) {
      const to = this.byId(f.to);
      if (!to) continue;
      if (to.netUp) safe(() => to.rawCall('on_internet_envelope', { bytes: b64(f.bytes) }));
      else this.netMailbox.set(f.to, [...(this.netMailbox.get(f.to) ?? []), f.bytes]);
    }
    for (const p of this.phones) {
      if (p.powered) p.core.tick();
      p.pumpEvents();
    }
    this.changed();
  }

  /** Real mixnet: connect (or drop) this phone's helper to match its switch and connectivity. */
  realNetRefresh(p: Phone) {
    const want = p.netWanted && p.netConnectivity;
    if (!want) {
      if (p.netWs) {
        p.netWs.onclose = null;
        p.netWs.close();
        p.netWs = null;
      }
      if (p.netUp) {
        p.netUp = false;
        p.core.set_net_online(false);
        safe(() => p.rawCall('on_internet_down', {}));
      }
      this.changed();
      return;
    }
    if (p.netWs) return;
    const ws = new WebSocket(`ws://127.0.0.1:${47910 + p.id}/`);
    p.netWs = ws;
    ws.onopen = () => ws.send(JSON.stringify({ t: 'config', enabled: true, ...JSON.parse(p.netConfigJson || '{}') }));
    ws.onmessage = (ev) => {
      const m = JSON.parse(String(ev.data)) as { t: string; address?: string; bytes?: string };
      if (m.t === 'ready' && m.address) {
        p.netUp = true;
        p.core.set_net_online(true);
        safe(() => p.rawCall('on_internet_ready', { address: m.address }));
      } else if (m.t === 'down') {
        if (p.netUp) {
          p.netUp = false;
          p.core.set_net_online(false);
          safe(() => p.rawCall('on_internet_down', {}));
        }
      } else if (m.t === 'env' && m.bytes) {
        safe(() => p.rawCall('on_internet_envelope', { bytes: m.bytes }));
      }
      this.changed();
    };
    ws.onclose = () => {
      if (p.netWs === ws) p.netWs = null;
      if (p.netUp) {
        p.netUp = false;
        p.core.set_net_online(false);
        safe(() => p.rawCall('on_internet_down', {}));
      }
      this.changed();
    };
  }

  /** Bring a phone's internet route up or down to match its switch and connectivity. */
  netRefresh(p: Phone) {
    if (this.realNet) return this.realNetRefresh(p);
    const up = p.netWanted && p.netConnectivity;
    if (up === p.netUp) return;
    p.netUp = up;
    p.core.set_net_online(up);
    if (up) {
      safe(() => p.rawCall('on_internet_ready', { address: `net${p.id}@gateway` }));
      for (const bytes of this.netMailbox.get(p.id) ?? []) safe(() => p.rawCall('on_internet_envelope', { bytes: b64(bytes) }));
      this.netMailbox.delete(p.id);
    } else {
      safe(() => p.rawCall('on_internet_down', {}));
    }
    this.changed();
  }

  private exec(p: Phone, c: Cmd, now: number) {
    switch (c.cmd) {
      case 'advertise':
        p.advertising = c.uuids;
        break;
      case 'stop_advertise':
        p.advertising = [];
        break;
      case 'scan':
        p.scanning = c.uuids;
        break;
      case 'stop_scan':
        p.scanning = [];
        break;
      case 'connect':
        this.pending.push({ a: p.id, b: c.peer, at: now + CONNECT_MS });
        break;
      case 'frame': {
        p.framesSent++;
        p.bytesSent += c.bytes.length;
        this.stats.frames++;
        this.stats.bytes += c.bytes.length;
        if (this.isConnected(p.id, c.peer) && !(this.loss > 0 && Math.random() < this.loss)) {
          this.inFlight.push({ at: now + this.latencyMs, from: p.id, to: c.peer, bytes: Uint8Array.from(c.bytes) });
        }
        break;
      }
      case 'disconnect':
        if (this.connected.delete(this.key(p.id, c.peer))) this.byId(c.peer)?.rawCall('on_disconnected', { peer: p.id });
        break;
      case 'net_config': {
        // Like the real add-on: no bridge and no permission to go undisguised
        // means it does not connect at all.
        let ok = false;
        try {
          const cfg = JSON.parse(c.config) as { bridges?: string[]; allow_direct?: boolean };
          ok = (cfg.bridges?.length ?? 0) > 0 || !!cfg.allow_direct;
        } catch {
          ok = false;
        }
        p.netWanted = c.enabled && ok;
        p.netConfigJson = c.config;
        if (this.realNet) this.realNetRefresh(p);
        else this.netRefresh(p);
        break;
      }
      case 'net_send': {
        if (this.realNet) {
          if (p.netWs?.readyState === WebSocket.OPEN) {
            this.netStats.sent++;
            p.netWs.send(JSON.stringify({ t: 'send', address: c.address, bytes: b64(Uint8Array.from(c.bytes)) }));
          }
          break;
        }
        const to = /^net(\d+)@gateway$/.exec(c.address);
        if (to && p.netUp) {
          this.netStats.sent++;
          this.netInFlight.push({ at: now + this.netLatencyMs, to: Number(to[1]), bytes: Uint8Array.from(c.bytes) });
        }
        break;
      }
      case 'lora':
        for (const o of this.phones) {
          if (o !== p && o.lora && o.powered && this.dist(p, o) <= this.loraRangeM) {
            o.rawCall('on_lora_packet', { bytes: btoa(String.fromCharCode(...c.bytes)) });
          }
        }
        break;
    }
  }
}

function safe(f: () => unknown) {
  try {
    f();
  } catch {
    /* a refused call (e.g. route switched off meanwhile) is not fatal to the ether */
  }
}

function b64(bytes: Uint8Array): string {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

async function pickImageFromBrowser(): Promise<PickedImage | null> {
  const file = await chooseFile('image/*');
  if (!file) return null;
  // Sender side may use the OS/browser decoder (12.3); the core re-encodes.
  const bmp = await createImageBitmap(file);
  const canvas = document.createElement('canvas');
  canvas.width = bmp.width;
  canvas.height = bmp.height;
  const ctx = canvas.getContext('2d')!;
  ctx.drawImage(bmp, 0, 0);
  const d = ctx.getImageData(0, 0, bmp.width, bmp.height);
  return { kind: 'rgba', width: bmp.width, height: bmp.height, rgba: new Uint8Array(d.data.buffer) };
}

async function pickFileFromBrowser(): Promise<{ name: string; bytes: Uint8Array } | null> {
  const file = await chooseFile('*/*');
  if (!file) return null;
  return { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) };
}

function chooseFile(accept: string): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.onchange = () => resolve(input.files?.[0] ?? null);
    input.oncancel = () => resolve(null);
    input.click();
  });
}

// Mac node: one phone UI in the browser, running the real Rust core (WASM),
// talking to a real Android phone over the Mac's Bluetooth via Web Bluetooth.
// The browser can only be a BLE central, so the Mac connects to the phone
// (which advertises) and exchanges frames over the phone's GATT data service.

import React, { useEffect, useMemo, useRef, useState } from 'react';
import { CoreClient, PickedImage, PlatformHooks, parseReply } from '../core/client';
import { CoreEvent } from '../core/types';
import { MeshApp } from '../ui/MeshApp';
import { loadCore, WasmCore } from '../harness/wasm';

const PUBLIC_UUID = '6e4d0001-b5a3-4f39-9c2e-0d5a9f1c7b21';
const DATA_SERVICE = '6e4d0002-b5a3-4f39-9c2e-0d5a9f1c7b21';
const RX = '6e4d0003-b5a3-4f39-9c2e-0d5a9f1c7b21'; // we write
const TX = '6e4d0004-b5a3-4f39-9c2e-0d5a9f1c7b21'; // phone notifies
// macOS negotiates a large ATT MTU; stay below common limits.
const MTU = 185;

type Status = { text: string; tone: 'idle' | 'ok' | 'busy' | 'error' };

interface BtLink {
  handle: number;
  device: any;
  rx: any;
  queue: Promise<unknown>;
  /** When the link came up, to spot links that drop straight away. */
  upAt: number;
}

class MacClient implements CoreClient {
  private listeners = new Set<(e: CoreEvent) => void>();
  links = new Map<number, BtLink>();
  private nextHandle = 1;
  /** Links in a row that dropped within seconds of coming up. */
  private quickDrops = 0;
  hooks: PlatformHooks;
  onStatus: (s: Status) => void = () => {};

  constructor(readonly core: WasmCore) {
    this.hooks = {
      platformLabel: 'Mac browser node (Web Bluetooth, software keystore, in-memory storage)',
      captureCamera: captureWebcam,
      pickImage: pickImage,
      pickFile: async () => {
        const f = await chooseFile('*/*');
        return f ? { name: f.name, bytes: new Uint8Array(await f.arrayBuffer()) } : null;
      },
      saveFile: async (name, bytes) => {
        const a = document.createElement('a');
        a.href = URL.createObjectURL(new Blob([bytes.slice().buffer]));
        a.download = name;
        a.click();
      },
      copyText: (t) => {
        navigator.clipboard?.writeText(t).catch(() => {});
        setTimeout(() => navigator.clipboard?.writeText('').catch(() => {}), 60_000);
      },
    };
    setInterval(() => this.pump(), 100);
  }

  async call<T>(method: string, args: Record<string, unknown> = {}): Promise<T> {
    await new Promise((r) => setTimeout(r, 0));
    const reply = this.core.call(method, JSON.stringify(args));
    this.pump(false);
    return parseReply<T>(reply);
  }

  subscribe(fn: (e: CoreEvent) => void) {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }

  /** Tick, then run radio commands and deliver UI events. */
  pump(tick = true) {
    if (tick) this.core.tick();
    const cmds = JSON.parse(this.core.drain_commands()) as any[];
    for (const c of cmds) {
      if (c.cmd === 'frame') this.write(c.peer, Uint8Array.from(c.bytes));
      else if (c.cmd === 'disconnect') {
        const l = this.links.get(c.peer);
        if (l) console.info('[mac node] core closed link', c.peer, '-', this.lastLog());
        l?.device.gatt?.disconnect();
      }
      else if (c.cmd === 'net_config') this.netConfig(c.enabled, c.config);
      else if (c.cmd === 'net_send') this.netSend(c.address, Uint8Array.from(c.bytes));
      // advertise / scan / connect: the browser cannot advertise or scan in the
      // background; connections start from the "Connect" button.
    }
    const evs = JSON.parse(this.core.drain_events()) as CoreEvent[];
    for (const e of evs) for (const l of this.listeners) l(e);
  }

  // ===== Internet route (test helper: plugins/internet/native mac_helper) =====

  private ws: WebSocket | null = null;

  /** The core wants the route on or off; the local helper runs the real Nym client. */
  private netConfig(enabled: boolean, config: string) {
    let cfg: Record<string, unknown> = {};
    try {
      cfg = JSON.parse(config);
    } catch {
      /* keep empty */
    }
    const msg = JSON.stringify({ t: 'config', enabled, ...cfg });
    if (!enabled) {
      if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(msg);
      this.netDown();
      return;
    }
    if (this.ws && this.ws.readyState <= WebSocket.OPEN) {
      if (this.ws.readyState === WebSocket.OPEN) this.ws.send(msg);
      return;
    }
    const ws = new WebSocket('ws://127.0.0.1:47900/');
    this.ws = ws;
    ws.onopen = () => ws.send(msg);
    ws.onmessage = (ev) => {
      const m = JSON.parse(String(ev.data)) as { t: string; address?: string; bytes?: string };
      if (m.t === 'ready' && m.address) {
        this.core.set_net_online(true);
        this.core.call('on_internet_ready', JSON.stringify({ address: m.address }));
      } else if (m.t === 'down') {
        this.netDown();
      } else if (m.t === 'env' && m.bytes) {
        this.core.call('on_internet_envelope', JSON.stringify({ bytes: m.bytes }));
      }
      this.pump(false);
    };
    ws.onclose = () => {
      if (this.ws === ws) this.ws = null;
      this.netDown();
    };
  }

  private netDown() {
    this.core.set_net_online(false);
    this.core.call('on_internet_down', '{}');
  }

  private netSend(address: string, bytes: Uint8Array) {
    if (this.ws?.readyState !== WebSocket.OPEN) return;
    let b = '';
    for (let i = 0; i < bytes.length; i += 0x8000) b += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    this.ws.send(JSON.stringify({ t: 'send', address, bytes: btoa(b) }));
  }

  private write(handle: number, bytes: Uint8Array) {
    const link = this.links.get(handle);
    if (!link) return;
    // GATT operations must not overlap: chain them. Acknowledged writes only:
    // macOS can silently drop "without response" writes under load, and one
    // lost frame breaks the link's Noise stream (the phone then sees
    // "noise decrypt failed" and drops the link). A write that still fails
    // closes the link so both sides start a fresh handshake.
    link.queue = link.queue
      .then(() => link.rx.writeValueWithResponse(bytes))
      .catch((e: any) => {
        // Some macOS/Chrome builds refuse acknowledged writes: say so, and fall
        // back to an unacknowledged one (a lost frame then breaks the link,
        // and both sides start again).
        this.onStatus({ text: `Bluetooth write failed (${e?.name ?? 'error'}: ${e?.message ?? e}); retrying without acknowledgement`, tone: 'busy' });
        return link.rx.writeValueWithoutResponse(bytes).catch((e2: any) => {
          this.onStatus({ text: `Bluetooth write failed again (${e2?.message ?? e2}); reconnecting`, tone: 'error' });
          link.device.gatt?.disconnect();
        });
      });
  }

  /**
   * Retry the same device with backoff. The phone rotates its Bluetooth
   * address for privacy (every 15 min); after that only a new "Connect"
   * (device chooser, needs a click) can find it again.
   */
  private reconnect(device: any, attempt: number) {
    if (attempt >= 6) {
      this.onStatus({ text: 'Lost the phone. It may have changed its Bluetooth address (privacy rotation). Click "Connect to phone" again.', tone: 'error' });
      return;
    }
    this.onStatus({ text: `Disconnected. Reconnecting (try ${attempt + 1} of 6)…`, tone: 'busy' });
    setTimeout(() => this.attach(device).catch(() => this.reconnect(device, attempt + 1)), 1500 * (attempt + 1));
  }

  /** Last line of the core's diagnostic log (no identifiers). */
  private lastLog(): string {
    try {
      const t = parseReply<string>(this.core.call('debug_log', '{}')) ?? '';
      const lines = t.trim().split('\n');
      return lines[lines.length - 1] ?? '';
    } catch {
      return '';
    }
  }

  /** Must run from a user gesture (Web Bluetooth device chooser). */
  async connect() {
    const bt = (navigator as any).bluetooth;
    if (!bt) throw new Error('This browser has no Web Bluetooth. Use Chrome or Edge on macOS.');
    // Without an identity the core refuses every link at once, and the page
    // would connect and drop in a loop.
    if (!parseReply<boolean>(this.core.call('has_identity', '{}'))) {
      throw new Error("Create this Mac's identity first (in the phone frame on the right), then click Connect.");
    }
    this.quickDrops = 0;
    this.onStatus({ text: 'Choose your phone in the Bluetooth dialog…', tone: 'busy' });
    const device = await bt.requestDevice({ filters: [{ services: [PUBLIC_UUID] }], optionalServices: [DATA_SERVICE] });
    await this.attach(device);
  }

  async attach(device: any) {
    this.onStatus({ text: `Connecting to ${device.name ?? 'phone'}…`, tone: 'busy' });
    // A phone that rotated its address or went to the background never answers;
    // fail with a reason instead of "Connecting…" forever.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const server = await Promise.race([
      device.gatt.connect(),
      new Promise((_, reject) => {
        timer = setTimeout(() => {
          device.gatt?.disconnect?.();
          reject(new Error('The phone did not answer. Keep Meshtrike open and unlocked on the phone, then click Connect again.'));
        }, 20_000);
      }),
    ]).finally(() => clearTimeout(timer));
    const svc = await server.getPrimaryService(DATA_SERVICE);
    const rx = await svc.getCharacteristic(RX);
    const tx = await svc.getCharacteristic(TX);
    const handle = this.nextHandle++;
    const link: BtLink = { handle, device, rx, queue: Promise.resolve(), upAt: Date.now() };
    this.links.set(handle, link);
    tx.addEventListener('characteristicvaluechanged', (ev: any) => {
      const v: DataView = ev.target.value;
      this.core.on_frame(handle, new Uint8Array(v.buffer, v.byteOffset, v.byteLength).slice());
      this.pump(false);
    });
    device.addEventListener(
      'gattserverdisconnected',
      () => {
        this.links.delete(handle);
        const why = this.lastLog();
        this.core.call('on_disconnected', JSON.stringify({ peer: handle }));
        this.pump(false);
        // A link that keeps dropping right after it comes up will not heal by
        // retrying: stop and say what the core saw last.
        this.quickDrops = Date.now() - link.upAt < 3_000 ? this.quickDrops + 1 : 0;
        if (this.quickDrops >= 3) {
          this.onStatus({ text: `The link drops right after connecting, so retrying stopped. Last core message: ${why || 'none'}`, tone: 'error' });
          return;
        }
        this.reconnect(device, 0);
      },
      { once: true },
    );
    await tx.startNotifications(); // phone sees the subscription and starts its side
    this.core.call('on_peer_found', JSON.stringify({ peer: handle, uuids: [PUBLIC_UUID.replace(/-/g, '')], rssi: -50, transport: 'Ble' }));
    this.core.call('on_connected', JSON.stringify({ peer: handle, mtu: MTU, outgoing: true }));
    this.pump(false);
    this.onStatus({ text: `Connected to ${device.name ?? 'phone'} over Bluetooth (encrypted link)`, tone: 'ok' });
  }
}

export function MacNode() {
  const [client, setClient] = useState<MacClient | null>(null);
  const [status, setStatus] = useState<Status>({ text: 'Not connected', tone: 'idle' });
  const [error, setError] = useState<string | null>(null);
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    document.title = 'Meshtrike · Mac node';
    document.body.style.overflow = 'auto';
    document.body.style.background = '#E9E6DF';
    loadCore()
      .then((b) => {
        const c = new MacClient(new b.WasmCore(JSON.stringify({ fast_kdf: false })));
        c.onStatus = setStatus;
        setClient(c);
      })
      .catch((e) => setError(String(e)));
  }, []);

  const hasBt = useMemo(() => typeof navigator !== 'undefined' && !!(navigator as any).bluetooth, []);
  const color = { idle: '#5D616B', ok: '#0B6E4A', busy: '#7A5200', error: '#A3360A' }[status.tone];

  return (
    <div style={{ minHeight: '100vh', background: '#E9E6DF', color: '#15171C', fontFamily: 'Geist_500Medium, system-ui, -apple-system, sans-serif', display: 'flex', gap: 32, padding: 24, flexWrap: 'wrap' }}>
      <div style={{ maxWidth: 380, display: 'flex', flexDirection: 'column', gap: 12 }}>
        <h2 style={{ margin: 0 }}>Meshtrike · Mac node</h2>
        <p style={{ color: '#5D616B', margin: 0, lineHeight: 1.5 }}>
          This Mac runs the same Rust core as the phone and talks to it over the Mac's own Bluetooth. Nothing goes over the internet unless you turn on its internet route (needs the local test helper).
        </p>
        <ol style={{ color: '#41454D', lineHeight: 1.6, paddingLeft: 18, margin: 0 }}>
          <li>On the phone: open Meshtrike and create your identity.</li>
          <li>Here: create an identity in the phone frame on the right.</li>
          <li>Click <b>Connect to phone</b> and pick your phone.</li>
          <li>Pair: on the phone open <i>New chat → Add contact → Scan code</i>; here open <i>New chat → Add contact</i> (Show my code). Point the phone at this screen.</li>
          <li>Chat.</li>
        </ol>
        <button
          disabled={!client || !hasBt}
          onClick={() => client?.connect().catch((e: any) => setStatus({ text: e?.message ?? String(e), tone: 'error' }))}
          style={{ background: '#15171C', color: '#fff', border: 0, borderRadius: 99, padding: '14px 22px', fontWeight: 600, fontSize: 15, cursor: 'pointer', opacity: client && hasBt ? 1 : 0.5 }}
        >
          Connect to phone (Bluetooth)
        </button>
        <div style={{ color, fontSize: 14 }} data-testid="bt-status">
          ● {status.text}
        </div>
        {!hasBt ? <div style={{ color: '#A3360A', fontSize: 14 }}>This browser has no Web Bluetooth. Open this page in Chrome or Edge on macOS.</div> : null}
        {error ? <div style={{ color: '#A3360A' }}>Core failed to load: {error}</div> : null}
        <p style={{ color: '#7A766D', fontSize: 12, margin: 0 }}>
          Browser limits: this page cannot advertise or scan in the background, so it connects to the phone; keys live in memory, so reloading the page resets this Mac's identity.
        </p>
      </div>
      <div style={{ width: 372, height: 772, background: '#15171C', borderRadius: 50, padding: 6, boxShadow: '0 40px 80px -30px rgba(40,36,26,.45)' }}>
        <div style={{ width: 360, height: 760, borderRadius: 44, overflow: 'hidden', display: 'flex', flexDirection: 'column', background: '#F6F5F1' }}>
          <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>{client ? <MeshApp client={client} /> : null}</div>
        </div>
      </div>
    </div>
  );
}

// ===== Webcam QR capture (frames decoded by the Rust core) =====

let video: HTMLVideoElement | null = null;

async function captureWebcam(): Promise<{ width: number; height: number; luma: Uint8Array } | null> {
  if (!video) {
    const stream = await navigator.mediaDevices.getUserMedia({ video: { width: 1280, height: 720 } });
    video = document.createElement('video');
    video.srcObject = stream;
    video.muted = true;
    video.playsInline = true;
    await video.play();
  }
  const w = video.videoWidth;
  const h = video.videoHeight;
  if (!w || !h) return null;
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext('2d', { willReadFrequently: true })!;
  ctx.drawImage(video, 0, 0);
  const px = ctx.getImageData(0, 0, w, h).data;
  const luma = new Uint8Array(w * h);
  for (let i = 0, j = 0; i < px.length; i += 4, j++) luma[j] = (px[i] * 77 + px[i + 1] * 150 + px[i + 2] * 29) >> 8;
  return { width: w, height: h, luma };
}

async function pickImage(): Promise<PickedImage | null> {
  const file = await chooseFile('image/*');
  if (!file) return null;
  const bmp = await createImageBitmap(file);
  const c = document.createElement('canvas');
  c.width = bmp.width;
  c.height = bmp.height;
  const ctx = c.getContext('2d')!;
  ctx.drawImage(bmp, 0, 0);
  const d = ctx.getImageData(0, 0, bmp.width, bmp.height);
  return { kind: 'rgba', width: bmp.width, height: bmp.height, rgba: new Uint8Array(d.data.buffer) };
}

function chooseFile(accept: string): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.onchange = () => resolve(input.files?.[0] ?? null);
    input.click();
  });
}

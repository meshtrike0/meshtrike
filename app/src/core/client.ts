import { CoreError, CoreEvent, QrMatrix } from './types';

/** What the UI may ask the platform beyond the core (camera, files). */
export interface PlatformHooks {
  /** Capture one camera frame as 8-bit luma, or null if nothing is in view. */
  captureCamera?: () => Promise<{ width: number; height: number; luma: Uint8Array } | null>;
  /** Names of things the camera can be pointed at (harness only). */
  cameraTargets?: () => { id: string; label: string }[];
  pointCamera?: (targetId: string | null) => void;
  /** Report the QR currently shown full-screen on this phone (harness camera source). */
  showQr?: (m: QrMatrix | null) => void;
  /**
   * Ask the user for a photo. Sender-side decoding may use the OS (12.3):
   * either raw RGBA pixels or a baseline JPEG ≤ 1,600 px that the core
   * decodes and re-encodes in Rust.
   */
  pickImage?: () => Promise<PickedImage | null>;
  pickFile?: () => Promise<{ name: string; bytes: Uint8Array } | null>;
  /**
   * Android: pick a file without reading it in JS, then let the platform read
   * and send it (large files would freeze the screen if read here).
   */
  pickFileRef?: () => Promise<{ name: string; size: number; uri: string } | null>;
  sendFileRef?: (conversation: string, file: { name: string; uri: string }, options: Record<string, unknown>) => Promise<void>;
  /** Nearby calls (Android): audio engine and tones. Absent on the web harness. */
  call?: {
    audio: (on: boolean, wifi: boolean) => Promise<void>;
    mute: (on: boolean) => Promise<void>;
    speaker: (on: boolean) => Promise<void>;
    ringback: (on: boolean) => Promise<void>;
    askMic: () => Promise<boolean>;
    micGranted: () => boolean;
    /** The phone's own lock screen is up (the call rang over it). */
    deviceLocked: () => boolean;
    /** Ask the phone to unlock (fingerprint, face or PIN): true once it is. */
    unlockDevice: () => Promise<boolean>;
  };
  /** Android settings page for Meshtrike (to turn a permission back on). */
  openAppSettings?: () => void;
  /** Videos are never played in the app: length for the bubble, and hand-off to a player. */
  video?: {
    info: (message: string) => Promise<{ durationMs: number; width: number; height: number }>;
    open: (message: string) => Promise<void>;
  };
  /** Voice notes (Android): record, send, play. Absent on the web harness. */
  voice?: VoiceHooks;
  /** Save / share a photo message (a fresh JPEG from its decoded pixels). */
  savePhoto?: (message: string) => Promise<void>;
  sharePhoto?: (message: string) => Promise<void>;
  saveFile?: (name: string, bytes: Uint8Array) => Promise<void>;
  setSecureWindow?: (on: boolean) => void;
  copyText?: (text: string) => void;
  /** Android: hardware key attestation chain (base64 DER, leaf first). */
  attestationChain?: () => Promise<string[]>;
  /** Meshtastic radio pairing (Android). */
  lora?: {
    scan: () => Promise<{ address: string; name: string }[]>;
    connect: (address: string) => Promise<boolean>;
    forget: () => Promise<boolean>;
    status: () => { connected: boolean; name: string | null };
  };
  platformLabel: string;
}

export type VoicePlaybackEvent = { message: string | null; positionMs: number; durationMs: number; playing: boolean; ended: boolean };

export interface VoiceHooks {
  micGranted: () => boolean;
  /** Refused before and Android will not ask again (only settings can turn it on). */
  micBlocked: () => boolean;
  askMic: () => Promise<boolean>;
  start: () => Promise<void>;
  /** Pause (true) or resume the recording; resolves to its length so far. */
  pauseRecording: (on: boolean) => Promise<number>;
  stop: () => Promise<{ durationMs: number; waveform: number[]; ok: boolean }>;
  cancel: () => Promise<void>;
  send: (conversation: string, options: Record<string, unknown>, replyTo?: string | null) => Promise<void>;
  play: (message: string, fromMs: number, speed: number) => Promise<void>;
  pause: () => Promise<void>;
  resume: () => Promise<void>;
  seek: (ms: number) => Promise<void>;
  setSpeed: (speed: number) => Promise<void>;
  stopPlayback: () => Promise<void>;
  onLevel: (fn: (e: { level: number; elapsedMs: number }) => void) => () => void;
  onLimit: (fn: () => void) => () => void;
  onPlayback: (fn: (e: VoicePlaybackEvent) => void) => () => void;
}

export type PickedImage = { kind: 'rgba'; width: number; height: number; rgba: Uint8Array } | { kind: 'jpeg'; base64: string };

export interface CoreClient {
  call<T = unknown>(method: string, args?: Record<string, unknown>): Promise<T>;
  subscribe(fn: (e: CoreEvent) => void): () => void;
  hooks: PlatformHooks;
}

export function parseReply<T>(json: string): T {
  const r = JSON.parse(json) as { ok?: T; err?: any };
  if ('err' in r && r.err !== undefined) throw new CoreError(r.err);
  return r.ok as T;
}

export function b64encode(bytes: Uint8Array): string {
  let s = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    s += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + chunk)));
  }
  return btoa(s);
}

export function b64decode(s: string): Uint8Array {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Render a QR matrix to grayscale luma (white background, black modules). */
export function qrToLuma(m: QrMatrix, scale = 4, quiet = 4): { width: number; height: number; luma: Uint8Array } {
  const modules = b64decode(m.modules);
  const side = (m.size + quiet * 2) * scale;
  const luma = new Uint8Array(side * side).fill(255);
  for (let y = 0; y < m.size; y++) {
    for (let x = 0; x < m.size; x++) {
      if (!modules[y * m.size + x]) continue;
      for (let dy = 0; dy < scale; dy++) {
        const row = ((y + quiet) * scale + dy) * side;
        for (let dx = 0; dx < scale; dx++) luma[row + (x + quiet) * scale + dx] = 0;
      }
    }
  }
  return { width: side, height: side, luma };
}

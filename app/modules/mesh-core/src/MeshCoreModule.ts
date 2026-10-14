import { NativeModule, requireNativeModule } from 'expo';

export type VoicePlayback = { message: string | null; positionMs: number; durationMs: number; playing: boolean; ended: boolean };

type Events = {
  onCoreEvent: (e: { json: string }) => void;
  onVoiceLevel: (e: { level: number; elapsedMs: number }) => void;
  onVoicePlayback: (e: VoicePlayback) => void;
  onVoiceLimit: (e: { elapsedMs: number }) => void;
};

declare class MeshCoreNative extends NativeModule<Events> {
  start(): Promise<boolean>;
  call(method: string, argsJson: string): Promise<string>;
  /** Read a picked file natively and send it (core reply JSON). */
  sendFileUri(conversation: string, uri: string, name: string, optionsJson: string): Promise<string>;
  /** Save a photo message to the phone's Photos as a fresh JPEG. */
  savePhoto(message: string): Promise<boolean>;
  micPermission(): Promise<{ granted: boolean }>;
  micGranted(): boolean;
  voiceStart(): Promise<boolean>;
  voiceStop(): Promise<{ durationMs: number; waveform: number[]; ok: boolean }>;
  voiceCancel(): Promise<boolean>;
  /** Pause (true) or resume the recording; returns its length so far. */
  voicePause(on: boolean): Promise<number>;
  /** Send the kept recording (core reply JSON), optionally as a reply. */
  voiceSend(conversation: string, optionsJson: string, replyTo: string | null): Promise<string>;
  /** Play a voice message id, or "draft" for the unsent recording. */
  voicePlay(message: string, fromMs: number, speed: number): Promise<boolean>;
  voicePausePlayback(): Promise<boolean>;
  voiceResume(): Promise<boolean>;
  voiceSeek(ms: number): Promise<boolean>;
  voiceSpeed(speed: number): Promise<boolean>;
  voiceStopPlayback(): Promise<boolean>;
  /** Nearby calls: audio on/off (Opus over the link), mute, speaker, tones. */
  callAudio(on: boolean, wifi: boolean): Promise<boolean>;
  callMute(on: boolean): Promise<boolean>;
  callSpeaker(on: boolean): Promise<boolean>;
  callRing(kind: 'outgoing' | 'none'): Promise<boolean>;
  callNotify(show: boolean): Promise<boolean>;
  /** The phone's lock screen is up; ask the phone to unlock (true once it is). */
  deviceLocked(): boolean;
  unlockDevice(): Promise<boolean>;
  /** Meshtrike's page in Android settings. */
  openAppSettings(): boolean;
  /** The microphone was refused and Android will not ask again. */
  micBlocked(): boolean;
  /** A received video's length and size (read by Android's media service). */
  videoInfo(message: string): Promise<{ durationMs: number; width: number; height: number }>;
  /** Hand a video to the user's chosen player (a short-lived copy). */
  openVideo(message: string): Promise<boolean>;
  /** Offer a photo message to Android's share sheet as a fresh JPEG. */
  sharePhoto(message: string): Promise<boolean>;
  requestPermissions(): Promise<{ granted: boolean }>;
  /** Android permission name → granted, read without prompting. */
  permissionStatus(): Record<string, boolean>;
  bluetoothEnabled(): boolean;
  /** The Meshtrike Link add-on's Wi-Fi Direct side, and what it reports. */
  wifiLinkStatus(): { installed: boolean; hardware: boolean; wifiOn: boolean; permitted: boolean };
  /** Opens the add-on's screen, which asks for its Wi-Fi permission. */
  openWifiLink(): boolean;
  /** The Meshtrike Link add-on (internet route: Nym mixnet behind a REALITY bridge) is installed with our signature. */
  internetAddonStatus(): { installed: boolean; mode: 'bridge' | 'direct' | null; version: string | null };
  /** The "Get Meshtrike Reach" prompt was already offered once. */
  reachPromptSeen(): boolean;
  /** SHA-256 of an installed app's signing certificate (AB:CD:…), or null. */
  signingFingerprint(pkg: string): string | null;
  setReachPromptSeen(): boolean;
  copySensitive(text: string): void;
  attestationChain(): Promise<string[]>;
  loraScan(): Promise<{ address: string; name: string }[]>;
  loraConnect(address: string): Promise<boolean>;
  loraForget(): Promise<boolean>;
  loraStatus(): { connected: boolean; name: string | null };
}

/** Loaded lazily so the web bundle never touches the native module. */
export function meshCoreNative(): MeshCoreNative {
  return requireNativeModule<MeshCoreNative>('MeshCore');
}

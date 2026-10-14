import { Directory, File } from 'expo-file-system';
import { ImageManipulator, SaveFormat } from 'expo-image-manipulator';
import * as ImagePicker from 'expo-image-picker';
import { meshCoreNative } from '../../modules/mesh-core/src/MeshCoreModule';
import { CoreClient, parseReply } from '../core/client';
import { CoreEvent } from '../core/types';

/** Android: the Rust core through the MeshCore Expo module (UniFFI + JNA). */
export async function createNativeClient(): Promise<CoreClient> {
  const M = meshCoreNative();
  await M.start();
  // Never block startup on the permission dialog (it can be interrupted by
  // activity recreation); re-apply the radio plan once it resolves.
  M.requestPermissions()
    .then(() => M.call('radio_restart', '{}'))
    .catch(() => {});
  const listeners = new Set<(e: CoreEvent) => void>();
  M.addListener('onCoreEvent', ({ json }) => {
    const e = JSON.parse(json) as CoreEvent;
    for (const l of listeners) l(e);
  });
  return {
    call: async <T,>(method: string, args: Record<string, unknown> = {}) => parseReply<T>(await M.call(method, JSON.stringify(args))),
    subscribe: (fn) => {
      listeners.add(fn);
      return () => {
        listeners.delete(fn);
      };
    },
    hooks: {
      platformLabel: 'Android',
      // The sender may use the OS decoder (12.3). The photo is resized to
      // 1,600 px and re-encoded as baseline JPEG, then the Rust core decodes,
      // re-encodes and strips everything again before sending.
      pickImage: async () => {
        const r = await ImagePicker.launchImageLibraryAsync({ mediaTypes: ['images'], quality: 1, exif: false });
        if (r.canceled || !r.assets[0]) return null;
        const a = r.assets[0];
        const scale = Math.min(1, 1600 / Math.max(a.width, a.height));
        const ctx = ImageManipulator.manipulate(a.uri);
        if (scale < 1) ctx.resize({ width: Math.round(a.width * scale), height: Math.round(a.height * scale) });
        const img = await ctx.renderAsync();
        const saved = await img.saveAsync({ format: SaveFormat.JPEG, compress: 0.85, base64: true });
        return saved.base64 ? { kind: 'jpeg' as const, base64: saved.base64 } : null;
      },
      pickFile: async () => {
        const picked = await File.pickFileAsync();
        const f = Array.isArray(picked) ? picked[0] : (picked as { result?: File }).result ?? (picked as unknown as File);
        if (!f || !(f instanceof File)) return null;
        return { name: f.name, bytes: new Uint8Array(await f.arrayBuffer()) };
      },
      pickFileRef: async () => {
        const picked = await File.pickFileAsync();
        const f = Array.isArray(picked) ? picked[0] : (picked as { result?: File }).result ?? (picked as unknown as File);
        if (!f || !(f instanceof File)) return null;
        return { name: f.name, size: f.size ?? 0, uri: f.uri };
      },
      sendFileRef: async (conversation, file, options) => {
        parseReply(await M.sendFileUri(conversation, file.uri, file.name, JSON.stringify(options)));
      },
      call: {
        audio: async (on, wifi) => {
          await M.callAudio(on, wifi);
        },
        mute: async (on) => {
          await M.callMute(on);
        },
        speaker: async (on) => {
          await M.callSpeaker(on);
        },
        ringback: async (on) => {
          await M.callRing(on ? 'outgoing' : 'none');
        },
        askMic: async () => (await M.micPermission()).granted,
        micGranted: () => M.micGranted(),
        deviceLocked: () => M.deviceLocked(),
        unlockDevice: () => M.unlockDevice(),
      },
      openAppSettings: () => {
        M.openAppSettings();
      },
      video: {
        info: (message) => M.videoInfo(message),
        open: async (message) => {
          await M.openVideo(message);
        },
      },
      voice: {
        micGranted: () => M.micGranted(),
        micBlocked: () => M.micBlocked(),
        askMic: async () => (await M.micPermission()).granted,
        start: async () => {
          await M.voiceStart();
        },
        pauseRecording: (on) => M.voicePause(on),
        stop: () => M.voiceStop(),
        cancel: async () => {
          await M.voiceCancel();
        },
        send: async (conversation, options, replyTo) => {
          parseReply(await M.voiceSend(conversation, JSON.stringify(options), replyTo ?? null));
        },
        play: async (message, fromMs, speed) => {
          await M.voicePlay(message, Math.round(fromMs), speed);
        },
        pause: async () => {
          await M.voicePausePlayback();
        },
        resume: async () => {
          await M.voiceResume();
        },
        seek: async (ms) => {
          await M.voiceSeek(Math.round(ms));
        },
        setSpeed: async (speed) => {
          await M.voiceSpeed(speed);
        },
        stopPlayback: async () => {
          await M.voiceStopPlayback();
        },
        onLevel: (fn) => {
          const s = M.addListener('onVoiceLevel', fn);
          return () => s.remove();
        },
        onLimit: (fn) => {
          const s = M.addListener('onVoiceLimit', () => fn());
          return () => s.remove();
        },
        onPlayback: (fn) => {
          const s = M.addListener('onVoicePlayback', fn);
          return () => s.remove();
        },
      },
      savePhoto: async (message) => {
        await M.savePhoto(message);
      },
      sharePhoto: async (message) => {
        await M.sharePhoto(message);
      },
      saveFile: async (name, bytes) => {
        const dir = await Directory.pickDirectoryAsync();
        const f = dir.createFile(name, 'application/octet-stream');
        f.write(bytes);
      },
      copyText: (text) => M.copySensitive(text),
      attestationChain: () => M.attestationChain(),
      lora: { scan: () => M.loraScan(), connect: (a) => M.loraConnect(a), forget: () => M.loraForget(), status: () => M.loraStatus() },
    },
  };
}


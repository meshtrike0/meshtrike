import { requireNativeView } from 'expo';
import React, { useEffect, useRef, useState } from 'react';
import { Platform, Text, View } from 'react-native';
import { b64decode, b64encode } from '../core/client';
import { Icon } from './icons';
import { Choice } from './kit';
import { useApp } from './state';
import { color, radius, space, type } from './theme';

// Android: CameraX preview whose luma frames are decoded in Rust (QrScanner.kt).
const NativeScanner: React.ComponentType<{ fps: number; style: object; onCode: (e: { nativeEvent: { base64: string } }) => void }> | null =
  Platform.OS === 'android' ? requireNativeView('MeshQrScanner') : null;

/**
 * Camera that hands luma frames to the Rust QR decoder (D25) and reports
 * decoded payloads. In the browser harness the "camera" is pointed at another
 * phone's screen; on Android it is the device camera.
 */
export function QrCamera({ onCode, fps = 4, active = true }: { onCode: (bytes: Uint8Array) => void | Promise<void>; fps?: number; active?: boolean }) {
  const { client, call } = useApp();
  const [targets, setTargets] = useState<{ id: string; label: string }[]>([]);
  const [target, setTarget] = useState<string | null>(null);
  const busy = useRef(false);
  const cb = useRef(onCode);
  cb.current = onCode;

  useEffect(() => {
    const t = setInterval(() => setTargets(client.hooks.cameraTargets?.() ?? []), 500);
    return () => clearInterval(t);
  }, [client]);

  useEffect(() => {
    client.hooks.pointCamera?.(target);
    return () => client.hooks.pointCamera?.(null);
  }, [client, target]);

  useEffect(() => {
    if (!active || NativeScanner) return;
    const t = setInterval(async () => {
      if (busy.current) return;
      busy.current = true;
      try {
        const frame = await client.hooks.captureCamera?.();
        if (frame) {
          const codes = await call<string[]>('decode_qr_luma', { width: frame.width, height: frame.height, luma: b64encode(frame.luma) });
          for (const c of codes) await cb.current(b64decode(c));
        }
      } finally {
        busy.current = false;
      }
    }, 1000 / fps);
    return () => clearInterval(t);
  }, [client, call, fps, active]);

  if (NativeScanner) {
    return (
      <NativeScanner
        fps={fps}
        style={{ height: 300, borderRadius: radius.lg, overflow: 'hidden' }}
        onCode={(e) => {
          if (active) cb.current(b64decode(e.nativeEvent.base64));
        }}
      />
    );
  }
  const harness = !!client.hooks.cameraTargets;
  return (
    <View style={{ gap: space.md }}>
      <View
        style={{ height: 200, borderRadius: radius.lg, backgroundColor: '#000', alignItems: 'center', justifyContent: 'center', borderWidth: 2, borderColor: target ? color.primary : color.border }}
        accessibilityLabel="Camera viewfinder"
      >
        <Icon name="camera" size={36} tint={target ? color.primary : color.textFaint} />
        <Text style={[type.small, { marginTop: space.sm }]}>{target ? `Looking at ${targets.find((t) => t.id === target)?.label ?? 'screen'}` : 'Point the camera at the code'}</Text>
      </View>
      {harness ? (
        targets.length ? (
          <View style={{ gap: space.xs }}>
            <Text style={type.small}>Simulated camera: point at</Text>
            <Choice options={targets.map((t) => ({ value: t.id, label: t.label }))} value={target ?? ''} onChange={setTarget} />
          </View>
        ) : (
          <Text style={type.small}>No phone is showing a code yet. Open "Show my code" on another phone.</Text>
        )
      ) : null}
    </View>
  );
}

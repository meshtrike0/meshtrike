import React from 'react';
import { requireNativeView } from 'expo';
import { Platform, Text, View } from 'react-native';
import { RgbaImage } from '../core/types';

// Android: a native view that copies raw RGBA into a Bitmap
// (Bitmap.copyPixelsFromBuffer) — no OS image decoder is involved (D31).
let NativeRgba: React.ComponentType<{ rgbaBase64: string; pixelWidth: number; pixelHeight: number; style: object }> | null = null;
if (Platform.OS === 'android') {
  try {
    NativeRgba = requireNativeView('MeshCore');
  } catch {
    NativeRgba = null;
  }
}

export function RgbaView({ image, maxWidth }: { image: RgbaImage; maxWidth: number }) {
  const w = Math.min(maxWidth, image.width);
  const h = (image.height / image.width) * w;
  if (!NativeRgba) {
    return (
      <View style={{ width: w, height: h, alignItems: 'center', justifyContent: 'center', backgroundColor: '#222' }}>
        <Text style={{ color: '#aaa' }}>Image view unavailable</Text>
      </View>
    );
  }
  return <NativeRgba rgbaBase64={image.rgba} pixelWidth={image.width} pixelHeight={image.height} style={{ width: w, height: h, borderRadius: 10 }} />;
}

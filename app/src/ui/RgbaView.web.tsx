import React, { useEffect, useRef } from 'react';
import { b64decode } from '../core/client';
import { RgbaImage } from '../core/types';

/** Draws decoded RGBA pixels (decoded in Rust) — never hands bytes to a codec. */
export function RgbaView({ image, maxWidth }: { image: RgbaImage; maxWidth: number }) {
  const ref = useRef<HTMLCanvasElement | null>(null);
  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    c.width = image.width;
    c.height = image.height;
    const px = b64decode(image.rgba);
    const data = new ImageData(new Uint8ClampedArray(px), image.width, image.height);
    c.getContext('2d')?.putImageData(data, 0, 0);
  }, [image]);
  const w = Math.min(maxWidth, image.width);
  const h = (image.height / image.width) * w;
  return React.createElement('canvas', { ref, role: 'img', 'aria-label': 'Photo', style: { width: w, height: h, borderRadius: 10, display: 'block' } });
}

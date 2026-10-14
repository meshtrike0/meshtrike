//! Photo encode/decode and text/filename sanitising (blueprint section 9).
//!
//! The receiving side never hands untrusted image bytes to OS codecs: every
//! received photo goes through [`decode_jpeg`], which accepts only small,
//! baseline (SOF0) JPEGs and decodes them with a pure-Rust decoder.
#![forbid(unsafe_code)]

use std::panic::{catch_unwind, AssertUnwindSafe};

pub mod voice;

use mesh_constants::{MAX_FILE_NAME_BYTES, MAX_IMAGE_BYTES};

/// Longest side, in pixels, of any photo we send or accept.
pub const MAX_IMAGE_SIDE: u32 = 1600;

/// JPEG quality ladder tried in order until the result fits `MAX_IMAGE_BYTES`.
const QUALITY_LADDER: [u8; 5] = [80, 70, 60, 50, 40];
/// How many times the image is shrunk by 0.75 before giving up.
const MAX_SHRINK_ROUNDS: usize = 6;

/// A decoded image, 8-bit RGBA, row-major, no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    #[error("image too large")]
    TooLarge,
    #[error("not a baseline JPEG")]
    NotBaseline,
    #[error("malformed image")]
    Malformed,
    #[error("invalid image dimensions")]
    Dimensions,
    #[error("JPEG encoding failed")]
    EncodeFailed,
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Re-encodes an RGBA bitmap as a metadata-free baseline JPEG of at most
/// `MAX_IMAGE_BYTES`, with the longest side at most `MAX_IMAGE_SIDE` pixels.
/// Alpha is composited onto white.
pub fn encode_photo(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, MediaError> {
    if width == 0 || height == 0 {
        return Err(MediaError::Dimensions);
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|p| p.checked_mul(4))
        .ok_or(MediaError::Dimensions)?;
    if rgba.len() != expected {
        return Err(MediaError::Dimensions);
    }

    let rgb = composite_on_white(rgba);
    let (mut tw, mut th) = fit_within(width, height, MAX_IMAGE_SIDE);

    for _round in 0..=MAX_SHRINK_ROUNDS {
        let scaled = if tw == width && th == height {
            rgb.clone()
        } else {
            resize_area_rgb(&rgb, width, height, tw, th)
        };
        for &q in QUALITY_LADDER.iter() {
            let jpeg = encode_baseline_rgb(&scaled, tw, th, q)?;
            if jpeg.len() <= MAX_IMAGE_BYTES {
                return Ok(jpeg);
            }
        }
        let nw = ((tw as f64) * 0.75).floor() as u32;
        let nh = ((th as f64) * 0.75).floor() as u32;
        if nw == 0 || nh == 0 {
            break;
        }
        tw = nw;
        th = nh;
    }
    Err(MediaError::TooLarge)
}

/// Composites RGBA over an opaque white background, returning packed RGB.
fn composite_on_white(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.as_chunks::<4>().0 {
        let a = px[3] as u32;
        for &c in &px[..3] {
            let v = (c as u32 * a + 255 * (255 - a) + 127) / 255;
            out.push(v as u8);
        }
    }
    out
}

/// Target size with the longest side at most `max_side`, preserving aspect
/// ratio (never upscales, never returns zero).
fn fit_within(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    let longest = w.max(h);
    if longest <= max_side {
        return (w, h);
    }
    let scale = max_side as f64 / longest as f64;
    let tw = ((w as f64 * scale).round() as u32).clamp(1, max_side);
    let th = ((h as f64 * scale).round() as u32).clamp(1, max_side);
    (tw, th)
}

/// For each destination index, the source span and per-source weights of an
/// area-averaging (box) downscale from `src` to `dst` samples (`dst <= src`).
fn area_weights(src: u32, dst: u32) -> Vec<(usize, Vec<f32>)> {
    let scale = src as f64 / dst as f64;
    (0..dst)
        .map(|i| {
            let start = i as f64 * scale;
            let end = ((i + 1) as f64 * scale).min(src as f64);
            let first = start.floor() as usize;
            let last = (end.ceil() as usize).min(src as usize).max(first + 1);
            let weights = (first..last)
                .map(|j| {
                    let lo = start.max(j as f64);
                    let hi = end.min((j + 1) as f64);
                    ((hi - lo).max(0.0) / scale) as f32
                })
                .collect();
            (first, weights)
        })
        .collect()
}

/// Area-averaging downscale of packed RGB. Processes one destination row at a
/// time so memory stays at O(source width) beyond the output.
fn resize_area_rgb(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (sw_us, dw_us) = (sw as usize, dw as usize);
    let xw = area_weights(sw, dw);
    let yw = area_weights(sh, dh);
    let mut out = Vec::with_capacity(dw_us * dh as usize * 3);
    let mut row = vec![0f32; sw_us * 3];
    for (y0, wys) in &yw {
        row.iter_mut().for_each(|v| *v = 0.0);
        for (k, &wy) in wys.iter().enumerate() {
            let sy = y0 + k;
            let line = &src[sy * sw_us * 3..(sy + 1) * sw_us * 3];
            for (acc, &s) in row.iter_mut().zip(line) {
                *acc += wy * s as f32;
            }
        }
        for (x0, wxs) in &xw {
            let mut px = [0f32; 3];
            for (k, &wx) in wxs.iter().enumerate() {
                let sx = (x0 + k) * 3;
                px[0] += wx * row[sx];
                px[1] += wx * row[sx + 1];
                px[2] += wx * row[sx + 2];
            }
            for c in px {
                out.push(c.round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    out
}

/// Baseline (SOF0), non-progressive JPEG with no metadata segments.
fn encode_baseline_rgb(rgb: &[u8], w: u32, h: u32, quality: u8) -> Result<Vec<u8>, MediaError> {
    let w16 = u16::try_from(w).map_err(|_| MediaError::Dimensions)?;
    let h16 = u16::try_from(h).map_err(|_| MediaError::Dimensions)?;
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, quality);
    enc.set_progressive(false);
    // Standard tables: jpeg-encoder 0.7's optimized tables mis-decode on
    // flat images (verified against zune-jpeg), so keep the Annex K tables.
    enc.set_optimized_huffman_tables(false);
    enc.encode(rgb, w16, h16, jpeg_encoder::ColorType::Rgb)
        .map_err(|_| MediaError::EncodeFailed)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Frame parameters from the single SOF0 segment.
#[derive(Debug, Clone, Copy)]
struct FrameInfo {
    width: u32,
    height: u32,
}

/// Walks every marker in the file (including inside entropy-coded data) and
/// requires exactly one SOF0 frame with 8-bit precision, 1 or 3 components and
/// dimensions in `1..=MAX_IMAGE_SIDE`. Any other frame type is rejected.
fn scan_markers(b: &[u8]) -> Result<FrameInfo, MediaError> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return Err(MediaError::Malformed);
    }
    let mut pos = 2usize;
    let mut frame: Option<FrameInfo> = None;
    let mut seen_sos = false;

    loop {
        // Expect a marker: one or more 0xFF fill bytes then a non-0xFF code.
        if pos >= b.len() || b[pos] != 0xFF {
            return Err(MediaError::Malformed);
        }
        while pos < b.len() && b[pos] == 0xFF {
            pos += 1;
        }
        let Some(&m) = b.get(pos) else {
            return Err(MediaError::Malformed);
        };
        pos += 1;

        match m {
            0xD9 => break, // EOI
            0x00 | 0xD8 => return Err(MediaError::Malformed),
            0x01 | 0xD0..=0xD7 => continue, // standalone markers
            _ => {}
        }

        // Segment with a 2-byte length (including the length field itself).
        let len = match (b.get(pos), b.get(pos + 1)) {
            (Some(&hi), Some(&lo)) => u16::from_be_bytes([hi, lo]) as usize,
            _ => return Err(MediaError::Malformed),
        };
        if len < 2 || pos + len > b.len() {
            return Err(MediaError::Malformed);
        }
        let seg = &b[pos + 2..pos + len];
        pos += len;

        match m {
            0xC0 => {
                if frame.is_some() || seen_sos {
                    return Err(MediaError::Malformed);
                }
                frame = Some(parse_sof0(seg)?);
            }
            // Every other SOFn (C4 = DHT, C8 = JPG, CC = DAC are not frames).
            0xC1..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err(MediaError::NotBaseline);
            }
            // DAC (arithmetic), DNL, DHP/EXP (hierarchical), JPG extensions.
            0xC8 | 0xCC | 0xDC | 0xDE | 0xDF | 0xF0..=0xFD => {
                return Err(MediaError::NotBaseline);
            }
            0xDA => {
                if frame.is_none() {
                    return Err(MediaError::Malformed);
                }
                seen_sos = true;
                // Skip entropy-coded data up to the next real marker.
                loop {
                    match b.get(pos) {
                        None => {
                            // Truncated without EOI: still have to have a frame.
                            return frame.ok_or(MediaError::Malformed);
                        }
                        Some(0xFF) => match b.get(pos + 1) {
                            Some(0x00) | Some(0xD0..=0xD7) => pos += 2,
                            Some(0xFF) => pos += 1,
                            Some(_) => break,
                            None => return frame.ok_or(MediaError::Malformed),
                        },
                        Some(_) => pos += 1,
                    }
                }
            }
            _ => {} // DHT, DQT, DRI, APPn, COM: allowed
        }
    }

    if !seen_sos {
        return Err(MediaError::Malformed);
    }
    frame.ok_or(MediaError::Malformed)
}

fn parse_sof0(seg: &[u8]) -> Result<FrameInfo, MediaError> {
    if seg.len() < 6 {
        return Err(MediaError::Malformed);
    }
    let precision = seg[0];
    let height = u16::from_be_bytes([seg[1], seg[2]]) as u32;
    let width = u16::from_be_bytes([seg[3], seg[4]]) as u32;
    let ncomp = seg[5] as usize;
    if precision != 8 || !(ncomp == 1 || ncomp == 3) || seg.len() != 6 + 3 * ncomp {
        return Err(MediaError::Malformed);
    }
    if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(MediaError::Dimensions);
    }
    Ok(FrameInfo { width, height })
}

/// Decodes an untrusted JPEG. Accepts only baseline (SOF0) files of at most
/// `MAX_IMAGE_BYTES` with both sides in `1..=MAX_IMAGE_SIDE`. Never panics.
pub fn decode_jpeg(bytes: &[u8]) -> Result<RgbaImage, MediaError> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(MediaError::TooLarge);
    }
    let frame = scan_markers(bytes)?;

    // Defence in depth: the decoder is pure safe Rust, but contain any panic.
    let decoded = catch_unwind(AssertUnwindSafe(|| {
        use zune_core::bytestream::ZCursor;
        use zune_core::colorspace::ColorSpace;
        use zune_core::options::DecoderOptions;

        let opts = DecoderOptions::new_safe()
            .set_max_width(MAX_IMAGE_SIDE as usize)
            .set_max_height(MAX_IMAGE_SIDE as usize)
            .set_strict_mode(true)
            .jpeg_set_max_scans(64)
            .jpeg_set_out_colorspace(ColorSpace::RGBA);
        let mut dec = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
        let pixels = dec.decode().ok()?;
        let info = dec.info()?;
        Some((info.width as u32, info.height as u32, pixels))
    }));

    let (w, h, mut rgba) = match decoded {
        Ok(Some(v)) => v,
        _ => return Err(MediaError::Malformed),
    };
    if w != frame.width || h != frame.height {
        return Err(MediaError::Malformed);
    }
    if rgba.len() != (w as usize) * (h as usize) * 4 {
        return Err(MediaError::Malformed);
    }
    for px in rgba.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    Ok(RgbaImage {
        width: w,
        height: h,
        rgba,
    })
}

// ---------------------------------------------------------------------------
// Sanitisers
// ---------------------------------------------------------------------------

/// Reduces a received file name to `[A-Za-z0-9._-]`, at most
/// `MAX_FILE_NAME_BYTES` bytes, with no leading or trailing dots and no runs of
/// dots (so never ".", "..", or a hidden file). Empty results become "file".
pub fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len().min(MAX_FILE_NAME_BYTES));
    for c in name.chars() {
        if !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') {
            continue;
        }
        if c == '.' && (out.is_empty() || out.ends_with('.')) {
            continue; // strips leading dots and collapses runs
        }
        out.push(c);
    }
    out.truncate(MAX_FILE_NAME_BYTES); // ASCII only, so byte == char boundary
    while out.ends_with('.') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("file");
    }
    out
}

fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}'
    )
}

/// Removes Unicode bidi controls and C0/C1 control characters (and DEL),
/// keeping only '\n' and '\t'.
pub fn sanitize_text(s: &str) -> String {
    s.chars()
        .filter(|&c| !is_bidi_control(c) && (!c.is_control() || c == '\n' || c == '\t'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn gradient(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                v.push((x * 255 / w.max(1)) as u8);
                v.push((y * 255 / h.max(1)) as u8);
                v.push(((x + y) % 256) as u8);
                v.push(255);
            }
        }
        v
    }

    fn small_jpeg() -> Vec<u8> {
        encode_photo(&gradient(64, 48), 64, 48).unwrap()
    }

    fn has_marker_segment(b: &[u8], marker: u8) -> bool {
        b.windows(2).any(|w| w[0] == 0xFF && w[1] == marker)
    }

    #[test]
    fn encode_large_gradient_fits_and_decodes() {
        let (w, h) = (3000, 2000);
        let jpeg = encode_photo(&gradient(w, h), w, h).unwrap();
        assert!(jpeg.len() <= MAX_IMAGE_BYTES, "len {}", jpeg.len());
        // Baseline and no EXIF/APP1 metadata.
        assert!(has_marker_segment(&jpeg, 0xC0));
        assert!(!has_marker_segment(&jpeg, 0xC2));
        assert!(!has_marker_segment(&jpeg, 0xE1));
        let img = decode_jpeg(&jpeg).unwrap();
        assert!(img.width.max(img.height) <= MAX_IMAGE_SIDE);
        assert_eq!((img.width, img.height), (1600, 1067));
        assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
    }

    #[test]
    fn encode_noise_shrinks_until_it_fits() {
        // Incompressible noise forces the shrink rounds.
        let (w, h) = (1600u32, 1600u32);
        let mut s: u32 = 0x1234_5678;
        let px: Vec<u8> = (0..w * h * 4)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s as u8
            })
            .collect();
        let jpeg = encode_photo(&px, w, h).unwrap();
        assert!(jpeg.len() <= MAX_IMAGE_BYTES);
        let img = decode_jpeg(&jpeg).unwrap();
        assert!(img.width < 1600);
    }

    #[test]
    fn encode_rejects_bad_buffer() {
        assert_eq!(encode_photo(&[0; 15], 2, 2), Err(MediaError::Dimensions));
        assert_eq!(encode_photo(&[], 0, 0), Err(MediaError::Dimensions));
    }

    #[test]
    fn alpha_composited_on_white() {
        let px = vec![0u8; 16 * 16 * 4]; // fully transparent black
        let jpeg = encode_photo(&px, 16, 16).unwrap();
        let img = decode_jpeg(&jpeg).unwrap();
        assert!(img
            .rgba
            .chunks(4)
            .all(|p| p[0] > 240 && p[1] > 240 && p[2] > 240));
    }

    #[test]
    fn small_roundtrip() {
        let img = decode_jpeg(&small_jpeg()).unwrap();
        assert_eq!((img.width, img.height), (64, 48));
        assert!(img.rgba.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn decode_rejects_progressive() {
        let rgb = vec![128u8; 32 * 32 * 3];
        let mut out = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut out, 80);
        enc.set_progressive(true);
        enc.encode(&rgb, 32, 32, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        assert!(has_marker_segment(&out, 0xC2));
        assert_eq!(decode_jpeg(&out), Err(MediaError::NotBaseline));
    }

    #[test]
    fn decode_rejects_handcrafted_sof2_and_others() {
        for sof in [
            0xC1u8, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF,
        ] {
            let b = [
                0xFF, 0xD8, 0xFF, sof, 0x00, 0x0B, 0x08, 0x00, 0x10, 0x00, 0x10, 0x01, 0x01, 0x11,
                0x00, 0xFF, 0xD9,
            ];
            assert_eq!(decode_jpeg(&b), Err(MediaError::NotBaseline), "sof {sof:x}");
        }
    }

    #[test]
    fn decode_rejects_extra_sof_after_scan() {
        let mut j = small_jpeg();
        // Replace EOI with a SOF2 segment then EOI.
        j.truncate(j.len() - 2);
        j.extend_from_slice(&[
            0xFF, 0xC2, 0x00, 0x0B, 0x08, 0x00, 0x10, 0x00, 0x10, 0x01, 0x01, 0x11, 0x00, 0xFF,
            0xD9,
        ]);
        assert_eq!(decode_jpeg(&j), Err(MediaError::NotBaseline));
    }

    #[test]
    fn decode_rejects_oversize() {
        assert_eq!(
            decode_jpeg(&vec![0u8; MAX_IMAGE_BYTES + 1]),
            Err(MediaError::TooLarge)
        );
    }

    #[test]
    fn decode_rejects_bad_dimensions() {
        let mut j = small_jpeg();
        let i = j.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        // height at i+5..i+7, width at i+7..i+9
        j[i + 5] = 0x06;
        j[i + 6] = 0x41; // 1601
        assert_eq!(decode_jpeg(&j), Err(MediaError::Dimensions));
        j[i + 5] = 0;
        j[i + 6] = 0;
        assert_eq!(decode_jpeg(&j), Err(MediaError::Dimensions));
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_jpeg(&[]).is_err());
        assert!(decode_jpeg(&[0xFF, 0xD8]).is_err());
        assert!(decode_jpeg(&[0xFF, 0xD8, 0xFF, 0xD9]).is_err());
        assert!(decode_jpeg(b"\x89PNG\r\n\x1a\n").is_err());
    }

    #[test]
    fn filename_sanitizer() {
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("."), "file");
        assert_eq!(sanitize_filename(".."), "file");
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_filename("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_filename(".bashrc"), "bashrc");
        assert_eq!(sanitize_filename("a..b...c"), "a.b.c");
        assert_eq!(sanitize_filename("evil\u{202E}gpj.exe"), "evilgpj.exe");
        assert_eq!(sanitize_filename("név fájl.txt"), "nvfjl.txt");
        assert_eq!(sanitize_filename("C:\\x\\y.txt"), "Cxy.txt");
        let long = "a".repeat(200);
        assert_eq!(sanitize_filename(&long).len(), MAX_FILE_NAME_BYTES);
        assert_eq!(sanitize_filename("trailing..."), "trailing");
    }

    #[test]
    fn text_sanitizer() {
        assert_eq!(sanitize_text("hi\u{202E}there"), "hithere");
        assert_eq!(
            sanitize_text("\u{2066}a\u{2067}b\u{2068}c\u{2069}\u{200E}\u{200F}\u{061C}"),
            "abc"
        );
        assert_eq!(
            sanitize_text("line1\nline2\tx\r\0\u{7}\u{1B}[31m"),
            "line1\nline2\tx[31m"
        );
        assert_eq!(sanitize_text("c1\u{85}\u{9B}\u{7F}"), "c1");
        assert_eq!(sanitize_text("emoji 😀 العربية"), "emoji 😀 العربية");
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

        #[test]
        fn decode_never_panics_on_random(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let _ = decode_jpeg(&bytes);
            let mut prefixed = vec![0xFF, 0xD8];
            prefixed.extend_from_slice(&bytes);
            let _ = decode_jpeg(&prefixed);
        }

        #[test]
        fn decode_never_panics_on_mutations(
            muts in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..16),
            cut in any::<prop::sample::Index>(),
        ) {
            let mut j = small_jpeg();
            for (idx, val) in muts {
                let i = idx.index(j.len());
                j[i] = val;
            }
            let _ = decode_jpeg(&j);
            j.truncate(cut.index(j.len()));
            let _ = decode_jpeg(&j);
        }

        #[test]
        fn filename_always_safe(s in "\\PC{0,200}") {
            let f = sanitize_filename(&s);
            prop_assert!(!f.is_empty() && f.len() <= MAX_FILE_NAME_BYTES);
            prop_assert!(!f.starts_with('.'));
            prop_assert!(f != "." && f != "..");
            prop_assert!(f.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)));
        }

        #[test]
        fn text_has_no_controls(s in "\\PC{0,200}|[\\x00-\\x1f\\u{80}-\\u{9f}\\u{202a}-\\u{202e}]{0,50}") {
            let t = sanitize_text(&s);
            prop_assert!(t.chars().all(|c| !is_bidi_control(c) && (!c.is_control() || c == '\n' || c == '\t')));
        }
    }
}

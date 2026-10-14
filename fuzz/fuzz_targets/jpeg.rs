#![no_main]
use libfuzzer_sys::fuzz_target;

// Section 23: the image decoder must never crash on hostile input.
fuzz_target!(|data: &[u8]| {
    if let Ok(img) = mesh_media::decode_jpeg(data) {
        assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
        assert!(img.width <= 1600 && img.height <= 1600);
    }
    let _ = mesh_media::sanitize_text(&String::from_utf8_lossy(data));
    let _ = mesh_media::sanitize_filename(&String::from_utf8_lossy(data));
});

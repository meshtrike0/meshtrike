#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = mesh_handoff::parse_bundle(data);
    let mut rx = mesh_handoff::HandoffReceiver::new();
    for f in data.chunks(812) {
        let _ = rx.push_frame(f);
    }
});

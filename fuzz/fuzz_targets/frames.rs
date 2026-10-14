#![no_main]
use libfuzzer_sys::fuzz_target;
use mesh_wire::*;

fuzz_target!(|data: &[u8]| {
    let mut r = Reassembler::default();
    for (i, chunk) in data.chunks(40).enumerate() {
        if let Ok(f) = LinkFrame::decode(chunk) {
            let _ = r.push(f, i as u64 * 1000);
        }
    }
    let _ = decode_summary(data);
    let _ = decode_request(data);
});

#![no_main]
use libfuzzer_sys::fuzz_target;
use mesh_wire::*;

fuzz_target!(|data: &[u8]| {
    if let Ok(ip) = InnerPlaintext::decode(data) {
        let _ = Content::decode(ip.kind, &ip.content, ip.chunk_index == 0);
    }
    if let Some((&k, rest)) = data.split_first() {
        if let Ok(Some(c)) = Content::decode(k, rest, true) {
            let _ = c.encode();
        }
    }
});

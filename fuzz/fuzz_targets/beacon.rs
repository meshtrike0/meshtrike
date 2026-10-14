#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(b) = mesh_discovery::Beacon::decode(data) {
        let _ = b.recognise(&[[1; 32], [2; 32]], 1_900_000);
        assert_eq!(mesh_discovery::Beacon::decode(&b.encode()), Some(b));
    }
});

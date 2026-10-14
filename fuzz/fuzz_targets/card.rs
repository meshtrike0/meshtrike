#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = mesh_identity::ContactCard::decode_verified(data);
    let _ = mesh_identity::PairingQr::decode(data);
    let _ = mesh_identity::decode_community_qr(data);
});

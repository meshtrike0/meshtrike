#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(p) = mesh_wire::Post::decode(data) {
        let msg = mesh_wire::Post::signing_bytes(&[0; 16], &[0; 16], &p.pseudonym_pk, &p.reply_tag, &p.inner);
        let _ = mesh_crypto::ed25519_verify(&p.pseudonym_pk, &msg, &p.signature);
    }
});

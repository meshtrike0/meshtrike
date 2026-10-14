#![no_main]
use libfuzzer_sys::fuzz_target;
use mesh_wire::*;

fuzz_target!(|data: &[u8]| {
    if let Ok(env) = Envelope::decode(data) {
        let _ = parse_ack(&env);
        let key = mesh_crypto::SymKey([7; 32]);
        let _ = open_payload(&env, &key);
        assert_eq!(Envelope::decode(&env.encode()).as_ref(), Ok(&env));
    }
    let _ = parse_direct_inner(data);
    let _ = parse_group_inner(data);
    let _ = parse_handshake_inner(data);
});

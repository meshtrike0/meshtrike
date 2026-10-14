//! JNI bindings for org.meshtrike.internet.Native.

use crate::{set_network_policy, Event, Node, Proxy};
use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, JNI_FALSE, JNI_TRUE};
use jni::{JNIEnv, JavaVM};
use std::sync::{Arc, Mutex, OnceLock};

static NODE: Mutex<Option<Node>> = Mutex::new(None);
static JVM: OnceLock<JavaVM> = OnceLock::new();
static CALLBACK: OnceLock<GlobalRef> = OnceLock::new();

fn string(env: &mut JNIEnv, s: &JString) -> Option<String> {
    env.get_string(s).ok().map(Into::into)
}

/// Deliver an event to `Native.onEvent(int kind, byte[] data)`.
fn deliver(e: Event) {
    let (Some(vm), Some(cb)) = (JVM.get(), CALLBACK.get()) else { return };
    let Ok(mut env) = vm.attach_current_thread_as_daemon() else { return };
    let (kind, data): (i32, Vec<u8>) = match e {
        Event::Ready(a) => (1, a.into_bytes()),
        Event::Down(r) => (2, r.into_bytes()),
        Event::Envelope(b) => (3, b),
        Event::Gateway(g) => (4, g.into_bytes()),
    };
    let Ok(arr) = env.byte_array_from_slice(&data) else { return };
    let _ = env.call_static_method(
        cb,
        "onEvent",
        "(I[B)V",
        &[JValue::Int(kind), JValue::Object(&JObject::from(arr))],
    );
    let _ = env.exception_clear();
}

/// `static native boolean start(String dir, String proxyHost, int proxyPort, String user, String pass)`
/// proxyHost null means a direct connection.
#[no_mangle]
pub extern "system" fn Java_org_meshtrike_internet_Native_start(
    mut env: JNIEnv,
    class: JClass,
    dir: JString,
    proxy_host: JString,
    proxy_port: i32,
    user: JString,
    pass: JString,
) -> jboolean {
    if JVM.get().is_none() {
        if let Ok(vm) = env.get_java_vm() {
            let _ = JVM.set(vm);
        }
    }
    if CALLBACK.get().is_none() {
        if let Ok(g) = env.new_global_ref(&class) {
            let _ = CALLBACK.set(g);
        }
    }
    let Some(dir) = string(&mut env, &dir) else { return JNI_FALSE };
    let proxy = if proxy_host.is_null() {
        None
    } else {
        let host = string(&mut env, &proxy_host).unwrap_or_default();
        let Ok(addr) = format!("{host}:{proxy_port}").parse() else { return JNI_FALSE };
        Some(Proxy {
            addr,
            user: string(&mut env, &user).unwrap_or_default(),
            pass: string(&mut env, &pass).unwrap_or_default(),
        })
    };
    let mut node = NODE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(n) = node.take() {
        n.stop();
    }
    set_network_policy(proxy);
    match Node::start(dir.into(), Arc::new(deliver)) {
        Ok(n) => {
            *node = Some(n);
            JNI_TRUE
        }
        Err(_) => JNI_FALSE,
    }
}

#[no_mangle]
pub extern "system" fn Java_org_meshtrike_internet_Native_stop(_env: JNIEnv, _class: JClass) {
    if let Some(n) = NODE.lock().unwrap_or_else(|p| p.into_inner()).take() {
        n.stop();
    }
}

/// `static native boolean send(String address, byte[] envelope)`
#[no_mangle]
pub extern "system" fn Java_org_meshtrike_internet_Native_send(
    mut env: JNIEnv,
    _class: JClass,
    address: JString,
    envelope: JByteArray,
) -> jboolean {
    let Some(addr) = string(&mut env, &address) else { return JNI_FALSE };
    let Ok(bytes) = env.convert_byte_array(&envelope) else { return JNI_FALSE };
    let node = NODE.lock().unwrap_or_else(|p| p.into_inner());
    match node.as_ref() {
        Some(n) if n.send(&addr, &bytes) => JNI_TRUE,
        _ => JNI_FALSE,
    }
}

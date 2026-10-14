// Loads the Rust core compiled to WebAssembly (core/wasm, wasm-bindgen
// --target no-modules) from /mesh-core/. Web only.

export interface WasmCore {
  call(method: string, argsJson: string): string;
  on_frame(peer: number, bytes: Uint8Array): void;
  tick(): void;
  drain_commands(): string;
  drain_events(): string;
  advance_clock(ms: number): void;
  set_lora(on: boolean): void;
  set_net_online(on: boolean): void;
  is_fresh(): boolean;
  debug_state(): string;
  free(): void;
}

interface Bindgen {
  (opts: { module_or_path: string }): Promise<unknown>;
  WasmCore: new (configJson: string) => WasmCore;
}

let ready: Promise<Bindgen> | null = null;

export function loadCore(): Promise<Bindgen> {
  if (ready) return ready;
  ready = new Promise((resolve, reject) => {
    // The no-modules glue declares a top-level `let wasm_bindgen`, which is a
    // global binding but not a property of window.
    // eslint-disable-next-line no-new-func
    const lookup = () => new Function('return typeof wasm_bindgen === "undefined" ? undefined : wasm_bindgen')() as Bindgen | undefined;
    const init = () => {
      const b = lookup();
      if (!b) return reject(new Error('mesh-core glue missing'));
      b({ module_or_path: '/mesh-core/mesh_wasm_bg.wasm' }).then(() => resolve(b), reject);
    };
    if (lookup()) return init();
    const s = document.createElement('script');
    s.src = '/mesh-core/mesh_wasm.js';
    s.onload = init;
    s.onerror = () => reject(new Error('could not load /mesh-core/mesh_wasm.js'));
    document.head.appendChild(s);
  });
  return ready;
}

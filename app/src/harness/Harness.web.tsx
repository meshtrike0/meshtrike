import React, { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { MeshApp } from '../ui/MeshApp';
import { Ether, Phone } from './ether';

const NAMES = ['Alice', 'Bob', 'Carol', 'Dave', 'Erin', 'Farid', 'Grace', 'Hana', 'Ivan', 'Jun'];
const SCALE = 6; // px per metre
const MAP_W = 130;
const MAP_H = 70;

const css = {
  page: { minHeight: '100vh', background: '#E9E6DF', color: '#15171C', fontFamily: 'Geist_500Medium, system-ui, -apple-system, Segoe UI, Roboto, sans-serif' },
  bar: { display: 'flex', flexWrap: 'wrap' as const, gap: 8, alignItems: 'center', padding: '10px 16px', borderBottom: '1px solid #DDD9D0', background: 'rgba(246,245,241,0.95)', position: 'sticky' as const, top: 0, zIndex: 10 },
  btn: { background: '#fff', color: '#15171C', border: '1.5px solid #DDD9D0', borderRadius: 99, padding: '7px 13px', cursor: 'pointer', fontSize: 13 },
  btnOn: { background: '#CDF25C', color: '#15171C', border: '1.5px solid #CDF25C' },
  label: { fontSize: 12, color: '#5D616B' },
};

function Btn({ on, children, onClick, title }: { on?: boolean; children: React.ReactNode; onClick: () => void; title?: string }) {
  return (
    <button style={{ ...css.btn, ...(on ? css.btnOn : {}) }} onClick={onClick} title={title} aria-pressed={on}>
      {children}
    </button>
  );
}

export function Harness() {
  const ether = useMemo(() => new Ether(), []);
  const [, force] = useReducer((x: number) => x + 1, 0);
  const [error, setError] = useState<string | null>(null);
  const [jamMode, setJamMode] = useState(false);
  const throttle = useRef(0);

  useEffect(() => {
    document.body.style.overflow = 'auto';
    document.title = 'Meshtrike harness';
    const off = ether.onChange(() => {
      const now = performance.now();
      if (now - throttle.current > 200) {
        throttle.current = now;
        force();
      }
    });
    ether.start();
    (async () => {
      try {
        await ether.addPhone(NAMES[0], 30, 35);
        await ether.addPhone(NAMES[1], 45, 35);
      } catch (e) {
        setError(String(e));
      }
    })();
    return () => {
      off();
      ether.stop();
    };
  }, [ether]);

  const add = async (x?: number, y?: number, lora = false) => {
    const n = ether.phones.length;
    await ether.addPhone(NAMES[n % NAMES.length] + (n >= NAMES.length ? ` ${Math.floor(n / NAMES.length) + 1}` : ''), x ?? 10 + ((n * 17) % (MAP_W - 20)), y ?? 15 + ((n * 23) % (MAP_H - 30)), lora);
  };

  const scenario = async (kind: 'chain' | 'crowd') => {
    for (const p of [...ether.phones]) ether.removePhone(p.id);
    if (kind === 'chain') for (let i = 0; i < 4; i++) await add(10 + i * 28, 35);
    else for (let i = 0; i < 6; i++) await add(30 + (i % 3) * 18, 25 + Math.floor(i / 3) * 18);
  };

  return (
    <div style={css.page} data-testid="harness">
      <div style={css.bar}>
        <strong style={{ marginRight: 8 }}>Meshtrike · browser harness</strong>
        <Btn onClick={() => add()} title="Add a phone">+ Phone</Btn>
        <Btn onClick={() => add(undefined, undefined, true)} title="Phone with a paired LoRa radio">+ Phone with LoRa</Btn>
        <Btn onClick={() => scenario('chain')}>Relay chain</Btn>
        <Btn onClick={() => scenario('crowd')}>Crowd</Btn>
        <span style={css.label}>|</span>
        <Btn on={jamMode} onClick={() => setJamMode((j) => !j)} title="Click on the map to place a jammer">Place jammer</Btn>
        <Btn onClick={() => { ether.jammers = []; force(); }}>Clear jammers</Btn>
        <span style={css.label}>|</span>
        <Btn onClick={() => ether.advance(10 * 60_000)}>+10 min</Btn>
        <Btn onClick={() => ether.advance(60 * 60_000)}>+1 hour</Btn>
        <Btn on={ether.paused} onClick={() => { ether.paused = !ether.paused; force(); }}>{ether.paused ? 'Resume radio' : 'Pause radio'}</Btn>
        <label style={css.label}>
          Range {ether.rangeM} m{' '}
          <input type="range" min={10} max={60} value={ether.rangeM} onChange={(e) => { ether.rangeM = Number(e.target.value); force(); }} />
        </label>
        <label style={css.label}>
          Loss {Math.round(ether.loss * 100)}%{' '}
          <input type="range" min={0} max={30} value={ether.loss * 100} onChange={(e) => { ether.loss = Number(e.target.value) / 100; force(); }} />
        </label>
        <label style={css.label}>
          <input type="checkbox" checked={ether.fastKdf} onChange={(e) => { ether.fastKdf = e.target.checked; force(); }} /> Fast PIN stretching for new phones (harness only)
        </label>
        <label style={css.label} title="Each phone's internet route uses a local helper running the real Nym client (mac_helper --port 47910+N)">
          <input type="checkbox" checked={ether.realNet} onChange={(e) => { ether.realNet = e.target.checked; force(); }} /> Real mixnet (local helpers)
        </label>
        <span style={css.label}>
          frames {ether.stats.frames} · {(ether.stats.bytes / 1024).toFixed(0)} KB on air
        </span>
      </div>
      {error ? <div style={{ padding: 16, color: '#ff6b6b' }}>Core failed to load: {error}. Run `npm run build:core` first.</div> : null}
      <MapView ether={ether} jamMode={jamMode} onJam={() => setJamMode(false)} />
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 24, padding: 16, alignItems: 'flex-start' }}>
        {ether.phones.map((p) => (
          <PhoneFrame key={p.id} phone={p} ether={ether} />
        ))}
      </div>
    </div>
  );
}

function MapView({ ether, jamMode, onJam }: { ether: Ether; jamMode: boolean; onJam: () => void }) {
  const drag = useRef<number | null>(null);
  const svg = useRef<SVGSVGElement | null>(null);
  const toMetres = (e: React.PointerEvent) => {
    const r = svg.current!.getBoundingClientRect();
    return { x: Math.max(0, Math.min(MAP_W, (e.clientX - r.left) / SCALE)), y: Math.max(0, Math.min(MAP_H, (e.clientY - r.top) / SCALE)) };
  };
  return (
    <div style={{ padding: '12px 16px 0' }}>
      <div style={{ ...css.label, marginBottom: 6 }}>
        Map {MAP_W} × {MAP_H} m — drag phones to move them. Circles show Bluetooth range; lines are live encrypted links.
      </div>
      <svg
        ref={svg}
        width={MAP_W * SCALE}
        height={MAP_H * SCALE}
        data-testid="map"
        style={{ background: '#15171C', border: '1px solid #15171C', borderRadius: 24, touchAction: 'none', cursor: jamMode ? 'crosshair' : 'default', maxWidth: '100%' }}
        onPointerDown={(e) => {
          if (jamMode) {
            const m = toMetres(e);
            ether.jammers.push({ x: m.x, y: m.y, r: 25 });
            onJam();
            ether.changed();
          }
        }}
        onPointerMove={(e) => {
          if (drag.current == null) return;
          const p = ether.byId(drag.current);
          if (!p) return;
          const m = toMetres(e);
          p.x = m.x;
          p.y = m.y;
          ether.changed();
        }}
        onPointerUp={() => (drag.current = null)}
        onPointerLeave={() => (drag.current = null)}
      >
        {[...Array(Math.floor(MAP_W / 10))].map((_, i) => (
          <line key={`v${i}`} x1={i * 10 * SCALE} y1={0} x2={i * 10 * SCALE} y2={MAP_H * SCALE} stroke="#172024" />
        ))}
        {[...Array(Math.floor(MAP_H / 10))].map((_, i) => (
          <line key={`h${i}`} x1={0} y1={i * 10 * SCALE} x2={MAP_W * SCALE} y2={i * 10 * SCALE} stroke="#172024" />
        ))}
        {ether.jammers.map((j, i) => (
          <g key={`j${i}`}>
            <circle cx={j.x * SCALE} cy={j.y * SCALE} r={j.r * SCALE} fill="rgba(255,107,107,0.12)" stroke="#ff6b6b" strokeDasharray="6 4" />
            <text x={j.x * SCALE} y={j.y * SCALE} fill="#ff6b6b" fontSize={12} textAnchor="middle">
              jammer
            </text>
          </g>
        ))}
        {ether.phones.map((p) => (
          <circle key={`r${p.id}`} cx={p.x * SCALE} cy={p.y * SCALE} r={ether.rangeM * SCALE} fill="rgba(61,220,151,0.04)" stroke="rgba(61,220,151,0.25)" />
        ))}
        {ether.links().map(([a, b]) => {
          const pa = ether.byId(a);
          const pb = ether.byId(b);
          if (!pa || !pb) return null;
          return <line key={`${a}-${b}`} x1={pa.x * SCALE} y1={pa.y * SCALE} x2={pb.x * SCALE} y2={pb.y * SCALE} stroke="#7cc4ff" strokeWidth={2} />;
        })}
        {ether.phones.map((p) => (
          <g
            key={p.id}
            style={{ cursor: 'grab' }}
            onPointerDown={(e) => {
              e.stopPropagation();
              drag.current = p.id;
            }}
            data-testid={`map-phone-${p.name}`}
          >
            <circle cx={p.x * SCALE} cy={p.y * SCALE} r={9} fill={p.powered ? (p.advertising.length ? '#3ddc97' : '#f2c14e') : '#555'} stroke="#0a0d0e" strokeWidth={2} />
            {p.lora ? <circle cx={p.x * SCALE} cy={p.y * SCALE} r={13} fill="none" stroke="#c39bff" strokeWidth={1.5} /> : null}
            <text x={p.x * SCALE} y={p.y * SCALE - 14} fill="#e8eef0" fontSize={12} textAnchor="middle">
              {p.name}
            </text>
          </g>
        ))}
      </svg>
    </div>
  );
}

function PhoneFrame({ phone, ether }: { phone: Phone; ether: Ether }) {
  const now = new Date();
  const time = `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}`;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, alignItems: 'center' }} data-testid={`phone-${phone.name}`}>
      <div style={{ display: 'flex', gap: 6, alignItems: 'center', fontSize: 13 }}>
        <strong>{phone.name}</strong>
        <span style={css.label}>
          ({phone.x.toFixed(0)}, {phone.y.toFixed(0)}) m · {phone.advertising.length ? 'advertising' : 'silent'}
        </span>
        <Btn on={phone.powered} onClick={() => { phone.powered = !phone.powered; ether.changed(); }} title="Bluetooth on/off">BT</Btn>
        <Btn on={phone.lora} onClick={() => { phone.lora = !phone.lora; phone.core.set_lora(phone.lora); ether.changed(); }} title="Paired LoRa radio">LoRa</Btn>
        <Btn on={phone.netConnectivity} onClick={() => { phone.netConnectivity = !phone.netConnectivity; ether.netRefresh(phone); }} title={`Internet connectivity (route ${phone.netUp ? 'up' : 'down'})`}>Net{phone.netUp ? ' ●' : ''}</Btn>
        <Btn onClick={() => ether.removePhone(phone.id)} title="Remove phone">✕</Btn>
      </div>
      <div
        style={{
          width: 372,
          height: 772,
          background: '#15171C',
          borderRadius: 50,
          padding: 6,
          boxShadow: '0 40px 80px -30px rgba(40,36,26,.45), 0 0 0 1px rgba(0,0,0,.25)',
          position: 'relative',
        }}
      >
        <div style={{ width: 360, height: 760, borderRadius: 44, overflow: 'hidden', display: 'flex', flexDirection: 'column', background: '#F6F5F1' }}>
          <div style={{ height: 28, display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '0 26px', fontSize: 13, fontWeight: 600, color: '#15171C', background: '#F6F5F1', position: 'relative' }}>
            <span>{time}</span>
            <span style={{ position: 'absolute', left: '50%', top: 5, width: 18, height: 18, marginLeft: -9, borderRadius: 9, background: '#15171C' }} />
            <span>{phone.powered ? 'ᛒ' : ''} ▮▮▮ 87%</span>
          </div>
          <div style={{ flex: 1, display: 'flex', position: 'relative', minHeight: 0 }}>
            <MeshApp client={phone} />
          </div>
          <div style={{ height: 20, display: 'flex', alignItems: 'center', justifyContent: 'center', background: '#F6F5F1' }}>
            <div style={{ width: 120, height: 5, borderRadius: 9, background: '#15171C' }} />
          </div>
        </div>
      </div>
    </div>
  );
}

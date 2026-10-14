import React, { ReactNode, useEffect, useRef, useState } from 'react';
import { Animated, Easing, Linking, Platform, Pressable, StyleSheet, Text, useWindowDimensions, View } from 'react-native';
import Svg, { Circle, Defs, RadialGradient, Rect, Stop } from 'react-native-svg';
import { meshCoreNative } from '../../../modules/mesh-core/src/MeshCoreModule';
import { b64encode } from '../../core/client';
import { Bridge, Community, HandoffProgress, Hex, IntegrityStatus, QrMatrix, RadioMode } from '../../core/types';
import { QrCamera } from '../Camera';
import { REACH, ReachCard, reachUi } from '../Reach';
import { VERIFY_URL } from '../../config';
import { Icon, IconName } from '../icons';
import {
  Appear,
  Avatar,
  Banner,
  Button,
  Card,
  Choice,
  ErrorText,
  Eyebrow,
  Field,
  HoldButton,
  IconTile,
  native,
  OptionGrid,
  QrView,
  Row,
  Screen,
  Section,
  Sheet,
  Toggle,
  ToggleRow,
  useLoop,
  useReduceMotion,
} from '../kit';
import appJson from '../../../app.json';
import { DISAPPEAR_OPTIONS, errorText, timeAgo, useApp, useCoreEvents } from '../state';
import { CARRIER, color, font, type } from '../theme';

// ===== Network =====

/** Fixed spots around "You" on the radar (percent of the card), as in the design. */
const SPOTS: [number, number][] = [
  [24, 27],
  [76, 33],
  [66, 78],
  [30, 74],
  [88, 62],
  [12, 54],
  [52, 11],
  [44, 88],
];

function ago(ms: number | null | undefined) {
  if (!ms) return '—';
  const m = Math.round((Date.now() - ms) / 60000);
  if (m < 0) return timeAgo(ms);
  if (m < 1) return 'Just now';
  if (m < 60) return `${m} min`;
  if (m < 48 * 60) return `${Math.round(m / 60)} h`;
  return `${Math.round(m / 1440)} d`;
}

function Ping({ delay }: { delay: number }) {
  const v = useRef(new Animated.Value(0)).current;
  const still = useReduceMotion();
  useEffect(() => {
    if (still) return;
    let loop: Animated.CompositeAnimation | null = null;
    const t = setTimeout(() => {
      loop = Animated.loop(Animated.timing(v, { toValue: 1, duration: 3000, easing: Easing.out(Easing.quad), useNativeDriver: native }));
      loop.start();
    }, delay);
    return () => {
      clearTimeout(t);
      loop?.stop();
    };
  }, [v, delay, still]);
  return (
    <Animated.View
      pointerEvents="none"
      style={{
        position: 'absolute',
        left: '50%',
        top: '50%',
        width: 320,
        height: 320,
        marginLeft: -160,
        marginTop: -160,
        opacity: v.interpolate({ inputRange: [0, 1], outputRange: [0.6, 0] }),
        transform: [{ scale: v.interpolate({ inputRange: [0, 1], outputRange: [0.15, 1] }) }],
      }}
    >
      <Svg width={320} height={320}>
        <Defs>
          <RadialGradient id="ping" cx="50%" cy="50%" r="50%">
            <Stop offset="0" stopColor={color.lime} stopOpacity={0.35} />
            <Stop offset="0.7" stopColor={color.lime} stopOpacity={0} />
          </RadialGradient>
        </Defs>
        <Circle cx={160} cy={160} r={160} fill="url(#ping)" />
      </Svg>
    </Animated.View>
  );
}

function Ring({ size }: { size: number }) {
  return (
    <View
      pointerEvents="none"
      style={{ position: 'absolute', left: '50%', top: '50%', width: size, height: size, marginLeft: -size / 2, marginTop: -size / 2, borderRadius: size, borderWidth: 1, borderColor: 'rgba(255,255,255,0.12)' }}
    />
  );
}

function Radar() {
  const { network, contacts } = useApp();
  const neighbours = network?.neighbours ?? 0;
  const near = contacts.filter((c) => c.nearby).slice(0, SPOTS.length);
  const dots = Math.max(0, Math.min(SPOTS.length - near.length, neighbours - near.length));
  const alone = neighbours === 0 && near.length === 0;
  return (
    <View
      style={{ height: 300, borderRadius: 28, backgroundColor: color.ink, overflow: 'hidden' }}
      accessibilityLabel={alone ? 'No phones in range' : `${neighbours} phones in range, ${near.length} contacts nearby`}
    >
      <Ring size={250} />
      <Ring size={160} />
      {/* Pings only while this phone is actually on air. */}
      {network?.advertising ? (
        <>
          <Ping delay={0} />
          <Ping delay={1500} />
        </>
      ) : null}
      {near.map((c, i) => (
        <Appear
          key={c.id}
          kind="pop"
          delay={i * 150}
          style={{ position: 'absolute', left: `${SPOTS[i][0]}%`, top: `${SPOTS[i][1]}%`, width: 64, marginLeft: -32, marginTop: -26, alignItems: 'center', gap: 4 }}
        >
          <View style={{ padding: 3, borderRadius: 14, backgroundColor: '#fff' }}>
            <Avatar seed={c.identity_key} size={34} />
          </View>
          <Text style={{ fontFamily: font.semi, fontSize: 11, color: '#fff' }} numberOfLines={1}>
            {c.nickname.split(' ')[0]}
          </Text>
        </Appear>
      ))}
      {Array.from({ length: dots }).map((_, j) => {
        const [x, y] = SPOTS[near.length + j];
        return (
          <Appear key={`d${j}`} kind="pop" delay={(near.length + j) * 150} style={{ position: 'absolute', left: `${x}%`, top: `${y}%`, marginLeft: -5, marginTop: -5 }}>
            <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: 'rgba(255,255,255,0.55)' }} />
          </Appear>
        );
      })}
      <View style={{ position: 'absolute', left: '50%', top: '50%', width: 56, height: 56, marginLeft: -28, marginTop: -28, borderRadius: 19, backgroundColor: color.lime, alignItems: 'center', justifyContent: 'center' }}>
        <Text style={{ fontFamily: font.bold, fontSize: 13, color: color.ink }}>You</Text>
      </View>
      {alone ? (
        <Text style={{ position: 'absolute', left: 16, right: 16, bottom: 16, textAlign: 'center', color: '#fff', fontFamily: font.medium, fontSize: 13.5, lineHeight: 19 }}>
          No phones in range. Your messages wait safely until someone comes near.
        </Text>
      ) : null}
    </View>
  );
}

function Stat({ value, label, onInfo }: { value: string; label: string; onInfo?: () => void }) {
  return (
    <View style={st.stat}>
      <View accessible accessibilityLabel={`${label}: ${value}`} style={{ gap: 4 }}>
        <Text style={{ fontFamily: font.display, fontSize: 26, color: color.ink }}>{value}</Text>
        <Text style={{ fontFamily: font.medium, fontSize: 12.5, color: color.textMuted }}>{label}</Text>
      </View>
      {onInfo ? (
        <Pressable onPress={onInfo} accessibilityRole="button" accessibilityLabel="What this counts" hitSlop={6} style={({ pressed }) => [st.statInfo, pressed ? { opacity: 0.6 } : null]}>
          <Icon name="info" size={15} />
        </Pressable>
      ) : null}
    </View>
  );
}

/**
 * Keeps a tab screen above the floating tab bar while its sheet is open or
 * still sliding away, so the scrim covers the whole phone.
 */
function useLinger(on: boolean, ms = 320) {
  const [shown, setShown] = useState(on);
  // Adjusting state while rendering (React's pattern for "state from props").
  if (on && !shown) setShown(true);
  useEffect(() => {
    if (on) return;
    const t = setTimeout(() => setShown(false), ms);
    return () => clearTimeout(t);
  }, [on, ms]);
  return on || shown;
}

/** A screen with a bottom sheet as its last child (Sheet fills its parent). */
function SheetHost({ raised, children }: { raised: boolean; children: ReactNode }) {
  return <View style={[st.host, raised ? st.hostRaised : null]}>{children}</View>;
}

// Relay store limits, as in core/constants (EXPIRY_DEFAULT_SECS, EXPIRY_MAX_SECS,
// RELAY_MAX_ENVELOPES, RELAY_MAX_BYTES).
const RELAY_EXPIRY_H = 24;
const RELAY_EXPIRY_MAX_H = 72;
const RELAY_MAX_MESSAGES = '2,000';
const RELAY_MAX_MB = 50;

function Transport({
  name,
  icon,
  status,
  desc,
  fg,
  bg,
  first,
  onPress,
}: {
  name: string;
  icon: IconName;
  status: string;
  desc: string;
  fg: string;
  bg: string;
  first?: boolean;
  onPress?: () => void;
}) {
  return (
    <Pressable
      disabled={!onPress}
      onPress={onPress}
      style={{ flexDirection: 'row', alignItems: 'center', gap: 12, paddingVertical: 12, borderTopWidth: first ? 0 : 1, borderTopColor: color.lineSoft }}
      accessibilityRole={onPress ? 'button' : undefined}
      accessibilityLabel={`${name}: ${status}. ${desc}`}
    >
      <IconTile icon={icon} bg={bg} fg={fg} size={40} iconSize={19} />
      <View style={{ flex: 1, gap: 2 }}>
        <Text style={{ fontFamily: font.semi, fontSize: 14.5, color: color.ink }}>{name}</Text>
        <Text style={{ fontFamily: font.regular, fontSize: 12, lineHeight: 16, color: color.textMuted }}>{desc}</Text>
      </View>
      <View style={{ paddingVertical: 5, paddingHorizontal: 9, borderRadius: 99, backgroundColor: bg }}>
        <Text style={{ fontFamily: font.semi, fontSize: 11.5, color: fg }}>{status}</Text>
      </View>
    </Pressable>
  );
}

export function Network() {
  const { network, refresh, push } = useApp();
  const [carry, setCarry] = useState(false);
  const raised = useLinger(carry);
  useEffect(() => {
    const t = setInterval(refresh, 2000);
    return () => clearInterval(t);
  }, [refresh]);
  const n = network;
  const hr = !!n?.high_risk;
  // A live Bluetooth link to someone, versus the radio merely being on.
  const bt = n?.transports.includes('Ble') ?? false;
  let btRadio = bt || !!n?.advertising;
  if (Platform.OS === 'android') {
    try {
      const nat = meshCoreNative();
      const perms = nat.permissionStatus();
      const blePerms = Object.entries(perms)
        .filter(([k]) => k.includes('BLUETOOTH') || k.endsWith('ACCESS_FINE_LOCATION'))
        .every(([, v]) => v);
      btRadio = nat.bluetoothEnabled() && blePerms;
    } catch {
      /* keep the core's view */
    }
  }
  const wifi = n?.transports.includes('WifiP2p') ?? false;
  const wifiCapable = !!n?.wifi_p2p_capable;
  // Android: Wi-Fi Direct runs in the MeshGRID add-on (Meshtrike has no INTERNET or Wi-Fi permission).
  let link = { installed: true, hardware: true, wifiOn: true, permitted: true };
  if (Platform.OS === 'android') {
    try {
      link = meshCoreNative().wifiLinkStatus();
    } catch {
      link = { installed: false, hardware: false, wifiOn: false, permitted: false };
    }
  }
  const openLink = () => {
    try {
      meshCoreNative().openWifiLink();
    } catch {
      /* add-on missing */
    }
  };
  const wifiOff = n ? !n.wifi_direct_on : false;
  const wifiStatus = hr
    ? 'Off in High-Risk'
    : wifiOff
      ? 'Off in Settings'
      : wifi
      ? 'Active'
      : !link.installed
        ? 'Add-on needed'
        : !link.hardware
          ? 'Not on this phone'
          : !link.permitted
            ? 'Allow in add-on'
            : !link.wifiOn
              ? 'Wi-Fi off'
              : wifiCapable
                ? 'Ready'
                : 'Unavailable';
  const wifiDesc = hr
    ? 'High-Risk keeps everything on Bluetooth'
    : wifiOff
      ? 'Switched off in Settings. Bluetooth carries everything.'
      : !link.installed
      ? 'Install the MeshGRID add-on for fast photos and files. Meshtrike itself stays offline.'
      : !link.hardware
        ? 'This phone cannot open Wi-Fi Direct links'
        : !link.permitted
          ? 'Tap to open MeshGRID and allow Nearby Wi-Fi'
          : !link.wifiOn
            ? 'Turn Wi-Fi on. No Wi-Fi network or internet is needed.'
            : 'Kicks in for photos and files over 64 KB';
  const off = { fg: color.danger, bg: color.dangerBg };
  const muted = { fg: color.grey, bg: '#EDEBE6' };
  // Bluetooth is "Active" whenever the radio is usable (design D:1563).
  const btOn = bt || btRadio;
  return (
    <SheetHost raised={raised}>
      <Screen title="Network" big>
        {n?.interference ? (
          <Banner
            tone="warning"
            icon="radio"
            title="Radio interference likely"
            text="Bluetooth went silent while you’re still. Try QR handoff, or move at least 100 m."
            action={<Button title="Handoff" icon="qr" size="sm" onPress={() => push({ name: 'Handoff' })} />}
          />
        ) : null}
        <Radar />
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 10 }}>
          <Stat value={String(n?.neighbours ?? 0)} label="Phones in range" />
          <Stat value={String(n?.contacts_nearby ?? 0)} label="Contacts nearby" />
          <Stat value={ago(n?.last_delivery_ms)} label="Last delivery" />
          <Stat value={String(n?.relay_envelopes ?? 0)} label="Carrying for others" onInfo={() => setCarry(true)} />
        </View>
        <View style={{ backgroundColor: '#fff', borderRadius: 24, paddingVertical: 6, paddingHorizontal: 14 }}>
          <Transport
            first
            name="Bluetooth"
            icon="bt"
            status={btOn ? 'Active' : 'Off'}
            desc={btOn ? 'Default for everything nearby' : 'Turn on Bluetooth and allow Nearby devices'}
            {...(btOn ? { fg: color.good, bg: color.goodBg } : off)}
          />
          <Transport
            name="Wi-Fi Direct"
            icon="wifi"
            status={wifiStatus}
            desc={wifiDesc}
            onPress={Platform.OS === 'android' && !hr ? (!link.installed ? reachUi.open : !link.permitted ? openLink : undefined) : undefined}
            {...((wifi || wifiCapable) && !hr && !wifiOff ? { fg: CARRIER.wifi.fg, bg: CARRIER.wifi.bg } : muted)}
          />
          <Transport
            name="LoRa radio"
            icon="radio"
            status={hr ? 'Off in High-Risk' : n?.lora_paired ? 'Paired' : 'Not paired'}
            desc="Short texts, fallback after 5 min"
            {...(n?.lora_paired && !hr ? { fg: CARRIER.lora.fg, bg: CARRIER.lora.bg } : muted)}
          />
          <Transport
            name="Internet"
            icon="globe"
            status={hr ? 'Never in High-Risk' : n?.internet_on ? 'Allowed' : 'Not allowed'}
            desc="Private route, only if you allow it"
            onPress={() => push({ name: 'Internet' })}
            {...muted}
          />
        </View>
        <Pressable
          onPress={() => push({ name: 'Handoff' })}
          accessibilityRole="button"
          accessibilityLabel="QR handoff"
          style={({ pressed }) => ({ borderRadius: 24, backgroundColor: color.lime, padding: 16, flexDirection: 'row', alignItems: 'center', gap: 12, transform: [{ scale: pressed ? 0.985 : 1 }] })}
        >
          <IconTile icon="qr" bg={color.ink} fg={color.lime} size={42} iconSize={20} />
          <View style={{ flex: 1, gap: 2 }}>
            <Text style={{ fontFamily: font.semi, fontSize: 15, color: color.ink }}>QR handoff</Text>
            <Text style={{ fontFamily: font.regular, fontSize: 12.5, color: color.limeInk }}>Move messages by camera when radio is jammed</Text>
          </View>
          <Icon name="right" size={20} />
        </Pressable>
        <Section title="Details">
          <Row
            title="Advertising"
            subtitle={n?.advertising ? `Yes · power ${n.tx_power}` : n?.radio_mode === 'ListenOnly' ? 'No · Listen-only mode' : 'No'}
            icon="bt"
          />
          <Row title="Carrying for others" subtitle={`${n?.relay_envelopes ?? 0} sealed messages · ${((n?.relay_bytes ?? 0) / 1024).toFixed(0)} KB`} icon="box" />
          <Row title="Undecryptable messages dropped" subtitle={String(n?.crypto_failures ?? 0)} icon="shieldX" />
        </Section>
      </Screen>
      <Sheet open={carry} onClose={() => setCarry(false)}>
        <View style={{ marginLeft: 4 }}>
          <IconTile icon="box" bg={color.chip2} size={48} iconSize={23} />
        </View>
        <Text style={st.sheetTitle} accessibilityRole="header">
          Carrying for others
        </Text>
        <Text style={st.sheetBody}>
          Sealed messages this phone holds for other people. It can’t read them. Every piece of other people’s photos and files counts, so a new phone often shows a higher number.
        </Text>
        <Text style={st.sheetBody}>
          {`They expire after ${RELAY_EXPIRY_H} hours (${RELAY_EXPIRY_MAX_H} at most) or when delivered. Limits: ${RELAY_MAX_MESSAGES} messages or ${RELAY_MAX_MB} MB.`}
        </Text>
        <Button title="Got it" onPress={() => setCarry(false)} style={{ height: 54 }} />
      </Sheet>
    </SheetHost>
  );
}

// ===== QR handoff =====

// One QR frame carries one 800-byte symbol (core/constants HANDOFF_SYMBOL_SIZE),
// shown 8 times a second (HANDOFF_FPS).
const HANDOFF_SYMBOL = 800;
const HANDOFF_FPS = 8;

function size(bytes: number) {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1).replace(/\.0$/, '')} MB`;
  return `${Math.round(bytes / 1024)} KB`;
}

export function Handoff() {
  const { pop, network } = useApp();
  const [mode, setMode] = useState<'choose' | 'send' | 'receive'>('choose');
  // The core also sends this phone's own waiting messages; the count covers the carried ones.
  const carried = network?.relay_envelopes ?? 0;
  return (
    <Screen title="QR handoff" onBack={pop}>
      {mode === 'choose' ? (
        <>
          <Text style={[type.small, { fontSize: 15, lineHeight: 21, paddingHorizontal: 4 }]}>
            Hold the phones face to face. One plays a moving code, the other films it. About 1 MB in 2–3 minutes, then you swap.
          </Text>
          <View accessibilityRole="radiogroup" style={{ gap: 14 }}>
            <Pressable
              onPress={() => setMode('send')}
              accessibilityRole="button"
              accessibilityLabel="Send (show codes)"
              style={({ pressed }) => ({ borderRadius: 26, backgroundColor: color.ink, padding: 22, gap: 14, transform: [{ scale: pressed ? 0.985 : 1 }] })}
            >
              <IconTile icon="qr" bg={color.lime} fg={color.ink} size={48} iconSize={24} />
              <Text style={{ fontFamily: font.display, fontSize: 22, color: '#fff' }}>Send</Text>
              <Text style={{ fontFamily: font.regular, fontSize: 13.5, lineHeight: 19, color: color.onInkMuted }}>
                {carried ? `Show a moving code with ${carried} sealed message${carried === 1 ? '' : 's'}, plus your own waiting ones.` : 'Show a moving code with your waiting sealed messages.'}
              </Text>
            </Pressable>
            <Pressable
              onPress={() => setMode('receive')}
              accessibilityRole="button"
              accessibilityLabel="Receive (film codes)"
              style={({ pressed }) => ({ borderRadius: 26, backgroundColor: '#fff', padding: 22, gap: 14, transform: [{ scale: pressed ? 0.985 : 1 }] })}
            >
              <IconTile icon="camera" bg={color.chip2} size={48} iconSize={24} />
              <Text style={{ fontFamily: font.display, fontSize: 22, color: color.ink }}>Receive</Text>
              <Text style={{ fontFamily: font.regular, fontSize: 13.5, lineHeight: 19, color: color.textMuted }}>Film the other phone’s code.</Text>
            </Pressable>
          </View>
        </>
      ) : mode === 'send' ? (
        <HandoffSend onCancel={() => setMode('choose')} onSwap={() => setMode('receive')} />
      ) : (
        <HandoffReceive onCancel={() => setMode('choose')} onSwap={() => setMode('send')} onDone={pop} />
      )}
    </Screen>
  );
}

function ProgressCard({ pct, left, detail }: { pct: number; left: string; detail: string }) {
  const w = Math.max(0, Math.min(100, pct));
  return (
    <View style={{ backgroundColor: '#fff', borderRadius: 22, padding: 16, gap: 10 }} accessibilityLabel={`${w.toFixed(0)} percent, ${left}. ${detail}`}>
      <View style={{ flexDirection: 'row', justifyContent: 'space-between', alignItems: 'baseline' }}>
        <Text style={{ fontFamily: font.display, fontSize: 30, color: color.ink }}>{w.toFixed(0)}%</Text>
        <Text style={{ fontFamily: font.medium, fontSize: 13, color: color.textMuted }}>{left}</Text>
      </View>
      <View style={{ height: 8, borderRadius: 9, backgroundColor: color.chip2, overflow: 'hidden' }}>
        <View style={{ height: 8, borderRadius: 9, backgroundColor: color.ink, width: `${w}%` }} />
      </View>
      <Text style={{ fontFamily: font.medium, fontSize: 12.5, color: color.textMuted }}>{detail}</Text>
    </View>
  );
}

function HandoffSend({ onCancel, onSwap }: { onCancel: () => void; onSwap: () => void }) {
  const { call, client } = useApp();
  const [qr, setQr] = useState<QrMatrix | null>(null);
  const [frames, setFrames] = useState(0);
  const [source, setSource] = useState(0);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let stop = false;
    let t: ReturnType<typeof setInterval> | null = null;
    (async () => {
      try {
        setSource(await call<number>('handoff_start_send'));
        // 8 frames per second (12.2).
        t = setInterval(async () => {
          if (stop) return;
          try {
            const r = await call<{ frame: string; qr: QrMatrix }>('handoff_next_frame');
            setQr(r.qr);
            client.hooks.showQr?.(r.qr);
            setFrames((f) => f + 1);
          } catch (e) {
            // Stop the stream and show the error once, not 8 times a second.
            stop = true;
            if (t) clearInterval(t);
            setError(errorText(e));
          }
        }, 125);
      } catch (e) {
        setError(errorText(e));
      }
    })();
    return () => {
      stop = true;
      if (t) clearInterval(t);
      client.hooks.showQr?.(null);
      call('handoff_stop').catch(() => {});
    };
  }, [call, client]);
  const pct = source ? (frames / source) * 100 : 0;
  // Every frame has been shown once; repair frames keep going until the other phone has it all.
  const firstPass = source > 0 && frames >= source;
  return (
    <>
      <View style={st.sendQr}>{qr ? <QrView matrix={qr} size={200} /> : null}</View>
      <ProgressCard
        pct={pct}
        left={!source ? 'Starting…' : firstPass ? 'Repeating' : `${Math.ceil((source - frames) / HANDOFF_FPS)} s left`}
        detail={source ? `${size(Math.min(frames, source) * HANDOFF_SYMBOL)} of ${size(source * HANDOFF_SYMBOL)} · keep the phones steady` : 'Preparing the code · keep the phones steady'}
      />
      <View style={{ flexDirection: 'row', gap: 8 }}>
        {firstPass ? <Button title="Swap roles" size="md" style={{ flex: 1 }} onPress={onSwap} /> : null}
        <Button title="Cancel" variant="secondary" size="md" style={{ flex: 1 }} onPress={onCancel} />
      </View>
      <ErrorText error={error} />
    </>
  );
}

/** Dark viewfinder backdrop (design: radial-gradient(120% 80% at 50% 30%, #3A3F38, #15171C)). */
function ViewfinderGlow() {
  return (
    <Svg style={StyleSheet.absoluteFill} width="100%" height="100%" pointerEvents="none">
      <Defs>
        <RadialGradient id="vf" cx="50%" cy="30%" fx="50%" fy="30%" rx="120%" ry="80%">
          <Stop offset="0" stopColor="#3A3F38" />
          <Stop offset="1" stopColor={color.ink} />
        </RadialGradient>
      </Defs>
      <Rect x="0" y="0" width="100%" height="100%" fill="url(#vf)" />
    </Svg>
  );
}

function ScanLine({ height }: { height: number }) {
  const v = useLoop(2000);
  return (
    <Animated.View
      pointerEvents="none"
      style={{
        position: 'absolute',
        left: 40,
        right: 40,
        top: 0,
        height: 2,
        backgroundColor: color.lime,
        shadowColor: color.lime,
        shadowOpacity: 0.8,
        shadowRadius: 10,
        transform: [{ translateY: v.interpolate({ inputRange: [0, 0.5, 1], outputRange: [height * 0.14, height * 0.82, height * 0.14] }) }],
      }}
    />
  );
}

function HandoffReceive({ onCancel, onSwap, onDone }: { onCancel: () => void; onSwap: () => void; onDone: () => void }) {
  const { call } = useApp();
  const { height: winH } = useWindowDimensions();
  const [p, setP] = useState<HandoffProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const done = useRef(false);
  useEffect(() => {
    call('handoff_start_receive').catch((e) => setError(errorText(e)));
    return () => {
      call('handoff_stop').catch(() => {});
    };
  }, [call]);
  if (p?.done) {
    return (
      // Centred in the space under the header, as in the design.
      <Appear kind="pop" style={{ minHeight: Math.max(380, winH - 190), alignItems: 'center', justifyContent: 'center', gap: 14 }}>
        <IconTile icon={p.error ? 'alert' : 'check'} bg={p.error ? color.dangerBg : color.lime} fg={p.error ? color.danger : color.ink} size={72} iconSize={34} />
        <Text style={{ fontFamily: font.display, fontSize: 26, color: color.ink }} accessibilityRole="header">
          {p.error ? 'Transfer damaged' : 'Handoff complete'}
        </Text>
        <Text style={{ fontFamily: font.regular, fontSize: 14, lineHeight: 20, color: color.textMuted, textAlign: 'center', maxWidth: 270 }}>
          {p.error ??
            `${size(p.needed * HANDOFF_SYMBOL)} moved by camera${p.envelopes ? `, ${p.envelopes} sealed message${p.envelopes === 1 ? '' : 's'}` : ''}. Now swap so the other phone can receive yours.`}
        </Text>
        <View style={{ flexDirection: 'row', gap: 8, marginTop: 8 }}>
          <Button title="Swap roles" size="md" style={{ height: 52 }} onPress={onSwap} />
          <Button title="Done" variant="secondary" size="md" style={{ height: 52 }} onPress={onDone} />
        </View>
      </Appear>
    );
  }
  // Android shows the native scanner in the design's 290 x 290 square; the
  // harness camera also lists what to point at, so it keeps its own height.
  const square = Platform.OS === 'android';
  const viewfinder = square ? 290 : 200;
  return (
    <>
      <View style={square ? st.viewfinder : st.viewfinderHarness}>
        <ViewfinderGlow />
        <QrCamera
          fps={8}
          onCode={async (bytes) => {
            if (done.current) return;
            try {
              const r = await call<HandoffProgress>('handoff_push_frame', { frame: b64encode(bytes) });
              setP(r);
              setError(null);
              if (r.done) done.current = true;
            } catch (e) {
              // Frames arrive 8 times a second: keep one error on screen.
              setError((prev) => prev ?? errorText(e));
            }
          }}
        />
        <View pointerEvents="none" style={{ position: 'absolute', left: square ? 0 : 8, right: square ? 0 : 8, top: square ? 0 : 8, height: viewfinder }}>
          <ScanLine height={viewfinder} />
        </View>
      </View>
      {p ? (
        <ProgressCard
          pct={(p.received / Math.max(1, p.needed)) * 100}
          left={`${Math.ceil(Math.max(0, p.needed - p.received) / HANDOFF_FPS)} s left`}
          detail={`${size(Math.min(p.received, p.needed) * HANDOFF_SYMBOL)} of ${size(p.needed * HANDOFF_SYMBOL)} · keep the phones steady`}
        />
      ) : null}
      <Button title="Cancel" variant="secondary" size="md" onPress={onCancel} />
      <ErrorText error={error} />
    </>
  );
}

// ===== Settings =====

const AUTO_LOCK = [
  { value: '0', label: 'Immediately' },
  { value: '60000', label: '1 minute' },
  { value: '300000', label: '5 minutes' },
];

function shortKey(k: string | undefined) {
  const s = (k ?? '').replace(/[^0-9a-zA-Z]/g, '').toUpperCase().slice(0, 12);
  return s.match(/.{1,4}/g)?.join(' · ') ?? '';
}

function Value({ text }: { text: string }) {
  return <Text style={{ fontFamily: font.medium, fontSize: 13, color: color.textMuted }}>{text}</Text>;
}

export function Settings() {
  const { settings, call, push, refresh, identity, notify, client, network } = useApp();
  // Android's capability depends on the add-on (shown on the Network tab);
  // elsewhere the device either has Wi-Fi Direct or never will.
  const noWifiDirect = Platform.OS !== 'android' && !!network && !network.wifi_p2p_capable;
  const netAddon = useInternetAddon();
  const [integrity, setIntegrity] = useState<IntegrityStatus | null>(null);
  const [open, setOpen] = useState<null | 'duress' | 'disappear' | 'lock' | 'lora' | 'bio'>(null);
  const [biometric, setBiometric] = useState<boolean | null>(null);
  const [bioPin, setBioPin] = useState('');
  const [loraConn, setLoraConn] = useState(() => client.hooks.lora?.status());
  // The radio link changes outside React: re-read it whenever the core reports network changes.
  useCoreEvents(
    (e) => {
      if (e.event === 'NetworkChanged') setLoraConn(client.hooks.lora?.status());
    },
    [client],
  );
  const [pin, setPin] = useState('');
  const [duress, setDuress] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [communities, setCommunities] = useState<Community[]>([]);
  const raised = useLinger(open === 'disappear');
  useEffect(() => {
    (async () => {
      const chain = (await client.hooks.attestationChain?.()) ?? null;
      setIntegrity(await call<IntegrityStatus>('integrity_report', chain && chain.length ? { chain } : {}));
    })().catch((e) => setError(errorText(e)));
    call<Community[]>('communities').then(setCommunities).catch((e) => setError(errorText(e)));
    call<boolean>('biometric_enabled').then(setBiometric).catch(() => setBiometric(null));
  }, [call, client]);
  if (!settings) return null;
  /** Calls the core; true on success. Errors are shown and never followed by a success toast. */
  const set = async (method: string, args: Record<string, unknown>): Promise<boolean> => {
    setError(null);
    try {
      await call(method, args);
      await refresh();
      return true;
    } catch (e) {
      setError(errorText(e));
      notify(errorText(e));
      return false;
    }
  };
  const toggle = (k: NonNullable<typeof open>) => setOpen((o) => (o === k ? null : k));
  const modes: { value: RadioMode; label: string }[] = [
    { value: 'Public', label: 'Public' },
    { value: 'Community', label: 'Community' },
    { value: 'ListenOnly', label: 'Listen-only' },
  ];
  const modeDesc =
    {
      Public: 'Anyone nearby can find you.',
      Community: 'Only members of your communities. Your beacon changes every 15 minutes.',
      ListenOnly: 'Never advertises. Connects briefly only to send.',
    }[settings.radio_mode] +
    // The core can run a different mode than the one chosen (effective_mode).
    (network && network.radio_mode !== settings.radio_mode
      ? network.radio_mode === 'Community'
        ? ' Running Community now: High-Risk with a community forces it.'
        : ' Running Public now: you have no community yet.'
      : '');
  const warning = integrity && typeof integrity === 'object' ? integrity.Warning : null;
  const loraStatus = network?.high_risk
    ? 'Off in High-Risk mode'
    : loraConn?.connected || network?.lora_paired
      ? `Connected${loraConn?.name ? ` · ${loraConn.name}` : ''} · fallback after 5 min`
      : client.hooks.lora
        ? 'Not paired · tap to find a Meshtastic radio'
        : Platform.OS === 'web'
          ? 'In the harness, use the LoRa toggle above each phone'
          : 'Radio pairing is not available on this device';
  const disappear = DISAPPEAR_OPTIONS.find((o) => o.value === settings.disappear_default_secs)?.label ?? `${Math.round(settings.disappear_default_secs / 3600)} hours`;
  // High-Risk locks as soon as the app is left, whatever the stored value.
  const lockLabel = settings.high_risk
    ? 'Immediately (High-Risk)'
    : (AUTO_LOCK.find((o) => o.value === String(settings.auto_lock_ms))?.label ?? `${Math.round(settings.auto_lock_ms / 60000)} min`);
  const netValue = netStatus(network, settings.high_risk, netAddon);
  return (
    <SheetHost raised={raised}>
      <Screen title="Settings" big>
        {warning ? (
          <View accessibilityRole="alert" style={{ backgroundColor: color.warnBg, borderRadius: 20, padding: 14, flexDirection: 'row', gap: 10, alignItems: 'flex-start' }}>
            <Icon name="cpu" size={18} tint={color.warnIcon} />
            <View style={{ flex: 1, gap: 2 }}>
              <Text style={{ fontFamily: font.semi, fontSize: 14, color: color.ink }}>Device integrity can’t be verified</Text>
              <Text style={{ fontFamily: font.regular, fontSize: 12.5, lineHeight: 17, color: color.warnInk }}>{warning}</Text>
            </View>
          </View>
        ) : null}

        <Card style={{ flexDirection: 'row', alignItems: 'center', gap: 14 }}>
          <Avatar seed={identity?.identity_key ?? 'me'} size={56} />
          <View style={{ flex: 1, gap: 3 }}>
            <Text style={{ fontFamily: font.display, fontSize: 19, color: color.ink }} numberOfLines={1}>
              {identity?.nickname}
            </Text>
            <Text style={{ fontFamily: font.mono, fontSize: 12, color: color.textMuted }}>key {shortKey(identity?.identity_key)}</Text>
          </View>
          <Pressable
            onPress={() => push({ name: 'Pair', tab: 'show' })}
            accessibilityRole="button"
            accessibilityLabel="Show my code"
            style={{ width: 44, height: 44, borderRadius: 15, backgroundColor: color.ink, alignItems: 'center', justifyContent: 'center' }}
          >
            <Icon name="qr" size={20} tint={color.lime} />
          </Pressable>
        </Card>

        <Card>
          <Text style={type.label}>Radio mode</Text>
          <Choice
            options={modes}
            value={settings.radio_mode}
            onChange={(m) => (m === 'Community' && communities.length === 0 ? push({ name: 'Communities' }) : set('set_radio_mode', { mode: m }))}
          />
          <Text style={{ fontFamily: font.regular, fontSize: 12.5, lineHeight: 17, color: color.textMuted }}>{modeDesc}</Text>
          <Pressable
            onPress={() => push({ name: 'Communities' })}
            accessibilityRole="button"
            accessibilityLabel={`Communities, ${communities.length} of 4`}
            style={{ flexDirection: 'row', alignItems: 'center', gap: 10, paddingTop: 4 }}
          >
            <Icon name="users" size={18} />
            <Text style={{ flex: 1, fontFamily: font.medium, fontSize: 14.5, color: color.ink }}>Communities</Text>
            <Value text={`${communities.length} of 4`} />
            <Icon name="right" size={18} tint={color.textFaint} />
          </Pressable>
        </Card>
        <ReachCard />

        <Pressable
          onPress={() => (settings.high_risk ? set('set_high_risk', { on: false }) : push({ name: 'HighRisk' }))}
          accessibilityRole="button"
          accessibilityLabel={`High-Risk mode, ${settings.high_risk ? 'on' : 'off'}`}
          style={{ backgroundColor: color.ink, borderRadius: 24, padding: 16, flexDirection: 'row', alignItems: 'center', gap: 12 }}
        >
          <IconTile icon="zap" bg={color.lime} fg={color.ink} size={42} iconSize={20} />
          <View style={{ flex: 1, gap: 2 }}>
            <Text style={{ fontFamily: font.semi, fontSize: 15, color: '#fff' }}>High-Risk mode</Text>
            <Text style={{ fontFamily: font.regular, fontSize: 12.5, color: 'rgba(255,255,255,0.7)' }}>Text only, cover traffic, instant lock</Text>
          </View>
          <Toggle
            value={settings.high_risk}
            onChange={(v) => (v ? push({ name: 'HighRisk' }) : set('set_high_risk', { on: false }))}
            label="High-Risk mode"
            onDark
          />
        </Pressable>

        <Section style={st.section}>
          <ToggleRow title="Relay for others" subtitle="Carry sealed messages for up to 72 h" icon="box" value={settings.relay_enabled} onChange={(v) => set('set_relay_enabled', { on: v })} />
          <ToggleRow title="Read receipts" icon="checks" value={settings.read_receipts} onChange={(v) => set('set_read_receipts', { on: v })} />
          <ToggleRow
            title="Wi-Fi Direct for large files"
            subtitle={
              settings.high_risk
                ? 'Off in High-Risk'
                : Platform.OS === 'android'
                  ? 'Photos and files over 64 KB, when both phones have MeshGRID'
                  : noWifiDirect
                    ? 'Not available on this device. Bluetooth carries everything.'
                    : 'Photos and files over 64 KB, with a phone next to you'
            }
            icon="wifi"
            value={!settings.high_risk && !noWifiDirect && !!network?.wifi_direct_on}
            disabled={settings.high_risk || noWifiDirect}
            onChange={(v) => set('set_wifi_direct', { on: v })}
          />
          <Row title="Disappearing default" icon="timer" right={<Value text={disappear} />} label={`Disappearing default, ${disappear}`} onPress={() => setOpen('disappear')} />
          <Row
            title="Internet route"
            icon="globe"
            right={
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }}>
                <Value text={netValue} />
                <Icon name="right" size={18} tint={color.textFaint} />
              </View>
            }
            label={`Internet route, ${netValue}`}
            onPress={() => push({ name: 'Internet' })}
          />
          <Row
            title="LoRa radio"
            icon="radio"
            subtitle={loraStatus}
            right={<Icon name={open === 'lora' ? 'left' : 'right'} size={18} tint={color.textFaint} />}
            label="LoRa radio"
            onPress={() => toggle('lora')}
          />
          {open === 'lora' ? <LoraControls onChange={() => setLoraConn(client.hooks.lora?.status())} /> : null}
        </Section>

        <Section style={st.section}>
          <Row
            title="Lock after leaving the app"
            icon="clock"
            right={<Value text={lockLabel} />}
            label={`Lock after leaving the app, ${lockLabel}`}
            onPress={settings.high_risk ? undefined : () => toggle('lock')}
          />
          {open === 'lock' && !settings.high_risk ? (
            <View style={{ paddingBottom: 14 }}>
              <OptionGrid
                columns={3}
                options={AUTO_LOCK}
                value={String(settings.auto_lock_ms)}
                onChange={(v) => {
                  set('set_auto_lock', { ms: Number(v) });
                  setOpen(null);
                }}
              />
            </View>
          ) : null}
          <Row title="Lock now" icon="lock" onPress={() => call('lock').catch((e) => notify(errorText(e)))} />
          <Row
            title="Fingerprint unlock"
            subtitle={biometric ? 'A shortcut only. The PIN is still needed after a restart.' : 'Off · set it up with your PIN'}
            icon="finger"
            right={<Value text={biometric == null ? '…' : biometric ? 'On' : 'Off'} />}
            label={`Fingerprint unlock, ${biometric ? 'on' : 'off'}`}
            onPress={biometric ? undefined : () => toggle('bio')}
          />
          {open === 'bio' && !biometric ? (
            <View style={{ paddingBottom: 14, gap: 10 }}>
              <Field label="PIN to enable fingerprint" mono value={bioPin} onChangeText={(t) => setBioPin(t.replace(/\D/g, ''))} secureTextEntry keyboardType="number-pad" maxLength={6} />
              <Button
                title="Turn on fingerprint unlock"
                size="md"
                disabled={bioPin.length !== 6}
                onPress={async () => {
                  if (await set('enable_biometric', { pin: bioPin })) {
                    setBiometric(true);
                    setOpen(null);
                    notify('Fingerprint unlock is on');
                  }
                  setBioPin('');
                }}
              />
            </View>
          ) : null}
          <Row title="Duress PIN" subtitle="Entering it wipes everything silently" icon="key" onPress={() => toggle('duress')} />
          {open === 'duress' ? (
            <View style={{ paddingBottom: 14, gap: 10 }}>
              <Field label="Current PIN" mono value={pin} onChangeText={(t) => setPin(t.replace(/\D/g, ''))} secureTextEntry keyboardType="number-pad" maxLength={6} />
              <Field label="New duress PIN (empty = remove)" mono value={duress} onChangeText={(t) => setDuress(t.replace(/\D/g, ''))} secureTextEntry keyboardType="number-pad" maxLength={6} />
              <Button
                title="Save duress PIN"
                size="md"
                disabled={pin.length !== 6 || (duress !== '' && duress.length !== 6)}
                onPress={async () => {
                  const ok = await set('set_duress_pin', { pin, duress: duress || undefined });
                  setPin('');
                  setDuress('');
                  if (ok) {
                    setOpen(null);
                    notify(duress ? 'Duress PIN saved' : 'Duress PIN removed');
                  }
                }}
              />
            </View>
          ) : null}
          <Row
            title="Reset my sessions"
            subtitle="New keys with every contact; use after removing suspected spyware"
            icon="retry"
            onPress={async () => {
              if (await set('reset_sessions', {})) notify('Sessions renewed');
            }}
          />
          <Row
            title="Device integrity"
            subtitle={integrity === 'Ok' ? 'Verified boot and locked bootloader' : integrity == null ? 'Checking…' : integrity === 'Unknown' ? 'Not checked on this platform' : `Warning: ${warning}`}
            icon={warning ? 'shieldX' : 'shieldOk'}
          />
          <Row title="About & known limits" icon="info" right={<Value text={`v${appJson.expo.version}`} />} onPress={() => push({ name: 'About' })} />
        </Section>

        <ErrorText error={error} />
        <View style={{ gap: 8 }}>
          <HoldButton title="Hold 3 seconds to panic wipe" icon="flame" ms={3000} onDone={() => call('panic_wipe').catch((e) => notify(errorText(e)))} />
          <Text style={{ fontFamily: font.regular, fontSize: 12, lineHeight: 17, color: color.textFaint, textAlign: 'center' }}>Erases keys, contacts and messages. There is no backup.</Text>
        </View>
      </Screen>
      <Sheet open={open === 'disappear'} onClose={() => setOpen(null)} title="Disappearing messages" subtitle="The timer starts when a message is first shown.">
        <OptionGrid
          options={DISAPPEAR_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
          value={settings.disappear_default_secs}
          onChange={(v) => {
            set('set_disappear_default', { secs: v });
            setOpen(null);
          }}
        />
      </Sheet>
    </SheetHost>
  );
}

function LoraControls({ onChange }: { onChange: () => void }) {
  const { client, notify } = useApp();
  const lora = client.hooks.lora;
  const [radios, setRadios] = useState<{ address: string; name: string }[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [status, setStatus] = useState(() => lora?.status());
  const reread = () => {
    setStatus(lora?.status());
    onChange();
  };
  return (
    <View style={{ paddingBottom: 14, gap: 10 }}>
      <Text style={[type.small, { fontSize: 12.5 }]}>
        Pair a Meshtastic-compatible radio (licence-free ISM band). Short texts use it when Bluetooth has not delivered within 5 minutes. Set region and power on the radio.
      </Text>
      {lora ? (
        <>
          {radios?.map((r) => (
            <Row
              key={r.address}
              title={r.name}
              subtitle={r.address}
              icon="radio"
              onPress={async () => {
                try {
                  const ok = await lora.connect(r.address);
                  setRadios(null);
                  notify(ok ? `Connected to ${r.name}` : `Could not connect to ${r.name}`);
                } catch (e) {
                  notify(errorText(e));
                } finally {
                  reread();
                }
              }}
            />
          ))}
          {radios && radios.length === 0 ? <Text style={type.small}>No radios found. Turn the radio on and keep it close.</Text> : null}
          <Button
            title={searching ? 'Searching…' : 'Find radio'}
            variant="secondary"
            size="md"
            busy={searching}
            onPress={async () => {
              setSearching(true);
              try {
                setRadios(await lora.scan());
              } catch (e) {
                notify(errorText(e));
              } finally {
                setSearching(false);
              }
            }}
          />
          {status?.connected ? <Button title="Forget radio" variant="ghost" size="md" onPress={() => lora.forget().catch((e) => notify(errorText(e))).finally(reread)} /> : null}
        </>
      ) : null}
    </View>
  );
}

// ===== High-Risk confirm =====

// Short design copy (D:1575). The timer line differs from the design on purpose:
// High-Risk changes the default for new chats only, and timers start when a
// message is first shown.
const HR_ITEMS: { icon: IconName; t: string; d: string }[] = [
  { icon: 'zap', t: 'Constant cover traffic', d: 'Your radio looks equally busy whether you send or not.' },
  { icon: 'chat', t: 'Text only', d: 'Photos and files are blocked.' },
  { icon: 'radio', t: 'LoRa and Wi-Fi off', d: 'Bluetooth only, with slower relaying.' },
  { icon: 'timer', t: '24-hour disappearing', d: 'Messages in new chats vanish a day after they are shown.' },
  { icon: 'lock', t: 'Instant lock', d: 'The app locks the moment you leave it.' },
];

export function HighRisk() {
  const { call, pop, refresh, notify } = useApp();
  const confirm = async () => {
    try {
      await call('set_high_risk', { on: true });
      await refresh();
      pop();
      setTimeout(() => notify('High-Risk mode is on'), 300);
    } catch (e) {
      notify(errorText(e));
    }
  };
  return (
    <Screen
      dark
      onBack={pop}
      backIcon="x"
      footer={
        <View style={{ paddingTop: 10, paddingHorizontal: 18, paddingBottom: 30, gap: 8 }}>
          <Button title="Turn on High-Risk" label="Confirm: turn on High-Risk mode" variant="lime" style={{ height: 58 }} onPress={confirm} />
          <Pressable onPress={pop} accessibilityRole="button" accessibilityLabel="Not now" style={{ height: 48, alignItems: 'center', justifyContent: 'center' }}>
            <Text style={{ fontFamily: font.medium, fontSize: 15, color: '#fff' }}>Not now</Text>
          </Pressable>
        </View>
      }
      contentStyle={{ paddingHorizontal: 22, gap: 18 }}
    >
      <IconTile icon="zap" bg={color.lime} fg={color.ink} size={56} iconSize={28} />
      <Text style={[type.display, { color: '#fff' }]} accessibilityRole="header">
        Turn on High-Risk mode?
      </Text>
      <View style={{ gap: 14 }}>
        {HR_ITEMS.map((it, i) => (
          <Appear key={it.t} kind="up" delay={i * 70} style={{ flexDirection: 'row', gap: 12, alignItems: 'flex-start' }}>
            <IconTile icon={it.icon} bg="rgba(255,255,255,0.1)" fg={color.lime} size={36} iconSize={18} />
            <View style={{ flex: 1, gap: 2 }}>
              <Text style={{ fontFamily: font.semi, fontSize: 15, color: '#fff' }}>{it.t}</Text>
              <Text style={{ fontFamily: font.regular, fontSize: 13, lineHeight: 18, color: color.onInkMuted }}>{it.d}</Text>
            </View>
          </Appear>
        ))}
      </View>
      <View style={{ borderWidth: 1.5, borderStyle: 'dashed', borderColor: 'rgba(255,255,255,0.35)', borderRadius: 20, padding: 14, flexDirection: 'row', gap: 10 }}>
        <Icon name="alert" size={18} tint="#FFB38A" />
        <View style={{ flex: 1, gap: 3 }}>
          <Text style={{ fontFamily: font.semi, fontSize: 14, color: '#fff' }}>What it can’t do</Text>
          <Text style={{ fontFamily: font.regular, fontSize: 13, lineHeight: 18, color: 'rgba(255,255,255,0.75)' }}>
            Your radio is still on. Someone with the right equipment can still tell roughly where a transmitting phone is.
          </Text>
        </View>
      </View>
    </Screen>
  );
}

// ===== Communities =====

export function Communities() {
  const { call, pop, client, refresh, notify } = useApp();
  const [list, setList] = useState<Community[]>([]);
  const [name, setName] = useState('');
  const [show, setShow] = useState<{ id: Hex; qr: QrMatrix } | null>(null);
  const [scan, setScan] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const load = () =>
    call<Community[]>('communities')
      .then(setList)
      .catch((e) => setError(errorText(e)));
  useEffect(() => {
    load();
    return () => client.hooks.showQr?.(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <Screen title="Communities" onBack={pop}>
      <Text style={[type.small, { fontSize: 14, lineHeight: 20, paddingHorizontal: 4 }]}>
        Members of a community find each other through Bluetooth names that change every 15 minutes, so outsiders cannot tell the app is running. Share the code only in person. Up to 4.
      </Text>
      <Eyebrow>{`Joined · ${list.length} of 4`}</Eyebrow>
      {list.length === 0 ? (
        <Card style={{ alignItems: 'center' }}>
          <Text style={type.small}>No communities yet.</Text>
        </Card>
      ) : (
        list.map((c) => (
          <View key={c.id} style={{ backgroundColor: '#fff', borderRadius: 20, paddingVertical: 10, paddingHorizontal: 12, flexDirection: 'row', alignItems: 'center', gap: 12 }}>
            <Avatar seed={c.id} size={40} kind="group" />
            <Text style={{ flex: 1, fontFamily: font.semi, fontSize: 15, color: color.ink }} numberOfLines={1}>
              {c.name}
            </Text>
            <Button
              title="Show code"
              size="sm"
              icon="qr"
              onPress={async () => {
                try {
                  const r = await call<{ qr: QrMatrix }>('community_qr', { community: c.id });
                  setShow({ id: c.id, qr: r.qr });
                  client.hooks.showQr?.(r.qr);
                } catch (e) {
                  notify(errorText(e));
                }
              }}
            />
            <Pressable
              onPress={async () => {
                try {
                  await call('leave_community', { community: c.id });
                  load();
                  refresh();
                } catch (e) {
                  notify(errorText(e));
                }
              }}
              accessibilityRole="button"
              accessibilityLabel={`Leave ${c.name}`}
              hitSlop={6}
              style={{ padding: 6 }}
            >
              <Icon name="logout" size={18} tint={color.danger} />
            </Pressable>
          </View>
        ))
      )}
      {show ? (
        <Appear kind="pop" style={{ alignItems: 'center', gap: 12 }}>
          <View style={{ width: 260, height: 260, borderRadius: 36, backgroundColor: '#fff', alignItems: 'center', justifyContent: 'center' }}>
            <QrView matrix={show.qr} size={216} />
          </View>
          <Button
            title="Hide code"
            variant="secondary"
            size="md"
            onPress={() => {
              setShow(null);
              client.hooks.showQr?.(null);
            }}
          />
        </Appear>
      ) : null}
      <Eyebrow>Create</Eyebrow>
      <View style={{ flexDirection: 'row', gap: 8, backgroundColor: '#fff', borderRadius: 22, paddingVertical: 6, paddingRight: 6, paddingLeft: 4, alignItems: 'center', borderWidth: 1.5, borderColor: color.line }}>
        <Field label="New community name" hideLabel value={name} onChangeText={setName} maxLength={64} placeholder="Community name" style={{ flex: 1, borderWidth: 0, height: 44 }} />
        <Button
          title="Create"
          label="Create community"
          size="md"
          disabled={!name.trim() || list.length >= 4}
          onPress={async () => {
            try {
              await call('create_community', { name: name.trim() });
              setName('');
              load();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        />
      </View>
      {scan ? (
        <View style={{ borderRadius: 30, overflow: 'hidden', backgroundColor: color.ink, padding: 8, gap: 8 }}>
          <QrCamera
            onCode={async (bytes) => {
              if (bytes.length < 36 || String.fromCharCode(...bytes.slice(0, 4)) !== 'MQC1') return;
              try {
                await call('join_community', { payload: b64encode(bytes) });
                setScan(false);
                load();
              } catch (e) {
                setError(errorText(e));
              }
            }}
          />
          <Button title="Cancel" variant="light" size="md" onPress={() => setScan(false)} />
        </View>
      ) : (
        <Button title="Join by scanning a code" variant="secondary" icon="camera" disabled={list.length >= 4} onPress={() => setScan(true)} />
      )}
      <ErrorText error={error} />
    </Screen>
  );
}

// ===== About =====

/** "AB:CD:EF:…" (32 bytes) as 16 groups of four hex digits, four groups a line. */
function fpLines(fp: string): string[] {
  const hex = fp.replace(/:/g, '');
  const groups = hex.match(/.{1,4}/g) ?? [];
  const lines: string[] = [];
  for (let i = 0; i < groups.length; i += 4) lines.push(groups.slice(i, i + 4).join(' '));
  return lines;
}

/**
 * The signing fingerprints of this app and of MeshGRID, as Android reports
 * them, to compare with the website. Honest about its limit: a tampered copy
 * could print anything here, so the real check is an independent app.
 */
function VerifyCard() {
  const { notify } = useApp();
  const [fps, setFps] = useState<{ app: string | null; grid: string | null } | null>(null);
  useEffect(() => {
    if (Platform.OS !== 'android') return;
    try {
      const n = meshCoreNative();
      setFps({ app: n.signingFingerprint('org.meshtrike.app'), grid: n.signingFingerprint('org.meshtrike.link') });
    } catch {
      setFps(null);
    }
  }, []);
  if (!fps) return null;
  const same = !!fps.app && fps.app === fps.grid;
  const block = (label: string, fp: string | null) => (
    <View style={{ gap: 4 }}>
      <Text style={{ fontFamily: font.semi, fontSize: 13, color: color.ink }}>{label}</Text>
      {fp ? (
        <View style={{ backgroundColor: color.chip, borderRadius: 12, padding: 10 }} accessible accessibilityLabel={`${label} fingerprint ${fp.replace(/:/g, ' ')}`}>
          {fpLines(fp).map((l) => (
            <Text key={l} selectable style={{ fontFamily: font.monoSemi, fontSize: 12.5, lineHeight: 18, color: color.ink, letterSpacing: 0.5 }}>
              {l}
            </Text>
          ))}
        </View>
      ) : (
        <Text style={type.small}>Not installed</Text>
      )}
    </View>
  );
  return (
    <>
      <Eyebrow>Verify this app</Eyebrow>
      <Card>
        <Text style={type.small}>
          Real Meshtrike and MeshGRID are signed by one key. Its fingerprint is published on the website, in the whitepaper and in every release. Compare it with these.
        </Text>
        {block('Meshtrike', fps.app)}
        {block('MeshGRID', fps.grid)}
        {fps.grid ? (
          <Text style={{ fontFamily: font.semi, fontSize: 12.5, color: same ? color.good : color.danger }}>
            {same ? 'Both apps carry the same key.' : 'MeshGRID is signed with a different key: Meshtrike will not use it.'}
          </Text>
        ) : null}
        <View style={{ flexDirection: 'row', gap: 10, alignItems: 'flex-start', backgroundColor: color.warnBg, borderRadius: 14, padding: 12 }}>
          <Icon name="alert" size={16} tint={color.warnIcon} />
          <Text style={{ flex: 1, fontFamily: font.regular, fontSize: 12.5, lineHeight: 18, color: color.inkSoft }}>
            A fake copy could show a fake number here. For a real check, use an independent app such as AppVerifier, or check the APK on a computer before installing.
          </Text>
        </View>
        <Button
          title="How to verify"
          variant="secondary"
          size="md"
          icon="ext"
          onPress={() => Linking.openURL(VERIFY_URL).catch(() => notify('Could not open the browser'))}
        />
      </Card>
    </>
  );
}

const LIMITS = [
  'Radio transmissions can be located by direction-finding equipment; the countermeasures reduce this, they do not remove it.',
  'In Public mode the fixed Bluetooth service ID shows that a phone runs this app. Community mode hides it from non-members only.',
  'A powerful wideband jammer stops all radio nearby; only QR handoff and moving away get messages through.',
  'Spyware with kernel access on an unlocked phone with the app open can read the screen.',
  'iPhones restrict background Bluetooth, so relaying is weaker when the app is not open.',
  'iPhones offer no transmit-power control.',
  'Fast file transfer between iPhone and Android is not available.',
  'A malicious relay can drop messages; it cannot read or change them.',
  'Anyone can create fake identities; trust comes only from verifying in person.',
  'Public channels are readable by anyone who knows the channel name.',
  'App Store builds cannot be reproduced byte for byte.',
];

export function About() {
  const { pop, client, call, notify } = useApp();
  const [log, setLog] = useState<string | null>(null);
  const [version, setVersion] = useState<{ core: string; protocol: number } | null>(null);
  useEffect(() => {
    call<{ core: string; protocol: number }>('version').then(setVersion).catch(() => setVersion(null));
  }, [call]);
  return (
    <Screen title="About & limits" onBack={pop}>
      <Card>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }}>
          <IconTile icon="mesh" bg={color.lime} size={44} iconSize={22} />
          <View style={{ gap: 2 }}>
            <Text style={{ fontFamily: font.display, fontSize: 22, color: color.ink }}>Meshtrike {appJson.expo.version}</Text>
            <Text style={{ fontFamily: font.mono, fontSize: 11.5, color: color.textMuted }}>
              {version ? `App ${appJson.expo.version} · core ${version.core} · protocol ${version.protocol}` : `App ${appJson.expo.version}`}
            </Text>
          </View>
        </View>
        <Text style={type.small}>{client.hooks.platformLabel}</Text>
        <Text style={type.small}>Open source under Apache-2.0. No analytics, crash reporting or network access.</Text>
        <Text style={[type.tiny, { lineHeight: 16 }]}>
          Uses vodozemac (Olm/Megolm), snow (Noise), RustCrypto, SQLCipher, zune-jpeg, jpeg-encoder, raptorq, qrcode and rqrr. This software is based in part on the work of the Independent JPEG Group.
        </Text>
      </Card>
      <VerifyCard />
      <Eyebrow>Debug log</Eyebrow>
      <Card>
        <Text style={type.small}>Kept in memory only (last 500 lines). It never contains names, keys or message content. Review it before sharing it any way you choose.</Text>
        {log === null ? (
          <Button title="Show debug log" variant="secondary" size="md" onPress={() => call<string>('debug_log').then(setLog).catch((e) => notify(errorText(e)))} />
        ) : (
          <>
            <View style={{ backgroundColor: color.chip, borderRadius: 14, padding: 12 }}>
              <Text style={{ fontFamily: font.mono, fontSize: 11, lineHeight: 16, color: color.ink }} selectable accessibilityLabel="Debug log text">
                {log || '(empty)'}
              </Text>
            </View>
            {client.hooks.saveFile ? (
              <Button title="Save log to a file" variant="secondary" size="md" icon="download" onPress={() => client.hooks.saveFile?.('meshtrike-debug.txt', new TextEncoder().encode(log))} />
            ) : null}
            <Button title="Hide" variant="ghost" size="md" onPress={() => setLog(null)} />
          </>
        )}
      </Card>
      <Eyebrow>Known limits</Eyebrow>
      <Section>
        {LIMITS.map((l, i) => (
          <View key={l} style={{ paddingVertical: 12, flexDirection: 'row', gap: 10, borderTopWidth: i ? 1 : 0, borderTopColor: color.lineSoft }}>
            <Icon name="alert" size={16} tint={color.warnIcon} />
            <Text style={{ flex: 1, fontFamily: font.regular, fontSize: 14, lineHeight: 20, color: color.ink }}>{l}</Text>
          </View>
        ))}
      </Section>
    </Screen>
  );
}

/** Internet add-on state on Android (null elsewhere: the harness simulates the route). */
/** The add-on, and how its route is connected ("bridge" or "direct"). */
type AddonStatus = { installed: boolean; mode?: 'bridge' | 'direct' | null };

function useInternetAddon(): AddonStatus | null {
  const [st, setSt] = useState<AddonStatus | null>(null);
  useEffect(() => {
    if (Platform.OS !== 'android') return;
    const read = () => {
      try {
        setSt(meshCoreNative().internetAddonStatus());
      } catch {
        setSt({ installed: false });
      }
    };
    read();
    const t = setInterval(read, 3000);
    return () => clearInterval(t);
  }, []);
  return st;
}

/**
 * Internet route (amendment N1): the same sealed messages, also sent through
 * the Nym mixnet to contacts who are out of range. The add-on (Android) runs
 * the network side; Meshtrike itself never connects to the internet. For now
 * the route connects to the mixnet directly: adding disguise bridges is shown
 * as coming soon (bridges already on the phone stay listed so they can be
 * removed).
 */
export function InternetRoute() {
  const { call, pop, client, refresh, notify, network, settings } = useApp();
  const [bridges, setBridges] = useState<Bridge[]>([]);
  const [show, setShow] = useState<{ id: string; qr: QrMatrix } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const addon = useInternetAddon();
  const load = () =>
    call<Bridge[]>('bridges')
      .then(setBridges)
      .catch((e) => setError(errorText(e)));
  useEffect(() => {
    load();
    return () => client.hooks.showQr?.(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const hr = !!settings?.high_risk;
  const on = !!network?.internet_on;
  const online = !!network?.internet_online;
  const direct = !!network?.internet_direct;
  const needsBridge = on && bridges.length === 0 && !direct;
  const missingAddon = addon !== null && !addon.installed;
  // Design order (D:1594): a missing add-on shows before the switch's state.
  const state = hr
    ? { title: 'Off in High-Risk', text: 'High-Risk keeps every message on the mesh.', tone: 'muted' as const }
    : missingAddon
      ? { title: 'Add-on needed', text: 'Install the MeshGRID add-on. Meshtrike itself never connects to the internet.', tone: 'warn' as const }
      : !on
        ? { title: 'Off', text: 'Messages travel only phone to phone.', tone: 'muted' as const }
        : needsBridge
          ? { title: 'Connecting…', text: 'Switching to a direct connection to the mixnet.', tone: 'muted' as const }
          : online
            ? {
                title: 'Connected',
                text:
                  addon?.mode === 'bridge'
                    ? 'Through the mixnet, disguised by your bridge.'
                    : bridges.length && addon?.mode === 'direct'
                      ? 'Through the mixnet, directly: your bridge cannot be reached from this network. It is used again once it can.'
                      : 'Through the mixnet, without a disguise.',
                tone: 'good' as const,
              }
            : { title: 'Connecting…', text: 'Waiting for the internet and the mixnet.', tone: 'muted' as const };
  const toneColors = state.tone === 'good' ? { fg: color.good, bg: color.goodBg } : state.tone === 'warn' ? { fg: color.warnIcon, bg: color.warnBg } : { fg: color.grey, bg: '#EDEBE6' };
  const connecting = state.title === 'Connecting…';
  const setInternet = async (v: boolean) => {
    try {
      if (v) await call('set_internet_direct', { on: true });
      await call('set_internet', { on: v });
      refresh();
    } catch (e) {
      notify(errorText(e));
    }
  };
  // Until bridges can be added here, the route connects to the mixnet
  // directly; switch that on for phones that turned the route on earlier.
  useEffect(() => {
    if (on && !direct && !hr) {
      call('set_internet_direct', { on: true })
        .then(() => refresh())
        .catch((e) => setError(errorText(e)));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [on, direct, hr]);
  return (
    <Screen title="Internet route" onBack={pop}>
      <View style={[st.inetStatus, { backgroundColor: toneColors.bg }]} accessibilityLiveRegion="polite">
        <View style={st.inetStatusIcon}>
          <Icon name={state.tone === 'warn' ? 'alert' : 'globe'} size={20} tint={toneColors.fg} />
        </View>
        <View style={{ flex: 1, gap: 4, minWidth: 0 }}>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
            <Text style={{ fontFamily: font.display, fontSize: 18, color: toneColors.fg }} accessibilityRole="header">
              {state.title}
            </Text>
            {connecting ? <SmallSpinner /> : null}
          </View>
          <Text style={{ fontFamily: font.regular, fontSize: 13.5, lineHeight: 19.5, color: color.inkSoft }}>{state.text}</Text>
          {missingAddon && !hr ? (
            <Pressable onPress={reachUi.open} accessibilityRole="button" style={({ pressed }) => [st.getReach, pressed ? { opacity: 0.85 } : null]}>
              <Text style={{ fontFamily: font.semi, fontSize: 13, color: '#fff' }}>Get {REACH}</Text>
            </Pressable>
          ) : null}
        </View>
      </View>
      <Pressable
        onPress={hr ? undefined : () => setInternet(!on)}
        disabled={hr}
        accessibilityRole="button"
        accessibilityLabel={`Use the internet when available, ${!hr && on ? 'on' : 'off'}`}
        style={({ pressed }) => [st.inetToggle, pressed && !hr ? { opacity: 0.6 } : null]}
      >
        <View style={{ flex: 1, gap: 2 }}>
          <Text style={{ fontFamily: font.semi, fontSize: 15, color: color.ink }}>Use the internet when available</Text>
          <Text style={{ fontFamily: font.regular, fontSize: 12.5, lineHeight: 17.5, color: color.textMuted }}>
            {hr ? 'Never in High-Risk' : 'Reach contacts who are far away. Off by default.'}
          </Text>
        </View>
        <Toggle value={!hr && on} onChange={setInternet} disabled={hr} label="Use the internet when available" />
      </Pressable>
      <Text style={{ fontFamily: font.regular, fontSize: 13.5, lineHeight: 21, color: color.inkSoft, paddingHorizontal: 4 }}>
        Messages stay sealed end to end. The phone connects straight to the Nym mixnet, which hides who talks to whom and when. Anyone watching your network can see that you use the mixnet, not who you talk to. No Meshtrike server exists.
      </Text>
      {bridges.length ? <Eyebrow>{`Bridges · ${bridges.length}`}</Eyebrow> : null}
      {bridges.map((b) => (
          <View key={b.id} style={st.bridge}>
            <IconTile icon="shield" bg={color.chip2} size={40} iconSize={19} />
            <View style={{ flex: 1, gap: 3, minWidth: 0 }}>
              <Text style={{ fontFamily: font.monoSemi, fontSize: 14, color: color.ink }} numberOfLines={1}>
                {b.host}:{b.port}
              </Text>
              <Text style={{ fontFamily: font.regular, fontSize: 12, color: color.textMuted }} numberOfLines={1}>
                Looks like {b.disguise}
                {b.from_contact ? ' · from a contact' : ''}
              </Text>
            </View>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Show bridge ${b.host} as QR`}
              hitSlop={4}
              style={({ pressed }) => [st.bridgeAction, pressed ? { opacity: 0.6 } : null]}
              onPress={async () => {
                try {
                  const r = await call<{ qr: QrMatrix }>('bridge_qr', { id: b.id });
                  setShow({ id: b.id, qr: r.qr });
                  client.hooks.showQr?.(r.qr);
                } catch (e) {
                  notify(errorText(e));
                }
              }}
            >
              <Icon name="qr" size={18} />
            </Pressable>
            <Pressable
              onPress={async () => {
                try {
                  await call('remove_bridge', { id: b.id });
                  if (show?.id === b.id) {
                    setShow(null);
                    client.hooks.showQr?.(null);
                  }
                  load();
                  refresh();
                } catch (e) {
                  notify(errorText(e));
                }
              }}
              accessibilityRole="button"
              accessibilityLabel={`Remove bridge ${b.host}`}
              hitSlop={4}
              style={({ pressed }) => [st.bridgeAction, pressed ? { opacity: 0.6 } : null]}
            >
              <Icon name="trash" size={18} tint={color.danger} />
            </Pressable>
          </View>
        ))}
      {show ? (
        <Appear kind="pop" style={{ alignItems: 'center', gap: 12 }}>
          <View style={{ width: 260, height: 260, borderRadius: 36, backgroundColor: '#fff', alignItems: 'center', justifyContent: 'center' }}>
            <QrView matrix={show.qr} size={216} />
          </View>
          <Text style={[type.small, { textAlign: 'center' }]}>Show this only to people you trust: whoever has it can use the bridge.</Text>
          <Button
            title="Hide code"
            variant="secondary"
            size="md"
            onPress={() => {
              setShow(null);
              client.hooks.showQr?.(null);
            }}
          />
        </Appear>
      ) : null}
      <View style={st.soon}>
        <IconTile icon="shield" bg={color.chip} fg={color.textFaint} size={40} iconSize={19} />
        <View style={{ flex: 1, gap: 3 }}>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
            <Text style={{ fontFamily: font.semi, fontSize: 14.5, color: color.inkSoft }}>Disguise bridges</Text>
            <View style={st.soonBadge}>
              <Text style={{ fontFamily: font.monoSemi, fontSize: 10.5, letterSpacing: 0.4, color: color.grey }}>COMING SOON</Text>
            </View>
          </View>
          <Text style={{ fontFamily: font.regular, fontSize: 12.5, lineHeight: 17.5, color: color.textMuted }}>
            Will make the connection look like a visit to an ordinary website.
          </Text>
        </View>
      </View>
      <ErrorText error={error} />
    </Screen>
  );
}

/** What the internet route is really doing, from core state and the add-on (Settings value, design D:1573). */
function netStatus(n: { internet_on: boolean; internet_online: boolean } | null | undefined, highRisk: boolean, addon: { installed: boolean } | null): string {
  if (highRisk) return 'Off in High-Risk';
  if (addon && !addon.installed) return 'Add-on needed';
  if (!n?.internet_on) return 'Off';
  return n.internet_online ? 'Connected' : 'Connecting…';
}

/** 14 px spinner beside "Connecting…" (design: 2 px ring, #4E525B top). */
function SmallSpinner() {
  const v = useLoop(800);
  const rotate = v.interpolate({ inputRange: [0, 1], outputRange: ['0deg', '360deg'] });
  return <Animated.View accessibilityElementsHidden importantForAccessibility="no" style={[st.spinner, { transform: [{ rotate }] }]} />;
}

const st = StyleSheet.create({
  getReach: { alignSelf: 'flex-start', height: 36, borderRadius: 99, backgroundColor: color.ink, paddingHorizontal: 14, justifyContent: 'center', marginTop: 6 },
  host: { flex: 1 },
  // Above the floating tab bar (zIndex 20, elevation 10 in MeshApp).
  hostRaised: { zIndex: 30, elevation: 30 },
  stat: { flexGrow: 1, flexBasis: '45%', backgroundColor: '#fff', borderRadius: 20, paddingVertical: 14, paddingHorizontal: 16 },
  statInfo: { position: 'absolute', right: 8, top: 8, width: 32, height: 32, borderRadius: 16, backgroundColor: color.chip, alignItems: 'center', justifyContent: 'center' },
  sheetTitle: { fontFamily: font.display, fontSize: 22, lineHeight: 25, color: color.ink, paddingHorizontal: 4 },
  sheetBody: { fontFamily: font.regular, fontSize: 14, lineHeight: 21, color: color.textMuted, paddingHorizontal: 4 },
  sendQr: {
    alignSelf: 'center',
    width: 290,
    height: 290,
    borderRadius: 36,
    backgroundColor: '#fff',
    alignItems: 'center',
    justifyContent: 'center',
    // Design: 0 10px 30px -14px rgba(0,0,0,.25)
    shadowColor: '#000',
    shadowOpacity: 0.25,
    shadowRadius: 15,
    shadowOffset: { width: 0, height: 10 },
    elevation: 4,
  },
  viewfinder: { alignSelf: 'center', width: 290, height: 290, borderRadius: 36, overflow: 'hidden', backgroundColor: color.ink },
  viewfinderHarness: { borderRadius: 36, overflow: 'hidden', backgroundColor: color.ink, padding: 8 },
  section: { paddingVertical: 4 },
  inetStatus: { borderRadius: 22, padding: 16, flexDirection: 'row', alignItems: 'flex-start', gap: 12 },
  inetStatusIcon: { width: 42, height: 42, borderRadius: 14, backgroundColor: '#fff', alignItems: 'center', justifyContent: 'center' },
  inetToggle: { backgroundColor: '#fff', borderRadius: 22, paddingVertical: 14, paddingHorizontal: 16, flexDirection: 'row', alignItems: 'center', gap: 12 },
  bridge: { backgroundColor: '#fff', borderRadius: 22, paddingTop: 12, paddingBottom: 12, paddingLeft: 14, paddingRight: 10, flexDirection: 'row', alignItems: 'center', gap: 12 },
  bridgeAction: { width: 40, height: 40, borderRadius: 13, backgroundColor: color.chip, alignItems: 'center', justifyContent: 'center' },
  soon: { borderWidth: 1.5, borderStyle: 'dashed', borderColor: color.dash, borderRadius: 22, padding: 14, flexDirection: 'row', alignItems: 'flex-start', gap: 12 },
  soonBadge: { paddingVertical: 3, paddingHorizontal: 7, borderRadius: 6, backgroundColor: color.chip2 },
  spinner: { width: 14, height: 14, borderRadius: 7, borderWidth: 2, borderColor: 'rgba(0,0,0,0.12)', borderTopColor: color.grey },
});

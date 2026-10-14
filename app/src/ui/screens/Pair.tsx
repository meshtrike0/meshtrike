import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Animated, Easing, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import Svg, { Circle, Defs, RadialGradient, Rect, Stop } from 'react-native-svg';
import { b64encode } from '../../core/client';
import { Hex, PairingOutcome, QrMatrix } from '../../core/types';
import { QrCamera } from '../Camera';
import { Icon } from '../icons';
import { Appear, Avatar, Button, ErrorText, IconTile, native, QrView, Screen, TrustBadge, useLoop } from '../kit';
import { errorText, useApp, useCoreEvents } from '../state';
import { color, font, type } from '../theme';

type Result = { contact: Hex; nickname: string; verified: boolean } | { failed: string };

const CODE_SECS = 120;

export function Pair({ tab: initial = 'show' }: { tab?: 'show' | 'scan' }) {
  const { pop } = useApp();
  const [tab, setTab] = useState<'show' | 'scan'>(initial);
  const [result, setResult] = useState<Result | null>(null);
  const [round, setRound] = useState(0);
  const again = () => {
    setResult(null);
    setRound((r) => r + 1);
  };
  return (
    <Screen title="Add in person" onBack={pop} scroll={false}>
      {/* Tabs stay visible in every state, as in the design; picking one starts that side afresh. */}
      <PairTabs
        value={tab}
        onChange={(t) => {
          setTab(t);
          if (result) again();
        }}
      />
      <ScrollView style={{ flex: 1 }} contentContainerStyle={st.body} keyboardShouldPersistTaps="handled" showsVerticalScrollIndicator={false}>
        {result ? (
          <PairResult result={result} onAgain={again} />
        ) : tab === 'show' ? (
          <ShowCode key={`show${round}`} onResult={setResult} />
        ) : (
          <ScanCode key={`scan${round}`} onResult={setResult} />
        )}
      </ScrollView>
    </Screen>
  );
}

/** Pair-specific segmented control: sand track, 42-tall tabs. */
function PairTabs({ value, onChange }: { value: 'show' | 'scan'; onChange: (v: 'show' | 'scan') => void }) {
  const options: { value: 'show' | 'scan'; label: string }[] = [
    { value: 'show', label: 'Show my code' },
    { value: 'scan', label: 'Scan code' },
  ];
  return (
    <View style={st.tabs} accessibilityRole="radiogroup">
      {options.map((o) => {
        const on = o.value === value;
        return (
          <Pressable
            key={o.value}
            onPress={() => onChange(o.value)}
            accessibilityRole="radio"
            accessibilityState={{ checked: on }}
            aria-checked={on}
            accessibilityLabel={o.label}
            style={[st.tab, on ? st.tabOn : null]}
          >
            <Text style={{ fontFamily: font.semi, fontSize: 14, color: color.ink }}>{o.label}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

function usePairingResult(onResult: (r: Result) => void, pairing: Hex | null) {
  useCoreEvents(
    (e) => {
      if (e.event !== 'PairingResult') return;
      const o: PairingOutcome = e.outcome;
      if (o.outcome === 'Paired' && (pairing === null || e.pairing === pairing)) onResult(o.detail);
      if (o.outcome === 'Failed' && (pairing === null || e.pairing === pairing)) onResult({ failed: o.detail });
    },
    [pairing, onResult],
  );
}

/** QR inside a ring that drains over the code's 2-minute life. */
function CountdownRing({ left, size, children }: { left: number; size: number; children: React.ReactNode }) {
  const r = size / 2 - 8;
  const ring = 2 * Math.PI * r;
  return (
    <View style={{ width: size, height: size, alignItems: 'center', justifyContent: 'center' }}>
      <View style={[StyleSheet.absoluteFill, { transform: [{ rotate: '-90deg' }] }]}>
        <Svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
          <Circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke={color.chip2} strokeWidth={6} />
          <Circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke={color.ink}
            strokeWidth={6}
            strokeLinecap="round"
            strokeDasharray={`${ring}`}
            strokeDashoffset={ring * (1 - left / CODE_SECS)}
          />
        </Svg>
      </View>
      {children}
    </View>
  );
}

function ShowCode({ onResult }: { onResult: (r: Result) => void }) {
  const { call, client } = useApp();
  const [qr, setQr] = useState<QrMatrix | null>(null);
  const [left, setLeft] = useState(CODE_SECS);
  const [error, setError] = useState<string | null>(null);
  const make = useCallback(async () => {
    try {
      const r = await call<{ payload: string; qr: QrMatrix }>('pairing_qr');
      setQr(r.qr);
      setLeft(CODE_SECS);
      client.hooks.showQr?.(r.qr);
    } catch (e) {
      setError(errorText(e));
    }
  }, [call, client]);
  useEffect(() => {
    make();
    return () => client.hooks.showQr?.(null);
  }, [make, client]);
  useEffect(() => {
    const t = setInterval(() => setLeft((l) => Math.max(0, l - 1)), 1000);
    return () => clearInterval(t);
  }, []);
  useEffect(() => {
    if (left === 0) client.hooks.showQr?.(null);
  }, [left, client]);
  usePairingResult(onResult, null);

  if (left === 0) {
    return (
      <Appear kind="up" style={st.state}>
        <IconTile icon="timer" bg={color.chip2} size={64} iconSize={30} />
        <Text style={st.stateTitle}>Code expired</Text>
        <Text style={[st.stateText, { maxWidth: 260 }]}>Codes last 2 minutes. Make a fresh one when you’re both ready.</Text>
        <Button title="New code" size="md" onPress={make} style={st.stateBtn} />
      </Appear>
    );
  }
  return (
    <>
      <Appear kind="in">
        <CountdownRing left={left} size={316}>
          <View style={st.qrCard}>{qr ? <QrView matrix={qr} size={210} /> : null}</View>
        </CountdownRing>
      </Appear>
      <View style={{ alignItems: 'center', gap: 4 }}>
        <Text style={{ fontFamily: font.monoSemi, fontSize: 26, letterSpacing: 0.5, color: color.ink }} accessibilityLiveRegion="polite" accessibilityLabel={`Code expires in ${left} seconds`}>
          {`${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`}
        </Text>
        <Text style={[st.stateText, { fontSize: 13.5, lineHeight: 19, maxWidth: 300 }]}>Let the other person scan this. It expires so nobody can reuse it.</Text>
      </View>
      <ErrorText error={error} />
    </>
  );
}

/** Two phones exchanging keys: a lime dot travels the dashed link. */
function Connecting() {
  const v = useLoop(1200);
  // A visible clock, so progress shows even when animations are off.
  const [secs, setSecs] = useState(0);
  useEffect(() => {
    const t = setInterval(() => setSecs((s) => s + 1), 1000);
    return () => clearInterval(t);
  }, []);
  // There and back along the 60px link, easing at each phone.
  const x = v.interpolate({ inputRange: [0, 0.5, 1], outputRange: [0, 60, 0], easing: Easing.inOut(Easing.ease) });
  return (
    <Appear kind="in" style={[st.state, { gap: 24 }]}>
      <View style={{ width: 200, height: 120 }}>
        <View style={[st.phone, { left: 0, backgroundColor: '#fff' }]}>
          <Icon name="phone" size={26} />
        </View>
        <View style={[st.phone, { right: 0, backgroundColor: color.ink }]}>
          <Icon name="phone" size={26} tint={color.lime} />
        </View>
        <DashedLink />
        <Animated.View style={[st.travel, { transform: [{ translateX: x }] }]}>
          <View style={st.travelCore} />
        </Animated.View>
      </View>
      <View style={{ alignItems: 'center', gap: 6 }}>
        <Text style={st.stateTitle}>Exchanging keys…</Text>
        <Text style={st.stateText}>Keep both phones close for a few seconds.</Text>
        <Text style={{ fontFamily: font.monoSemi, fontSize: 15, color: color.ink, marginTop: 4 }} accessibilityLiveRegion="polite">
          {`${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, '0')}`}
        </Text>
      </View>
    </Appear>
  );
}

/** 2px ink link drawn as 4px-on / 5px-off segments (borderStyle dashed renders unevenly on Android). */
function DashedLink() {
  const width = 64;
  const n = Math.ceil(width / 9);
  return (
    <View style={st.link}>
      {Array.from({ length: n }, (_, i) => (
        <View key={i} style={{ position: 'absolute', left: i * 9, top: 0, width: Math.min(4, width - i * 9), height: 2, backgroundColor: color.ink }} />
      ))}
    </View>
  );
}

/** Dark viewfinder with lime corner brackets and a sweeping scan line. */
function Viewfinder({ children }: { children: React.ReactNode }) {
  const v = useLoop(2400);
  // Design keyframes: 14% → 82% of the 200px bracket box.
  const top = v.interpolate({ inputRange: [0, 0.5, 1], outputRange: [28, 164, 28], easing: Easing.inOut(Easing.ease) });
  return (
    <View style={st.finder}>
      <Svg pointerEvents="none" style={StyleSheet.absoluteFill} width={316} height={380}>
        <Defs>
          <RadialGradient id="finderBg" gradientUnits="userSpaceOnUse" cx={158} cy={114} r={379} rx={379} ry={304} fx={158} fy={114}>
            <Stop offset="0" stopColor="#3A3F38" />
            <Stop offset="1" stopColor={color.ink} />
          </RadialGradient>
        </Defs>
        <Rect x={0} y={0} width={316} height={380} fill="url(#finderBg)" />
      </Svg>
      {children}
      <View pointerEvents="none" style={st.brackets}>
        <View style={[st.corner, { left: 0, top: 0, borderLeftWidth: 4, borderTopWidth: 4, borderTopLeftRadius: 18 }]} />
        <View style={[st.corner, { right: 0, top: 0, borderRightWidth: 4, borderTopWidth: 4, borderTopRightRadius: 18 }]} />
        <View style={[st.corner, { left: 0, bottom: 0, borderLeftWidth: 4, borderBottomWidth: 4, borderBottomLeftRadius: 18 }]} />
        <View style={[st.corner, { right: 0, bottom: 0, borderRightWidth: 4, borderBottomWidth: 4, borderBottomRightRadius: 18 }]} />
        <Animated.View style={[st.scanLine, { transform: [{ translateY: top }] }]} />
      </View>
      <Text pointerEvents="none" style={st.finderText}>
        Point at the other phone’s code
      </Text>
    </View>
  );
}

function ScanCode({ onResult }: { onResult: (r: Result) => void }) {
  const { call } = useApp();
  const [pairing, setPairing] = useState<Hex | null>(null);
  const [error, setError] = useState<string | null>(null);
  const started = useRef(false);
  usePairingResult(onResult, pairing);
  useEffect(() => {
    if (!pairing) return;
    const t = setTimeout(() => onResult({ failed: 'No answer within 2 minutes. Stay close and try again.' }), 125_000);
    return () => clearTimeout(t);
  }, [pairing, onResult]);
  const onCode = async (bytes: Uint8Array) => {
    if (started.current || bytes.length !== 97) return;
    started.current = true;
    try {
      setPairing(await call<Hex>('pair_from_qr', { payload: b64encode(bytes) }));
    } catch (e) {
      started.current = false;
      setError(errorText(e));
    }
  };
  if (pairing) return <Connecting />;
  return (
    <>
      {Platform.OS === 'web' ? (
        // Browser harness: the simulated camera picks which phone's screen it films.
        <View style={{ alignSelf: 'stretch' }}>
          <QrCamera onCode={onCode} />
        </View>
      ) : (
        <Viewfinder>
          <QrCamera onCode={onCode} />
        </Viewfinder>
      )}
      <ErrorText error={error} />
    </>
  );
}

function PairResult({ result, onAgain }: { result: Result; onAgain: () => void }) {
  const { contacts, reset, push } = useApp();
  const check = useRef(new Animated.Value(0)).current;
  useEffect(() => {
    Animated.spring(check, { toValue: 1, friction: 5, tension: 120, delay: 200, useNativeDriver: native }).start();
  }, [check]);
  if ('failed' in result) {
    return (
      <Appear kind="up" style={st.state}>
        <IconTile icon="alert" bg={color.dangerBg} fg={color.danger} size={64} iconSize={30} />
        <Text style={st.stateTitle}>Pairing didn’t finish</Text>
        {/* The core's real reason, in the design's "reason. Stand closer and try again." shape. */}
        <Text style={[st.stateText, { maxWidth: 270 }]}>{result.failed.replace(/\.$/, '')}. Stand closer and try again.</Text>
        <Text style={st.stateNote}>Nothing was saved.</Text>
        <Button title="Try again" size="md" onPress={onAgain} style={st.stateBtn} />
      </Appear>
    );
  }
  const c = contacts.find((x) => x.id === result.contact);
  return (
    <Appear kind="pop" style={[st.state, { gap: 16 }]}>
      <View>
        <Avatar seed={c?.identity_key ?? result.contact} size={88} />
        <Animated.View style={[st.check, { transform: [{ scale: check }] }]}>
          <Icon name="check" size={18} />
        </Animated.View>
      </View>
      <Text style={{ fontFamily: font.display, fontSize: 28, letterSpacing: -0.5, color: color.ink, textAlign: 'center' }}>{result.nickname}</Text>
      <TrustBadge trust={result.verified ? 'Verified' : 'Unverified'} large />
      <Text style={[st.stateText, { maxWidth: 260 }]}>
        {result.verified
          ? 'Paired in person. Messages between you are sealed with keys only your two phones hold.'
          : `Paired with ${result.nickname}. Compare safety numbers in person to verify.`}
      </Text>
      {c?.conversation ? (
        <Button
          title="Open chat"
          onPress={() => {
            reset({ name: 'Chats' });
            push({ name: 'Chat', conv: c.conversation! });
          }}
          style={{ marginTop: 10, paddingHorizontal: 30 }}
        />
      ) : (
        // No chat to open yet: give a way out besides the header back button.
        <Button title="Done" size="md" onPress={() => reset({ name: 'Chats' })} style={[st.stateBtn, { marginTop: 10 }]} />
      )}
    </Appear>
  );
}

const st = StyleSheet.create({
  body: { flexGrow: 1, alignItems: 'center', gap: 18, paddingTop: 22, paddingHorizontal: 22, paddingBottom: 34 },
  tabs: { flexDirection: 'row', marginHorizontal: 20, backgroundColor: color.sand, borderRadius: 16, padding: 4 },
  tab: { flex: 1, height: 42, borderRadius: 12, alignItems: 'center', justifyContent: 'center', paddingHorizontal: 6 },
  tabOn: { backgroundColor: '#fff', shadowColor: '#000', shadowOpacity: 0.12, shadowRadius: 3, shadowOffset: { width: 0, height: 1 }, elevation: 1 },
  // Result states sit in the middle of the free space.
  state: { flex: 1, alignItems: 'center', justifyContent: 'center', gap: 14, alignSelf: 'stretch' },
  stateTitle: { fontFamily: font.display, fontSize: 22, color: color.ink, textAlign: 'center' },
  stateText: { ...type.small, fontSize: 14, lineHeight: 20, textAlign: 'center', maxWidth: 280 },
  stateNote: { fontFamily: font.regular, fontSize: 12, lineHeight: 17, color: color.textFaint, textAlign: 'center', marginTop: -6 },
  stateBtn: { height: 52, paddingHorizontal: 26 },
  qrCard: {
    width: 250,
    height: 250,
    borderRadius: 40,
    backgroundColor: '#fff',
    alignItems: 'center',
    justifyContent: 'center',
    shadowColor: '#000',
    shadowOpacity: 0.25,
    shadowRadius: 15,
    shadowOffset: { width: 0, height: 10 },
    elevation: 5,
  },
  phone: { position: 'absolute', top: 30, width: 60, height: 60, borderRadius: 20, alignItems: 'center', justifyContent: 'center' },
  link: { position: 'absolute', left: 68, width: 64, top: 59, height: 2, overflow: 'hidden' },
  // 24px halo around a 16px lime core; the core starts at x=70 (design offset-path M70→130).
  travel: {
    position: 'absolute',
    left: 66,
    top: 48,
    width: 24,
    height: 24,
    borderRadius: 12,
    backgroundColor: color.limeGlow,
    alignItems: 'center',
    justifyContent: 'center',
  },
  travelCore: { width: 16, height: 16, borderRadius: 8, backgroundColor: color.lime },
  check: {
    position: 'absolute',
    right: -8,
    bottom: -8,
    width: 36,
    height: 36,
    borderRadius: 18,
    backgroundColor: color.lime,
    borderWidth: 4,
    borderColor: color.bg,
    alignItems: 'center',
    justifyContent: 'center',
  },
  finder: { width: 316, height: 380, borderRadius: 36, overflow: 'hidden', backgroundColor: color.ink, justifyContent: 'center' },
  brackets: { position: 'absolute', left: 58, top: 80, width: 200, height: 200 },
  corner: { position: 'absolute', width: 40, height: 40, borderColor: color.lime },
  scanLine: {
    position: 'absolute',
    left: 10,
    right: 10,
    top: 0,
    height: 2,
    backgroundColor: color.lime,
    shadowColor: color.lime,
    shadowOpacity: 0.8,
    shadowRadius: 10,
    elevation: 4,
  },
  finderText: { position: 'absolute', left: 0, right: 0, bottom: 22, textAlign: 'center', color: '#fff', fontFamily: font.medium, fontSize: 14 },
});


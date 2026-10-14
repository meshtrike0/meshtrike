import React, { useEffect, useRef, useState } from 'react';
import { AccessibilityInfo, Animated, AppState, Easing, Linking, Platform, Pressable, ScrollView, Text, TextInput, View } from 'react-native';
import Svg, { Circle, Path } from 'react-native-svg';
import { meshCoreNative } from '../../../modules/mesh-core/src/MeshCoreModule';
import { UnlockResult } from '../../core/types';
import { Icon, IconName } from '../icons';
import { Appear, Button, ErrorText, Field, IconTile, native, PinPad, Spinner, Toggle, useReduceMotion } from '../kit';
import { errorText, useApp } from '../state';
import { color, font, type } from '../theme';

// ===== Welcome =====

const CARDS: [string, string][] = [
  ['No account. No internet.', 'Meshtrike makes your identity on this phone. No number, no email, no server.'],
  ['Messages hop phone to phone.', 'Nearby phones carry sealed messages for a day (up to 72 hours when radio is jammed), so a note can reach someone several hops away.'],
  ['Slow is normal here.', 'Delivery can take minutes or hours. The app shows where each message is, so waiting never looks like failure.'],
];

export function Welcome() {
  const [step, setStep] = useState(0);
  // Back from Create returns to the card the user left from (Skip keeps its card).
  const [creating, setCreating] = useState(false);
  if (creating) return <CreateIdentity onBack={() => setCreating(false)} />;
  const last = step === CARDS.length - 1;
  return (
    <Appear kind="in" style={{ flex: 1, backgroundColor: color.bg, paddingTop: 12, paddingHorizontal: 24, paddingBottom: 34 }}>
      <View style={{ flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center', height: 40 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
          <IconTile icon="mesh" bg={color.lime} size={28} iconSize={16} />
          <Text style={{ fontFamily: font.display, fontSize: 17, color: color.ink }} accessibilityRole="header">
            Meshtrike
          </Text>
        </View>
        <Pressable onPress={() => setCreating(true)} accessibilityRole="button" accessibilityLabel="Skip" style={{ padding: 10 }}>
          <Text style={{ fontFamily: font.medium, fontSize: 14, color: color.textMuted }}>Skip</Text>
        </Pressable>
      </View>
      <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center' }}>
        <MeshIllustration />
      </View>
      <View style={{ gap: 12, minHeight: 170 }}>
        <Appear key={step} kind="up">
          <Text style={{ fontFamily: font.display, fontSize: 34, lineHeight: 36, letterSpacing: -1, color: color.ink }}>{CARDS[step][0]}</Text>
        </Appear>
        <Text style={{ fontFamily: font.regular, fontSize: 16, lineHeight: 24, color: color.inkSoft }}>{CARDS[step][1]}</Text>
      </View>
      <View style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 16, marginTop: 16 }}>
        <View style={{ flexDirection: 'row', gap: 6 }} accessibilityLabel={`Card ${step + 1} of ${CARDS.length}`}>
          {CARDS.map((_, i) => (
            <Dot key={i} on={i === step} />
          ))}
        </View>
        <Pressable
          onPress={() => (last ? setCreating(true) : setStep(step + 1))}
          accessibilityRole="button"
          accessibilityLabel={last ? 'Create my identity' : 'Continue'}
          style={({ pressed }) => ({
            height: 56,
            paddingHorizontal: 24,
            borderRadius: 99,
            backgroundColor: color.ink,
            flexDirection: 'row',
            alignItems: 'center',
            gap: 10,
            transform: [{ scale: pressed ? 0.97 : 1 }],
          })}
        >
          <Text style={{ fontFamily: font.semi, fontSize: 16, color: '#fff' }}>{last ? 'Create my identity' : 'Continue'}</Text>
          <Icon name="right" size={18} tint={color.lime} />
        </Pressable>
      </View>
    </Appear>
  );
}

function Dot({ on }: { on: boolean }) {
  const v = useRef(new Animated.Value(on ? 1 : 0)).current;
  useEffect(() => {
    Animated.timing(v, { toValue: on ? 1 : 0, duration: 300, easing: Easing.bezier(0.2, 0.8, 0.2, 1), useNativeDriver: false }).start();
  }, [on, v]);
  return (
    <Animated.View
      style={{
        height: 7,
        borderRadius: 9,
        width: v.interpolate({ inputRange: [0, 1], outputRange: [7, 22] }),
        backgroundColor: v.interpolate({ inputRange: [0, 1], outputRange: [color.dash, color.ink] }),
      }}
    />
  );
}

/** Three phones on a dashed path, a lime packet hopping between them. */
function MeshIllustration() {
  const [scale, setScale] = useState(1);
  const hop = useRef(new Animated.Value(0)).current;
  const still = useReduceMotion();
  useEffect(() => {
    if (still) return;
    const a = Animated.loop(Animated.timing(hop, { toValue: 1, duration: 3200, easing: Easing.bezier(0.6, 0, 0.4, 1), useNativeDriver: native }));
    a.start();
    return () => a.stop();
  }, [hop, still]);
  // Path M50 210 L150 70 L250 200: the corner sits at 51% of its length.
  const x = hop.interpolate({ inputRange: [0, 0.512, 1], outputRange: [50, 150, 250] });
  const y = hop.interpolate({ inputRange: [0, 0.512, 1], outputRange: [210, 70, 200] });
  return (
    <View
      style={{ alignSelf: 'stretch', alignItems: 'center', justifyContent: 'center', flex: 1 }}
      onLayout={(e) => setScale(Math.min(1, e.nativeEvent.layout.width / 300, e.nativeEvent.layout.height / 280))}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      <View style={{ width: 300, height: 280, transform: [{ scale }] }}>
        <Svg width={300} height={280} viewBox="0 0 300 280" style={{ position: 'absolute', left: 0, top: 0 }}>
          <Path d="M50 210 L150 70 L250 200 L50 210" fill="none" stroke={color.ink} strokeWidth={1.5} strokeDasharray="3 7" strokeLinecap="round" />
          <Circle cx={150} cy={140} r={128} fill="none" stroke="#DDD9D0" strokeWidth={1} />
          <Circle cx={150} cy={140} r={88} fill="none" stroke="#E6E3DC" strokeWidth={1} />
        </Svg>
        {/* 14 px lime packet with a 4 px halo outside it (design: box-shadow 0 0 0 4px). */}
        <Animated.View
          style={{
            position: 'absolute',
            left: -11,
            top: -11,
            width: 22,
            height: 22,
            borderRadius: 11,
            backgroundColor: color.limeGlow,
            alignItems: 'center',
            justifyContent: 'center',
            transform: [{ translateX: x }, { translateY: y }],
          }}
        >
          <View style={{ width: 14, height: 14, borderRadius: 7, backgroundColor: color.lime }} />
        </Animated.View>
        <Floaty left={22} top={180} delay={0} />
        <Floaty left={121} top={38} delay={600} dark />
        <Floaty left={221} top={170} delay={1200} />
      </View>
    </View>
  );
}

function Floaty({ left, top, delay, dark }: { left: number; top: number; delay: number; dark?: boolean }) {
  const v = useRef(new Animated.Value(0)).current;
  const still = useReduceMotion();
  useEffect(() => {
    if (still) return;
    const half = (to: number) => Animated.timing(v, { toValue: to, duration: 2000, easing: Easing.inOut(Easing.sin), useNativeDriver: native });
    const a = Animated.sequence([Animated.delay(delay), Animated.loop(Animated.sequence([half(1), half(0)]))]);
    a.start();
    return () => a.stop();
  }, [v, delay, still]);
  return (
    <Animated.View
      style={{
        position: 'absolute',
        left,
        top,
        width: 58,
        height: 58,
        borderRadius: 20,
        backgroundColor: dark ? color.ink : '#fff',
        alignItems: 'center',
        justifyContent: 'center',
        boxShadow: `0 10px 24px -10px rgba(0,0,0,${dark ? 0.35 : 0.25})`,
        transform: [{ translateY: v.interpolate({ inputRange: [0, 1], outputRange: [0, -6] }) }],
      }}
    >
      <Icon name="phone" size={24} tint={dark ? color.lime : color.ink} />
    </Animated.View>
  );
}

// ===== Create identity =====

// What create_identity does, in order, and where each step starts on the
// progress ring. It is one core call with no progress reports, so the ring is
// an estimate: it eases toward 90% while the call runs and fills when it returns.
const STEPS: [string, number][] = [
  ['Generating your keys', 0],
  ['Stretching your PIN', 35],
  ['Sealing the database', 75],
];
const NICK_MAX = 24; // characters, as in the design
const NICK_MAX_BYTES = 32; // the core's limit (MAX_NICKNAME_BYTES)
const RING_R = 54;
const RING_C = 2 * Math.PI * RING_R;
const FILL_MS = 350;
const AnimatedCircle = Animated.createAnimatedComponent(Circle);

function CreateIdentity({ onBack }: { onBack: () => void }) {
  const { call, afterUnlock, push, notify } = useApp();
  const [nickname, setNickname] = useState('');
  const [pin, setPin] = useState('');
  const [pin2, setPin2] = useState('');
  const [duressOn, setDuressOn] = useState(false);
  const [duress, setDuress] = useState('');
  const [bio, setBio] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pct, setPct] = useState<number | null>(null);

  const digits = (t: string) => t.replace(/\D/g, '').slice(0, 6);
  const create = async () => {
    const nick = nickname.trim();
    const d = duressOn ? duress : '';
    if (!nick) return setError('Choose a nickname. Others see it when you pair.');
    if (nick.length > NICK_MAX) return setError(`That nickname is too long. Keep it to ${NICK_MAX} characters.`);
    if (new TextEncoder().encode(nick).length > NICK_MAX_BYTES) return setError('That nickname is too long. Try fewer emoji or special characters.');
    if (!/^\d{6}$/.test(pin)) return setError('Your PIN needs 6 digits.');
    if (pin !== pin2) return setError('PINs don’t match. Enter the same 6 digits twice.');
    if (d && (!/^\d{6}$/.test(d) || d === pin)) return setError('Duress PIN must be 6 digits and different from your PIN.');
    setError(null);
    setPct(0);
    try {
      await call('create_identity', { nickname: nick, pin, duress_pin: d || undefined });
      // The identity exists now: a fingerprint failure must not strand the
      // user on this form (a retry would hit an existing identity).
      let bioError: string | null = null;
      if (bio) await call('enable_biometric', { pin }).catch((e) => (bioError = errorText(e)));
      setPct(100);
      // Let the ring fill and the last step tick, then move on (design: 400 ms at 100%).
      await new Promise((r) => setTimeout(r, FILL_MS + 400));
      await afterUnlock();
      if (bioError) notify(`Fingerprint unlock not set up: ${bioError}`);
      // Android asks for radio, camera and notification permissions next.
      if (Platform.OS === 'android') push({ name: 'Permissions' });
    } catch (e) {
      setPct(null);
      setError(errorText(e));
    }
  };

  if (pct !== null) return <Creating finished={pct >= 100} />;
  return (
    <Appear kind="right" style={{ flex: 1, backgroundColor: color.bg }}>
      <ScrollView contentContainerStyle={{ paddingTop: 8, paddingHorizontal: 22, paddingBottom: 30, gap: 20 }} keyboardShouldPersistTaps="handled" showsVerticalScrollIndicator={false}>
        <Pressable onPress={onBack} accessibilityRole="button" accessibilityLabel="Back" style={{ width: 44, height: 44, marginLeft: -10, alignItems: 'center', justifyContent: 'center' }}>
          <Icon name="left" size={24} />
        </Pressable>
        <View style={{ gap: 8 }}>
          <Text style={type.display} accessibilityRole="header">
            Create your identity
          </Text>
          <Text style={{ fontFamily: font.regular, fontSize: 15, lineHeight: 22, color: color.textMuted }}>Your keys are made here and never leave this phone.</Text>
        </View>
        <Field
          label="Nickname"
          value={nickname}
          onChangeText={(t) => {
            setNickname(t.slice(0, NICK_MAX));
            setError(null);
          }}
          maxLength={NICK_MAX}
          placeholder="What friends call you"
          hint="Anyone can pick any name. That’s why trust badges matter more."
        />
        <View style={{ flexDirection: 'row', gap: 10 }}>
          <PinField shown="6-digit PIN" label="App PIN (6 digits)" value={pin} onChange={(t) => (setPin(digits(t)), setError(null))} />
          <PinField shown="Repeat PIN" label="Repeat PIN" value={pin2} onChange={(t) => (setPin2(digits(t)), setError(null))} />
        </View>
        <View style={{ backgroundColor: '#fff', borderRadius: 20, paddingVertical: 4, paddingHorizontal: 16 }}>
          <OptionRow icon="key" iconBg={color.dangerBg} iconFg={color.danger} title="Duress PIN" sub="Entering it wipes everything silently and shows a fresh install.">
            <Toggle label="Duress PIN" value={duressOn} onChange={setDuressOn} />
          </OptionRow>
          {duressOn ? (
            <Appear kind="up" style={{ paddingBottom: 14 }}>
              <TextInput
                accessibilityLabel="Duress PIN (optional)"
                value={duress}
                onChangeText={(t) => (setDuress(digits(t)), setError(null))}
                placeholder="Duress PIN"
                placeholderTextColor={color.textFaint}
                keyboardType="number-pad"
                secureTextEntry
                maxLength={6}
                style={{
                  height: 50,
                  borderRadius: 14,
                  borderWidth: 1.5,
                  borderStyle: 'dashed',
                  borderColor: color.dangerLine,
                  backgroundColor: '#FFF8F3',
                  paddingHorizontal: 14,
                  fontFamily: font.monoSemi,
                  fontSize: 18,
                  letterSpacing: 5.4,
                  color: color.ink,
                }}
              />
            </Appear>
          ) : null}
          <View style={{ height: 1, backgroundColor: '#EEEBE4' }} />
          <OptionRow icon="finger" iconBg={color.chip2} iconFg={color.ink} title="Fingerprint shortcut" sub="A shortcut only. Your PIN is still needed after restarts.">
            <Toggle label="Fingerprint" value={bio} onChange={setBio} />
          </OptionRow>
        </View>
        <ErrorText error={error} />
        <Button title="Create identity" onPress={create} style={{ height: 58 }} />
      </ScrollView>
    </Appear>
  );
}

function PinField({ shown, label, value, onChange }: { shown: string; label: string; value: string; onChange: (t: string) => void }) {
  return (
    <View style={{ flex: 1, gap: 8, minWidth: 0 }}>
      <Text style={{ fontFamily: font.semi, fontSize: 13, color: color.ink }}>{shown}</Text>
      <Field label={label} hideLabel mono style={{ letterSpacing: 6 }} value={value} onChangeText={onChange} keyboardType="number-pad" secureTextEntry maxLength={6} placeholder="••••••" />
    </View>
  );
}

function OptionRow({ icon, iconBg, iconFg, title, sub, children }: { icon: IconName; iconBg: string; iconFg: string; title: string; sub: string; children: React.ReactNode }) {
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 12, paddingVertical: 12 }}>
      <IconTile icon={icon} bg={iconBg} fg={iconFg} size={36} iconSize={18} />
      <View style={{ flex: 1, gap: 2 }}>
        <Text style={{ fontFamily: font.semi, fontSize: 15, color: color.ink }}>{title}</Text>
        <Text style={{ fontFamily: font.regular, fontSize: 12.5, lineHeight: 17, color: color.textMuted }}>{sub}</Text>
      </View>
      {children}
    </View>
  );
}

/**
 * Progress ring while create_identity runs (design: 132 px ring, NN% centred).
 * The core reports no progress, so it eases toward 90% over ~3.6 s and fills
 * to 100% only when the call has returned.
 */
function Creating({ finished }: { finished: boolean }) {
  const p = useRef(new Animated.Value(0)).current;
  const [pct, setPct] = useState(0);
  useEffect(() => {
    const id = p.addListener(({ value }) => setPct(Math.round(value)));
    return () => p.removeListener(id);
  }, [p]);
  useEffect(() => {
    const a = finished
      ? Animated.timing(p, { toValue: 100, duration: FILL_MS, easing: Easing.out(Easing.cubic), useNativeDriver: false })
      : Animated.timing(p, { toValue: 90, duration: 3600, easing: Easing.out(Easing.quad), useNativeDriver: false });
    a.start();
    return () => a.stop();
  }, [p, finished]);
  const offset = p.interpolate({ inputRange: [0, 100], outputRange: [RING_C, 0], extrapolate: 'clamp' });
  return (
    <Appear kind="in" style={{ flex: 1, backgroundColor: color.bg, alignItems: 'center', justifyContent: 'center', gap: 34, padding: 24 }}>
      <View
        style={{ width: 132, height: 132 }}
        accessibilityRole="progressbar"
        accessibilityLabel={finished ? 'Identity created' : 'Creating identity'}
        accessibilityValue={{ min: 0, max: 100, now: pct }}
        accessibilityState={{ busy: !finished }}
      >
        <Svg width={132} height={132} viewBox="0 0 132 132" style={{ transform: [{ rotate: '-90deg' }] }}>
          <Circle cx={66} cy={66} r={RING_R} fill="none" stroke="#E6E3DC" strokeWidth={10} />
          <AnimatedCircle
            cx={66}
            cy={66}
            r={RING_R}
            fill="none"
            stroke={color.ink}
            strokeWidth={10}
            strokeLinecap="round"
            strokeDasharray={`${RING_C} ${RING_C}`}
            strokeDashoffset={offset}
          />
        </Svg>
        <View style={{ position: 'absolute', left: 0, top: 0, right: 0, bottom: 0, alignItems: 'center', justifyContent: 'center' }}>
          <Text style={{ fontFamily: font.display, fontSize: 28, color: color.ink }}>{pct}%</Text>
        </View>
      </View>
      <View style={{ gap: 14, width: 240 }}>
        {STEPS.map(([label, at], i) => {
          const end = STEPS[i + 1]?.[1] ?? 100;
          const done = pct >= end;
          const active = !done && pct >= at;
          return (
            <View key={label} style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }} accessibilityLabel={`${label}${done ? ', done' : active ? ', in progress' : ''}`}>
              {done ? (
                <Appear kind="pop">
                  <View style={{ width: 26, height: 26, borderRadius: 13, backgroundColor: color.lime, alignItems: 'center', justifyContent: 'center' }}>
                    <Icon name="check" size={15} />
                  </View>
                </Appear>
              ) : active ? (
                <Spinner size={26} />
              ) : (
                <View style={{ width: 26, height: 26, borderRadius: 13, borderWidth: 2, borderStyle: 'dashed', borderColor: color.dash }} />
              )}
              <Text style={{ fontFamily: font.medium, fontSize: 15, color: color.ink }}>{label}</Text>
            </View>
          );
        })}
      </View>
      <Text style={{ fontFamily: font.regular, fontSize: 13, lineHeight: 18, color: color.textMuted, textAlign: 'center', maxWidth: 260 }}>
        This takes a few seconds on purpose. It makes your PIN much harder to guess.
      </Text>
    </Appear>
  );
}

// ===== Permissions (Android) =====

type PermState = 'ask' | 'granted' | 'denied';
type PermKey = 'bt' | 'loc' | 'cam' | 'notif';

const SDK = Platform.OS === 'android' ? (Platform.Version as number) : 0;

const PERMS: { key: PermKey; icon: IconName; title: string; desc: string; android: string[] }[] = [
  {
    key: 'bt',
    icon: 'bt',
    title: 'Nearby devices',
    desc: 'Find and talk to phones around you over Bluetooth.',
    android: SDK >= 31 ? ['android.permission.BLUETOOTH_SCAN', 'android.permission.BLUETOOTH_ADVERTISE', 'android.permission.BLUETOOTH_CONNECT'] : ['android.permission.ACCESS_FINE_LOCATION'],
  },
  // Android 11 and older need location for Bluetooth scanning; newer versions do not.
  ...(SDK > 0 && SDK < 31
    ? [{ key: 'loc' as const, icon: 'pin' as const, title: 'Location', desc: 'Android requires it for Bluetooth scanning. Meshtrike never reads where you are.', android: ['android.permission.ACCESS_FINE_LOCATION'] }]
    : []),
  { key: 'cam', icon: 'camera', title: 'Camera', desc: 'Scan pairing codes and QR handoff streams.', android: ['android.permission.CAMERA'] },
  ...(SDK >= 33 || SDK === 0
    ? [{ key: 'notif' as const, icon: 'bell' as const, title: 'Notifications', desc: 'Alerts say only “New message”. Never names or text.', android: ['android.permission.POST_NOTIFICATIONS'] }]
    : []),
];

type PermResponse = { granted: boolean; permissions?: Record<string, { status?: string; granted?: boolean }> };

export function Permissions() {
  const { tab, call } = useApp();
  const [st, setSt] = useState<Record<string, PermState>>({});
  const [busy, setBusy] = useState(false);
  const stRef = useRef(st);
  useEffect(() => {
    stRef.current = st;
  }, [st]);

  // Start from what Android has actually granted (no dialog shown), and read it
  // again whenever the app comes back to the front: the user may have allowed a
  // permission in system settings ("Denied · Open system settings").
  useEffect(() => {
    if (Platform.OS !== 'android') return;
    const read = (fromSettings: boolean) => {
      try {
        const g = meshCoreNative().permissionStatus();
        const now = (p: (typeof PERMS)[number]) => p.android.every((n) => g[n]);
        const gained = PERMS.some((p) => now(p) && stRef.current[p.key] !== 'granted');
        setSt((prev) => {
          const next = { ...prev };
          for (const p of PERMS) {
            if (now(p)) next[p.key] = 'granted';
            else if (prev[p.key] === 'granted') next[p.key] = 'denied';
          }
          return next;
        });
        // The radio only picks up new permissions on restart.
        if (fromSettings && gained) call('radio_restart').catch(() => {});
      } catch {
        /* older native module: fall back to asking */
      }
    };
    read(false);
    const sub = AppState.addEventListener('change', (a) => a === 'active' && read(true));
    return () => sub.remove();
  }, [call]);

  const apply = (r: PermResponse, asked: PermKey) => {
    setSt((prev) => {
      const next = { ...prev };
      for (const p of PERMS) {
        const detail = r.permissions ? p.android.map((n) => r.permissions![n]).filter(Boolean) : [];
        const ok = detail.length ? detail.every((d) => d!.granted || d!.status === 'granted') : r.granted;
        if (ok) next[p.key] = 'granted';
        else if (p.key === asked) next[p.key] = 'denied';
      }
      return next;
    });
  };

  // The app already asked once at startup; reflect what is granted without new dialogs where possible.
  const request = async (asked: PermKey) => {
    if (Platform.OS !== 'android') return setSt((s) => ({ ...s, [asked]: 'granted' }));
    setBusy(true);
    try {
      const r = (await meshCoreNative().requestPermissions()) as PermResponse;
      apply(r, asked);
      call('radio_restart').catch(() => {});
    } catch {
      setSt((s) => ({ ...s, [asked]: 'denied' }));
    } finally {
      setBusy(false);
    }
  };

  const allowed = PERMS.filter((p) => st[p.key] === 'granted').length;
  return (
    <Appear kind="right" style={{ flex: 1, backgroundColor: color.bg, paddingTop: 16, paddingHorizontal: 22, paddingBottom: 34, gap: 18 }}>
      <View style={{ gap: 8 }}>
        <Text style={{ fontFamily: font.monoSemi, fontSize: 12, letterSpacing: 0.7, color: color.textMuted }}>
          {allowed} of {PERMS.length} allowed
        </Text>
        <Text style={{ fontFamily: font.display, fontSize: 30, lineHeight: 32, letterSpacing: -0.9, color: color.ink }} accessibilityRole="header">
          A few permissions
        </Text>
        <Text style={{ fontFamily: font.regular, fontSize: 15, lineHeight: 22, color: color.textMuted }}>Meshtrike has no internet access. These let it reach phones nearby.</Text>
      </View>
      <ScrollView style={{ flex: 1 }} contentContainerStyle={{ gap: 10 }} showsVerticalScrollIndicator={false}>
        {PERMS.map((p, i) => {
          const s = st[p.key] ?? 'ask';
          return (
            <Appear key={p.key} kind="up" delay={i * 60}>
              <View style={{ backgroundColor: '#fff', borderRadius: 20, padding: 14, flexDirection: 'row', gap: 12, alignItems: 'flex-start' }}>
                <IconTile icon={p.icon} size={40} />
                <View style={{ flex: 1, gap: 3, minWidth: 0 }}>
                  <Text style={{ fontFamily: font.semi, fontSize: 15, color: color.ink }}>{p.title}</Text>
                  <Text style={{ fontFamily: font.regular, fontSize: 12.5, lineHeight: 17, color: color.textMuted }}>{p.desc}</Text>
                  {s === 'denied' ? (
                    <Pressable
                      onPress={() => Linking.openSettings().catch(() => {})}
                      accessibilityRole="button"
                      accessibilityLabel={`${p.title} denied. Open system settings`}
                      style={{ marginTop: 6, alignSelf: 'flex-start', backgroundColor: color.dangerBg, borderRadius: 99, paddingVertical: 7, paddingHorizontal: 12 }}
                    >
                      <Text style={{ fontFamily: font.semi, fontSize: 12.5, color: color.dangerDeep }}>Denied · Open system settings</Text>
                    </Pressable>
                  ) : null}
                </View>
                {s === 'ask' ? (
                  <View style={{ alignItems: 'flex-end', gap: 4 }}>
                    <Pressable
                      onPress={() => request(p.key)}
                      disabled={busy}
                      accessibilityRole="button"
                      accessibilityLabel={`Allow ${p.title}`}
                      style={{ height: 36, paddingHorizontal: 16, borderRadius: 99, backgroundColor: color.ink, alignItems: 'center', justifyContent: 'center', opacity: busy ? 0.5 : 1 }}
                    >
                      <Text style={{ fontFamily: font.semi, fontSize: 13, color: '#fff' }}>Allow</Text>
                    </Pressable>
                    <Pressable onPress={() => setSt((x) => ({ ...x, [p.key]: 'denied' }))} accessibilityRole="button" accessibilityLabel={`Deny ${p.title}`} style={{ padding: 4 }}>
                      <Text style={{ fontFamily: font.medium, fontSize: 12, color: color.textMuted }}>Deny</Text>
                    </Pressable>
                  </View>
                ) : s === 'granted' ? (
                  <Appear kind="pop">
                    <View accessibilityLabel={`${p.title} allowed`} style={{ height: 32, paddingLeft: 9, paddingRight: 12, borderRadius: 99, backgroundColor: color.goodBg, flexDirection: 'row', alignItems: 'center', gap: 5 }}>
                      <Icon name="check" size={15} tint={color.good} />
                      <Text style={{ fontFamily: font.semi, fontSize: 12.5, color: color.good }}>Allowed</Text>
                    </View>
                  </Appear>
                ) : null}
              </View>
            </Appear>
          );
        })}
      </ScrollView>
      <Button title="Continue to chats" onPress={() => tab('Chats')} style={{ height: 58 }} />
    </Appear>
  );
}

// ===== Unlock =====

export function Unlock() {
  const { call, afterUnlock, notify } = useApp();
  const [busy, setBusy] = useState(false);
  // Show the fingerprint key only when fingerprint unlock was set up.
  const [bioOn, setBioOn] = useState(false);
  useEffect(() => {
    call<boolean>('biometric_enabled').then(setBioOn, () => setBioOn(false));
  }, [call]);
  const [msg, setMsg] = useState<{ text: string; tone: 'idle' | 'wrong' | 'danger' }>({ text: 'Enter your 6-digit PIN', tone: 'idle' });
  const [shake, setShake] = useState(0);
  const tryPin = async (pin: string) => {
    setBusy(true);
    AccessibilityInfo.announceForAccessibility('Checking PIN');
    try {
      const r = await call<UnlockResult>('unlock', { pin });
      if (r.result === 'Ok') await afterUnlock();
      else if (r.result === 'WrongPin') {
        setShake((n) => n + 1);
        setMsg(
          r.remaining != null
            ? { text: `Wrong PIN · ${r.remaining} attempts left before Meshtrike wipes itself`, tone: 'danger' }
            : { text: 'Wrong PIN. Try again.', tone: 'wrong' },
        );
      }
      // 'Wiped' → the provider receives a Wiped event and shows Welcome.
    } catch (e) {
      setMsg({ text: errorText(e), tone: 'danger' });
    } finally {
      setBusy(false);
    }
  };
  const bio = async () => {
    try {
      const r = await call<UnlockResult>('unlock_biometric');
      if (r.result === 'Ok') {
        notify('Fingerprint accepted');
        await afterUnlock();
      }
    } catch (e) {
      // Say what actually failed (cancelled, not enrolled, key gone…), not a guess.
      setMsg({ text: `Fingerprint didn't unlock: ${errorText(e)}. Use your PIN.`, tone: 'wrong' });
    }
  };
  const tint = { idle: color.textMuted, wrong: color.ink, danger: color.danger }[msg.tone];
  return (
    <Appear kind="in" style={{ flex: 1, backgroundColor: color.bg, alignItems: 'center', paddingTop: 30, paddingHorizontal: 30, paddingBottom: 40 }}>
      {/* "Enter PIN" keeps the Android test drivers' anchor on this screen. */}
      <View accessibilityLabel="Enter PIN" style={{ marginTop: 20 }}>
        <IconTile icon="lock" bg={color.ink} fg={color.lime} size={64} iconSize={28} />
      </View>
      <Text style={{ fontFamily: font.display, fontSize: 26, lineHeight: 29, letterSpacing: -0.5, color: color.ink, marginTop: 20 }} accessibilityRole="header">
        Meshtrike is locked
      </Text>
      <Text
        accessibilityLiveRegion="polite"
        accessibilityRole={msg.tone === 'idle' ? undefined : 'alert'}
        style={{ fontFamily: font.medium, fontSize: 14, lineHeight: 20, marginTop: 8, textAlign: 'center', minHeight: 40, maxWidth: 270, color: tint }}
      >
        {msg.text}
      </Text>
      {/* Dots right under the message, keypad at the bottom (design). */}
      <PinPad split onComplete={tryPin} disabled={busy} onBiometric={bioOn ? bio : undefined} shake={shake} />
    </Appear>
  );
}

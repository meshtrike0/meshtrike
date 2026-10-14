import React, { useEffect, useRef, useState } from 'react';
import { Animated, Modal, Pressable, StyleSheet, Text, View } from 'react-native';
import { CallView, TransportKind } from '../core/types';
import { Icon, IconName } from './icons';
import { Appear, Avatar, Spinner, useLoop, useReduceMotion } from './kit';
import { errorText, useApp, useCoreEvents } from './state';
import { color, font } from './theme';

const LINK: Partial<Record<TransportKind, { label: string; icon: IconName }>> = {
  Ble: { label: 'Bluetooth', icon: 'bt' },
  WifiP2p: { label: 'Wi-Fi', icon: 'wifi' },
};
const RED = '#C2410C';

function clock(ms: number) {
  const t = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`;
}

function bigClock(ms: number) {
  const t = Math.max(0, Math.floor(ms / 1000));
  return `${String(Math.floor(t / 60)).padStart(2, '0')}:${String(t % 60).padStart(2, '0')}`;
}

/**
 * The nearby call, full screen over the app: calling, incoming, in call,
 * reconnecting, and a short "Call ended". Rings and the lock-screen alert
 * are native (no name ever reaches a notification). When the call rang over
 * the phone's lock screen, the caller stays hidden until the phone unlocks.
 */
export function CallScreen() {
  const { call, client, contacts, notify } = useApp();
  const hooks = client.hooks.call;
  const [c, setC] = useState<CallView | null>(null);
  const [ended, setEnded] = useState<{ view: CallView; secs: number } | null>(null);
  const [muted, setMuted] = useState(false);
  const [speaker, setSpeaker] = useState(false);
  const [unlocking, setUnlocking] = useState(false);
  const [locked, setLocked] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  // A short note when the call moves to another link.
  const [chip, setChip] = useState<string | null>(null);
  const lastLink = useRef<TransportKind | null>(null);
  const lostAt = useRef<number | null>(null);
  const audioFor = useRef<string | null>(null);
  const cRef = useRef<CallView | null>(null);
  cRef.current = c;

  useEffect(() => {
    call<CallView | null>('current_call')
      .then((v) => {
        if (!v) return;
        if (v.state === 'Ringing') setLocked(!!hooks?.deviceLocked());
        setC(v);
      })
      .catch(() => {});
  }, [call, hooks]);
  useCoreEvents(
    (e) => {
      if (e.event !== 'CallChanged') return;
      const v = e.call;
      if (v.state === 'Ended') {
        const prev = cRef.current;
        setEnded({ view: v, secs: prev?.answered_at ? Math.floor((Date.now() - prev.answered_at) / 1000) : 0 });
        setC(null);
        setUnlocking(false);
        setTimeout(() => setEnded((x) => (x?.view.id === v.id ? null : x)), 1400);
        return;
      }
      if (v.state === 'Active' && v.link && lastLink.current && v.link !== lastLink.current) {
        setChip(v.link === 'WifiP2p' ? 'Switched to Wi-Fi' : 'Back on Bluetooth');
        setTimeout(() => setChip(null), 3000);
      }
      if (v.link) lastLink.current = v.link;
      // The phone's own lock screen: checked as the call starts ringing.
      if (v.state === 'Ringing' && cRef.current?.id !== v.id) setLocked(!!hooks?.deviceLocked());
      if (v.reconnecting && lostAt.current == null) lostAt.current = Date.now();
      if (!v.reconnecting) lostAt.current = null;
      setC(v);
    },
    [hooks],
  );
  // Ring-back while calling; audio while active.
  useEffect(() => {
    if (!hooks) return;
    hooks.ringback(c?.state === 'Calling').catch(() => {});
    if (c?.state === 'Active' && audioFor.current !== c.id) {
      audioFor.current = c.id;
      lastLink.current = c.link;
      setMuted(false);
      setSpeaker(false);
      hooks.audio(true, c.link === 'WifiP2p').catch((e) => notify(errorText(e)));
    }
    if (c?.state !== 'Active' && audioFor.current) {
      audioFor.current = null;
      hooks.audio(false, false).catch(() => {});
    }
  }, [c?.state, c?.id, c?.link, hooks, notify]);
  useEffect(() => {
    if (c?.state !== 'Active') return;
    const t = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(t);
  }, [c?.state]);

  const shown = c ?? ended?.view ?? null;
  if (!shown) return null;
  const who = contacts.find((x) => x.id === shown.contact);
  const name = who?.nickname ?? 'Contact';
  const link = shown.link ? LINK[shown.link] : undefined;
  const ringing = c?.state === 'Ringing';
  const hidden = ringing && locked;
  const reconnecting = c?.state === 'Active' && c.reconnecting;
  const active = c?.state === 'Active' && !c.reconnecting;

  const answer = async () => {
    if (!c) return;
    if (hooks && !hooks.micGranted() && !(await hooks.askMic())) return notify('Meshtrike needs the microphone for calls');
    call('answer_call', { call: c.id }).catch((e) => notify(errorText(e)));
  };
  const accept = async () => {
    if (!c) return;
    if (!hidden) return answer();
    setUnlocking(true);
  };
  const unlock = async () => {
    if (!hooks) return;
    const ok = await hooks.unlockDevice().catch(() => false);
    if (!ok) return notify('Unlock your phone to answer');
    setLocked(false);
    setUnlocking(false);
    answer();
  };
  const end = () => {
    if (!c) return;
    call('end_call', { call: c.id }).catch((e) => notify(errorText(e)));
  };

  let title = name;
  let status: string | null = null;
  if (!c) {
    const r = ended?.view.reason;
    status = ended?.view.answered_at ? `Call ended · ${clock((ended?.secs ?? 0) * 1000)}` : `Call ended${r && r !== 'ended' ? ` · ${r}` : ''}`;
  } else if (c.state === 'Calling') status = `Calling…${link ? ` · over ${link.label}` : ''}`;
  else if (ringing && hidden) {
    title = unlocking ? 'Unlock to answer' : 'Incoming call';
    status = unlocking ? 'The caller is shown after you unlock' : 'Meshtrike · nearby';
  } else if (ringing) status = `Incoming call${link ? ` · over ${link.label}` : ''}`;
  else if (reconnecting) {
    title = 'Reconnecting…';
    status = `Stay close · ${clock(now - (lostAt.current ?? now))}`;
  }
  const verified = who?.trust === 'Verified';
  const q = c?.quality ?? 0;

  return (
    <Modal visible transparent={false} animationType="fade" onRequestClose={() => {}} statusBarTranslucent>
      <View style={st.screen} accessibilityViewIsModal>
        <View style={{ alignItems: 'center' }}>
          <View style={st.avatarBox}>
            {ringing || c?.state === 'Calling' || reconnecting ? <Glow /> : null}
            {hidden ? (
              <View style={st.hiddenTile}>
                <Icon name="call" size={46} tint={color.lime} />
              </View>
            ) : (
              <View style={st.avatarRing}>
                <Avatar seed={who?.identity_key ?? shown.contact} size={112} kind="person" />
              </View>
            )}
          </View>
          <Text style={st.title} numberOfLines={1} accessibilityRole="header">
            {title}
          </Text>
          {status ? (
            <Text style={st.status} accessibilityLiveRegion="polite">
              {status}
            </Text>
          ) : null}
          {active && c ? (
            <>
              <Text style={st.timer} accessibilityLabel={`Call time ${clock(now - (c.answered_at ?? now))}`}>
                {bigClock(now - (c.answered_at ?? now))}
              </Text>
              <View style={st.linkChip} accessibilityLabel={`${link?.label ?? 'Link'}, signal ${q >= 3 ? 'good' : 'weak'}`}>
                {link ? <Icon name={link.icon} size={15} tint="#fff" /> : null}
                <Text style={st.linkText}>
                  {link?.label ?? 'Nearby'} · {q >= 3 ? 'Good' : 'Weak'}
                </Text>
                <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 2, height: 14, marginLeft: 2 }}>
                  {[1, 2, 3, 4].map((i) => (
                    <View key={i} style={{ width: 4, borderRadius: 1, height: 2 + i * 3, backgroundColor: i <= q ? (q >= 3 ? color.lime : '#FFB38A') : 'rgba(255,255,255,0.22)' }} />
                  ))}
                </View>
              </View>
              {q < 3 ? (
                <Text accessibilityRole="alert" style={st.weak}>
                  Moving apart weakens the call
                </Text>
              ) : null}
              {chip ? (
                <Appear kind="pop" style={st.switchChip}>
                  <Text style={{ fontFamily: font.semi, fontSize: 12.5, color: color.ink }}>{chip}</Text>
                </Appear>
              ) : null}
            </>
          ) : null}
          {reconnecting ? (
            <View style={st.lostChip}>
              <Icon name={lastLink.current === 'WifiP2p' ? 'wifi' : 'bt'} size={15} tint="#FFB38A" />
              <Text style={{ fontFamily: font.semi, fontSize: 13, color: '#FFB38A' }}>
                {lastLink.current === 'WifiP2p' ? 'Lost Wi-Fi · trying Bluetooth' : 'Lost Bluetooth · stay close'}
              </Text>
            </View>
          ) : null}
          {!hidden && c ? (
            <View style={st.secRow}>
              <Icon name="shieldOk" size={15} tint={verified ? '#9BE3BF' : 'rgba(255,255,255,0.75)'} />
              <Text style={[st.small, { color: verified ? '#9BE3BF' : 'rgba(255,255,255,0.75)' }]}>{verified ? 'Verified' : 'Not verified'} · end-to-end encrypted</Text>
            </View>
          ) : null}
        </View>
        <View style={{ flex: 1 }} />
        {!c ? (
          <View style={{ alignItems: 'center' }}>
            <Spinner size={22} />
          </View>
        ) : c.state === 'Calling' ? (
          <View style={st.row}>
            <RoundButton icon="x" label="Cancel" bg={RED} size={72} onPress={end} />
          </View>
        ) : ringing ? (
          <View style={[st.row, { gap: 64 }]}>
            <RoundButton icon="x" label="Decline" bg={RED} size={76} onPress={end} />
            {hidden && unlocking ? (
              <RoundButton icon="finger" label="Fingerprint" bg="#fff" tint={color.ink} size={76} onPress={unlock} />
            ) : (
              <RoundButton icon="check" label="Accept" bg={color.lime} tint={color.ink} size={76} onPress={accept} />
            )}
          </View>
        ) : (
          <View style={[st.row, { gap: 28 }]}>
            <RoundButton
              icon={muted ? 'micOff' : 'mic'}
              label={muted ? 'Muted' : 'Mute'}
              bg={muted ? '#fff' : 'rgba(255,255,255,0.12)'}
              tint={muted ? color.ink : '#fff'}
              onPress={() => {
                setMuted(!muted);
                hooks?.mute(!muted).catch(() => {});
              }}
            />
            {active ? (
              <RoundButton
                icon="speaker"
                label="Speaker"
                bg={speaker ? '#fff' : 'rgba(255,255,255,0.12)'}
                tint={speaker ? color.ink : '#fff'}
                selected={speaker}
                onPress={() => {
                  setSpeaker(!speaker);
                  hooks?.speaker(!speaker).catch(() => {});
                }}
              />
            ) : null}
            <RoundButton icon="x" label="End" bg={RED} onPress={end} />
          </View>
        )}
      </View>
    </Modal>
  );
}

function RoundButton({
  icon,
  label,
  bg,
  tint = '#fff',
  size = 68,
  selected,
  onPress,
}: {
  icon: IconName;
  label: string;
  bg: string;
  tint?: string;
  size?: number;
  selected?: boolean;
  onPress: () => void;
}) {
  return (
    <View style={{ alignItems: 'center', gap: 8 }}>
      <Pressable
        onPress={onPress}
        accessibilityRole="button"
        accessibilityLabel={label}
        accessibilityState={selected === undefined ? undefined : { selected }}
        style={({ pressed }) => [{ width: size, height: size, borderRadius: size / 2, alignItems: 'center', justifyContent: 'center', backgroundColor: bg }, pressed ? { transform: [{ scale: 0.94 }] } : null]}
      >
        <Icon name={icon} size={26} tint={tint} />
      </Pressable>
      <Text style={{ fontFamily: font.medium, fontSize: 13, color: 'rgba(255,255,255,0.8)' }}>{label}</Text>
    </View>
  );
}

/** Two soft lime rings spreading from the avatar while it rings (still under reduce motion). */
function Glow() {
  const reduce = useReduceMotion();
  const a = useLoop(2400);
  const b = useLoop(2400, 1200);
  if (reduce) return null;
  const ring = (v: Animated.Value, o: number) => ({
    transform: [{ scale: v.interpolate({ inputRange: [0, 1], outputRange: [0.15, 1] }) }],
    opacity: v.interpolate({ inputRange: [0, 1], outputRange: [o, 0] }),
  });
  return (
    <>
      <Animated.View pointerEvents="none" style={[st.glow, ring(a, 0.28)]} />
      <Animated.View pointerEvents="none" style={[st.glow, ring(b, 0.22)]} />
    </>
  );
}

const st = StyleSheet.create({
  screen: { flex: 1, backgroundColor: '#0B0C0F', paddingTop: 110, paddingHorizontal: 28, paddingBottom: 56 },
  avatarBox: { width: 132, height: 132, alignItems: 'center', justifyContent: 'center' },
  avatarRing: { padding: 4, borderRadius: 44, backgroundColor: 'rgba(255,255,255,0.08)' },
  hiddenTile: { width: 120, height: 120, borderRadius: 40, backgroundColor: 'rgba(255,255,255,0.08)', alignItems: 'center', justifyContent: 'center' },
  glow: { position: 'absolute', width: 300, height: 300, borderRadius: 150, backgroundColor: color.lime },
  title: { fontFamily: font.display, fontSize: 30, letterSpacing: -0.6, color: '#fff', marginTop: 28, textAlign: 'center', maxWidth: '92%' },
  status: { fontFamily: font.medium, fontSize: 15, color: 'rgba(255,255,255,0.75)', marginTop: 8, textAlign: 'center' },
  timer: { fontFamily: font.monoSemi, fontSize: 42, color: '#fff', marginTop: 12 },
  linkChip: { flexDirection: 'row', alignItems: 'center', gap: 8, marginTop: 12, height: 32, paddingHorizontal: 12, borderRadius: 99, backgroundColor: 'rgba(255,255,255,0.08)' },
  linkText: { fontFamily: font.semi, fontSize: 13, color: '#fff' },
  weak: { marginTop: 10, fontFamily: font.medium, fontSize: 13, color: '#FFB38A' },
  switchChip: { marginTop: 10, height: 30, paddingHorizontal: 12, borderRadius: 99, backgroundColor: color.lime, justifyContent: 'center' },
  lostChip: { marginTop: 14, flexDirection: 'row', alignItems: 'center', gap: 8, height: 32, paddingHorizontal: 12, borderRadius: 99, backgroundColor: 'rgba(255,179,138,0.14)' },
  secRow: { marginTop: 16, flexDirection: 'row', alignItems: 'center', gap: 6 },
  small: { fontFamily: font.medium, fontSize: 12.5 },
  row: { flexDirection: 'row', justifyContent: 'center' },
});

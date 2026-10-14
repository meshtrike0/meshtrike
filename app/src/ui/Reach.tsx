import React, { useEffect, useState, useSyncExternalStore } from 'react';
import { Animated, Linking, Platform, Pressable, StyleSheet, Text, useWindowDimensions, View } from 'react-native';
import Svg, { Circle, Defs, RadialGradient, Stop } from 'react-native-svg';
import { meshCoreNative } from '../../modules/mesh-core/src/MeshCoreModule';
import { REACH_DOWNLOAD_URL } from '../config';
import { Icon, IconName } from './icons';
import { Sheet, useLoop } from './kit';
import { useApp } from './state';
import { color, font } from './theme';

/**
 * MeshGRID: the optional add-on that adds Wi-Fi Direct and the private
 * internet route. Meshtrike never downloads it itself: the "Get MeshGRID" sheet
 * opens the browser, and the app notices once Android has it installed.
 */

export const REACH = 'MeshGRID';
const VIOLET = '#6236D0';
const VIOLET_BG = '#ECE5FF';

export type ReachStatus = { installed: boolean; version: string | null };

// Shared between the sheet (drawn once, above everything) and the screens
// that offer it: whether it is open, and whether a download is under way.
let ui = { open: false, waiting: false };
const subs = new Set<() => void>();
const set = (u: Partial<typeof ui>) => {
  ui = { ...ui, ...u };
  subs.forEach((f) => f());
};
export const reachUi = {
  open: () => set({ open: true }),
  close: () => set({ open: false }),
  sub: (f: () => void) => {
    subs.add(f);
    return () => {
      subs.delete(f);
    };
  },
  get: () => ui,
};

function readStatus(): ReachStatus | null {
  if (Platform.OS !== 'android') return null; // the browser harness has no add-on
  try {
    const s = meshCoreNative().internetAddonStatus();
    return { installed: s.installed, version: s.version ?? null };
  } catch {
    return { installed: false, version: null };
  }
}

/** The add-on's state on this phone (null where there is no add-on to get). */
export function useReachStatus(): ReachStatus | null {
  const [st, setSt] = useState<ReachStatus | null>(readStatus);
  useEffect(() => {
    if (Platform.OS !== 'android') return;
    const t = setInterval(() => setSt(readStatus()), 2500);
    return () => clearInterval(t);
  }, []);
  return st;
}

export function useReachUi() {
  return useSyncExternalStore(reachUi.sub, reachUi.get);
}

/** Open the browser on the download page; the app then waits for the install. */
export async function getReach() {
  reachUi.close();
  await Linking.openURL(REACH_DOWNLOAD_URL);
  set({ waiting: true });
}

/** Open the add-on's own screen. */
export function openReach() {
  try {
    meshCoreNative().openWifiLink();
  } catch {
    /* not installed */
  }
}

/** Offer the add-on once, unprompted (first visit to Chats, if it is missing). */
export function useReachFirstOffer(enabled: boolean) {
  const st = useReachStatus();
  useEffect(() => {
    if (!enabled || !st || st.installed) return;
    let seen = true;
    try {
      seen = meshCoreNative().reachPromptSeen();
    } catch {
      /* treat as seen */
    }
    if (seen) return;
    const t = setTimeout(() => {
      try {
        meshCoreNative().setReachPromptSeen();
      } catch {
        /* ignore */
      }
      reachUi.open();
    }, 650);
    return () => clearTimeout(t);
  }, [enabled, st]);
}

/** The "Get MeshGRID" sheet. Mounted once, above the tab bar. */
export function ReachSheet() {
  const { notify } = useApp();
  const { open, waiting } = useReachUi();
  const st = useReachStatus();
  // Fits short screens: a smaller picture and headline, fewer extras.
  const { height } = useWindowDimensions();
  const compact = height < 760;
  const heroH = Math.round(Math.max(120, Math.min(170, height * 0.18)));
  // Installed while we waited: say so.
  useEffect(() => {
    if (waiting && st?.installed) {
      set({ waiting: false });
      notify(`${REACH} installed${st.version ? ` · v${st.version}` : ''}`);
    }
  }, [waiting, st, notify]);
  return (
    <Sheet open={open} onClose={reachUi.close}>
      <Hero height={heroH} />
      <Text style={[st_.title, compact ? { fontSize: 22, lineHeight: 25 } : null]}>Reach people anywhere, not just nearby.</Text>
      <Text style={[st_.body, compact ? { fontSize: 13.5, lineHeight: 19 } : null]}>
        {REACH} is a free companion app. When you have internet, it carries your sealed messages to contacts across the world, and it speeds up photos nearby.
      </Text>
      <SpeedCompare />
      {compact ? null : (
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6, paddingHorizontal: 2 }}>
          <Chip icon="lock" text="Sealed before it leaves" good />
          <Chip icon="mesh" text="Meshtrike stays offline" />
          <Chip icon="x" text="Remove anytime" />
        </View>
      )}
      <Pressable
        onPress={() => getReach().catch(() => notify('Could not open the browser'))}
        accessibilityRole="link"
        accessibilityLabel={`Get ${REACH}, it’s free. Opens your browser`}
        style={({ pressed }) => [st_.cta, pressed ? { transform: [{ scale: 0.98 }] } : null]}
      >
        <Text style={{ fontFamily: font.bold, fontSize: 16, color: color.ink }}>Get MeshGRID · it’s free</Text>
        <Icon name="ext" size={16} tint={color.ink} />
      </Pressable>
      <View style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', paddingHorizontal: 6 }}>
        <Text style={{ fontFamily: font.regular, fontSize: 12, color: color.textFaint }}>Opens your browser</Text>
        <Pressable
          onPress={() => {
            reachUi.close();
            notify('You can get it any time in Settings');
          }}
          accessibilityRole="button"
          style={{ height: 40, justifyContent: 'center', paddingHorizontal: 4 }}
        >
          <Text style={{ fontFamily: font.medium, fontSize: 14, color: color.textMuted }}>Maybe later</Text>
        </Pressable>
      </View>
    </Sheet>
  );
}

/** Settings: get the add-on (dashed card), or open it once installed. */
export function ReachCard() {
  const st = useReachStatus();
  const { waiting } = useReachUi();
  if (!st) return null;
  if (st.installed) {
    return (
      <Pressable onPress={openReach} accessibilityRole="button" accessibilityLabel={`${REACH}, installed. Open it`} style={({ pressed }) => [st_.card, { paddingVertical: 14, flexDirection: 'row', alignItems: 'center', gap: 12 }, pressed ? { opacity: 0.8 } : null]}>
        <ReachTile />
        <View style={{ flex: 1, gap: 2 }}>
          <Text style={{ fontFamily: font.semi, fontSize: 15, color: color.ink }}>{REACH}</Text>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
            <View style={{ width: 7, height: 7, borderRadius: 4, backgroundColor: '#22A36B' }} />
            <Text style={{ fontFamily: font.medium, fontSize: 12, color: '#0B6E4A' }}>Installed{st.version ? ` · v${st.version}` : ''} · linked</Text>
          </View>
        </View>
        <Icon name="ext" size={17} tint={color.textFaint} />
      </Pressable>
    );
  }
  return (
    <View style={[st_.card, st_.dashed]}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }}>
        <ReachTile />
        <View style={{ flex: 1, gap: 2 }}>
          <Text style={{ fontFamily: font.semi, fontSize: 15, color: color.ink }}>{REACH}</Text>
          <Text style={{ fontFamily: font.medium, fontSize: 12, color: color.textMuted }}>Optional add-on · not installed</Text>
        </View>
      </View>
      <Text style={{ fontFamily: font.regular, fontSize: 12.5, lineHeight: 18, color: color.textMuted }}>
        Adds Wi-Fi Direct for fast transfers and the private internet route. Meshtrike works without it.
      </Text>
      {waiting ? (
        <View style={{ height: 46, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 10 }} accessibilityLiveRegion="polite">
          <SmallSpin />
          <Text style={{ fontFamily: font.medium, fontSize: 13.5, color: color.textMuted }}>Waiting for install…</Text>
        </View>
      ) : (
        <Pressable onPress={reachUi.open} accessibilityRole="button" style={({ pressed }) => [st_.dl, pressed ? { opacity: 0.85 } : null]}>
          <Text style={{ fontFamily: font.semi, fontSize: 14.5, color: '#fff' }}>Download {REACH}</Text>
          <Icon name="ext" size={15} tint={color.lime} />
        </Pressable>
      )}
    </View>
  );
}

export function ReachTile({ size = 42 }: { size?: number }) {
  return (
    <View style={{ width: size, height: size, borderRadius: Math.round(size / 3), backgroundColor: VIOLET_BG, alignItems: 'center', justifyContent: 'center' }}>
      <Icon name="link" size={Math.round(size * 0.48)} tint={VIOLET} />
    </View>
  );
}

function SmallSpin() {
  const v = useLoop(800);
  return (
    <Animated.View
      style={{
        width: 16,
        height: 16,
        borderRadius: 8,
        borderWidth: 2.5,
        borderColor: '#E6E3DC',
        borderTopColor: color.ink,
        transform: [{ rotate: v.interpolate({ inputRange: [0, 1], outputRange: ['0deg', '360deg'] }) }],
      }}
    />
  );
}

function Chip({ icon, text, good }: { icon: IconName; text: string; good?: boolean }) {
  const fg = good ? '#0B6E4A' : color.inkSoft;
  return (
    <View style={[st_.chip, { backgroundColor: good ? '#DCF2E6' : color.chip2 }]}>
      <Icon name={icon} size={13} tint={fg} />
      <Text style={{ fontFamily: font.semi, fontSize: 11.5, color: fg }}>{text}</Text>
    </View>
  );
}

/** The dark picture at the top: you, your reach today (30 m), and friends far away. */
function Hero({ height }: { height: number }) {
  const p1 = useLoop(3000);
  const p2 = useLoop(3000, 1500);
  const f1 = useLoop(4000);
  const f2 = useLoop(4500, 800);
  const f3 = useLoop(5000, 1600);
  const ping = (v: Animated.Value, o: number) => ({
    opacity: v.interpolate({ inputRange: [0, 1], outputRange: [o, 0] }),
    transform: [{ scale: v.interpolate({ inputRange: [0, 1], outputRange: [0.15, 1] }) }],
  });
  const float = (v: Animated.Value) => ({ transform: [{ translateY: v.interpolate({ inputRange: [0, 0.5, 1], outputRange: [0, -6, 0] }) }] });
  return (
    <View style={[st_.hero, { height }]} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      <Animated.View style={[st_.glow, ping(p1, 1)]}>
        <Glow id="g1" o={0.22} />
      </Animated.View>
      <Animated.View style={[st_.glow, ping(p2, 1)]}>
        <Glow id="g2" o={0.18} />
      </Animated.View>
      <View style={[st_.ring, { width: 86, height: 86, borderRadius: 43, marginLeft: -43, marginTop: -43, borderStyle: 'dashed', borderColor: 'rgba(255,255,255,0.28)' }]} />
      <View style={[st_.ring, { width: 220, height: 220, borderRadius: 110, marginLeft: -110, marginTop: -110, borderColor: 'rgba(205,242,92,0.35)' }]} />
      <View style={st_.me} />
      <Text style={st_.range}>30 M</Text>
      <Animated.View style={[st_.friend, { left: 18, top: Math.max(46, Math.round(height * 0.3)) }, float(f1)]}>
        <Friend letter="S" bg="#CFDDFF" fg="#2442A8" name="Sara · Lisbon" />
      </Animated.View>
      {height >= 150 ? (
        <Animated.View style={[st_.friend, { right: 16, top: Math.round(height * 0.36) }, float(f2)]}>
          <Friend letter="O" bg="#FFDCC8" fg="#9A3A0B" name="Omar · Nairobi" />
        </Animated.View>
      ) : null}
      <Animated.View style={[st_.friend, { left: 30, bottom: Math.round(height * 0.1) }, float(f3)]}>
        <Friend letter="M" bg="#D3EFE3" fg="#0C6B49" name="Mum · home" />
      </Animated.View>
      <View style={{ position: 'absolute', left: 14, top: 14, flexDirection: 'row', gap: 6 }}>
        <View style={[st_.tag, { backgroundColor: color.lime }]}>
          <Text style={[st_.tagText, { color: color.ink, fontFamily: font.monoSemi }]}>FREE</Text>
        </View>
        <View style={[st_.tag, { backgroundColor: 'rgba(255,255,255,0.12)' }]}>
          <Text style={[st_.tagText, { color: '#fff' }]}>OPTIONAL ADD-ON</Text>
        </View>
      </View>
      <View style={{ position: 'absolute', right: 14, bottom: 14, width: 40, height: 40, borderRadius: 13, backgroundColor: VIOLET_BG, alignItems: 'center', justifyContent: 'center' }}>
        <Icon name="globe" size={20} tint={VIOLET} />
      </View>
    </View>
  );
}

/** A soft lime disc fading out at 65% of its radius. */
function Glow({ id, o }: { id: string; o: number }) {
  return (
    <Svg width={420} height={420}>
      <Defs>
        <RadialGradient id={id} cx="50%" cy="50%" r="50%">
          <Stop offset="0" stopColor="#CDF25C" stopOpacity={o} />
          <Stop offset="0.65" stopColor="#CDF25C" stopOpacity={0} />
        </RadialGradient>
      </Defs>
      <Circle cx={210} cy={210} r={210} fill={`url(#${id})`} />
    </Svg>
  );
}

function Friend({ letter, bg, fg, name }: { letter: string; bg: string; fg: string; name: string }) {
  return (
    <>
      <View style={{ width: 22, height: 22, borderRadius: 8, backgroundColor: bg, alignItems: 'center', justifyContent: 'center' }}>
        <Text style={{ fontFamily: font.bold, fontSize: 10, color: fg }}>{letter}</Text>
      </View>
      <Text style={{ fontFamily: font.semi, fontSize: 11.5, color: '#fff' }}>{name}</Text>
    </>
  );
}

/** Sending a 2.9 MB photo: Bluetooth crawls, MeshGRID (Wi-Fi Direct) is done in seconds. */
function SpeedCompare() {
  const v = useLoop(4000);
  const slow = v.interpolate({ inputRange: [0, 1], outputRange: ['0%', '6%'] });
  const fast = v.interpolate({ inputRange: [0, 0.35, 1], outputRange: ['0%', '100%', '100%'] });
  const row = (icon: IconName, label: string, tint: string, w: Animated.AnimatedInterpolation<string>, time: string) => (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
      <View style={{ width: 88, flexDirection: 'row', alignItems: 'center', gap: 5 }}>
        <Icon name={icon} size={14} tint={tint} />
        <Text style={{ fontFamily: font.semi, fontSize: 12.5, color: tint }}>{label}</Text>
      </View>
      <View style={{ flex: 1, height: 8, borderRadius: 9, backgroundColor: '#E3E0D8', overflow: 'hidden' }}>
        <Animated.View style={{ height: 8, borderRadius: 9, backgroundColor: tint, width: w }} />
      </View>
      <Text style={{ width: 66, textAlign: 'right', fontFamily: font.monoSemi, fontSize: 12, color: label === 'Bluetooth' ? color.textMuted : tint }}>{time}</Text>
    </View>
  );
  return (
    <View style={st_.compare} accessible accessibilityLabel="Sending a 2.9 megabyte photo: about one and a half minutes over Bluetooth, about two seconds with MeshGRID">
      <Text style={{ fontFamily: font.monoSemi, fontSize: 11, letterSpacing: 0.7, color: color.textFaint }}>SENDING A 2.9 MB PHOTO</Text>
      {row('bt', 'Bluetooth', '#2D57E0', slow, '~1.5 min')}
      {row('wifi', 'With MeshGRID', '#08736E', fast, '~2 s')}
    </View>
  );
}

const st_ = StyleSheet.create({
  hero: { borderRadius: 24, backgroundColor: color.ink, overflow: 'hidden', marginTop: 2 },
  glow: { position: 'absolute', left: '50%', top: '58%', width: 420, height: 420, marginLeft: -210, marginTop: -210, borderRadius: 210 },
  ring: { position: 'absolute', left: '50%', top: '58%', borderWidth: 1.5 },
  me: { position: 'absolute', left: '50%', top: '58%', width: 22, height: 22, marginLeft: -11, marginTop: -11, borderRadius: 11, backgroundColor: color.lime, boxShadow: '0 0 0 6px rgba(205,242,92,0.25)' },
  range: { position: 'absolute', left: '50%', top: '58%', marginLeft: 26, marginTop: 28, fontFamily: font.monoSemi, fontSize: 10, letterSpacing: 0.6, color: 'rgba(255,255,255,0.55)' },
  friend: { position: 'absolute', flexDirection: 'row', alignItems: 'center', gap: 6, paddingVertical: 5, paddingLeft: 5, paddingRight: 10, borderRadius: 99, backgroundColor: 'rgba(255,255,255,0.1)' },
  tag: { height: 24, paddingHorizontal: 9, borderRadius: 7, justifyContent: 'center' },
  tagText: { fontFamily: font.monoSemi, fontSize: 10.5, letterSpacing: 0.6 },
  title: { fontFamily: font.display, fontSize: 24, lineHeight: 27, letterSpacing: -0.6, color: color.ink, paddingHorizontal: 4 },
  body: { fontFamily: font.regular, fontSize: 14, lineHeight: 21, color: color.textMuted, paddingHorizontal: 4 },
  compare: { backgroundColor: color.chip, borderRadius: 18, paddingVertical: 12, paddingHorizontal: 14, gap: 10 },
  chip: { flexDirection: 'row', alignItems: 'center', gap: 5, height: 28, paddingLeft: 8, paddingRight: 10, borderRadius: 99 },
  cta: { height: 54, borderRadius: 99, backgroundColor: color.lime, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 8, boxShadow: '0 10px 24px -12px rgba(90,120,0,0.7)' },
  card: { backgroundColor: '#fff', borderRadius: 24, padding: 16, gap: 12 },
  dashed: { borderWidth: 1.5, borderStyle: 'dashed', borderColor: '#C9C5BB' },
  dl: { height: 46, borderRadius: 99, backgroundColor: color.ink, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 8 },
});

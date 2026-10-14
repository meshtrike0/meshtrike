import React, { ReactNode, useEffect, useRef, useState } from 'react';
import {
  AccessibilityInfo,
  ActivityIndicator,
  Animated,
  Easing,
  PanResponder,
  Platform,
  Pressable,
  ScrollView,
  StyleProp,
  StyleSheet,
  Text,
  TextInput,
  TextInputProps,
  useWindowDimensions,
  View,
  ViewStyle,
} from 'react-native';
import Svg, { Path, Rect } from 'react-native-svg';
import { b64decode } from '../core/client';
import { QrMatrix, Trust } from '../core/types';
import { Icon, IconName } from './icons';
import { CARRIER, Carrier, color, font, radius, space, type } from './theme';

// The web harness has no native animation driver.
export const native = Platform.OS !== 'web';
export const EASE = Easing.bezier(0.2, 0.8, 0.2, 1);

// ===== Motion =====

/** Fade + rise on mount (design: fadeUp / scrIn / bubIn / pop). */
export function Appear({
  children,
  kind = 'up',
  delay = 0,
  style,
}: {
  children: ReactNode;
  kind?: 'up' | 'in' | 'right' | 'pop' | 'bubble';
  delay?: number;
  style?: StyleProp<ViewStyle>;
}) {
  const v = useRef(new Animated.Value(0)).current;
  useEffect(() => {
    const spring = kind === 'pop';
    const anim = spring
      ? Animated.spring(v, { toValue: 1, friction: 6, tension: 140, delay, useNativeDriver: native })
      : Animated.timing(v, { toValue: 1, duration: kind === 'right' ? 320 : 350, delay, easing: EASE, useNativeDriver: native });
    anim.start();
  }, [v, kind, delay]);
  const transform =
    kind === 'up' || kind === 'bubble'
      ? [{ translateY: v.interpolate({ inputRange: [0, 1], outputRange: [10, 0] }) }, ...(kind === 'bubble' ? [{ scale: v.interpolate({ inputRange: [0, 1], outputRange: [0.97, 1] }) }] : [])]
      : kind === 'right'
        ? [{ translateX: v.interpolate({ inputRange: [0, 1], outputRange: [18, 0] }) }]
        : kind === 'pop'
          ? [{ scale: v.interpolate({ inputRange: [0, 1], outputRange: [0.6, 1] }) }]
          : [];
  return <Animated.View style={[{ opacity: v, transform }, style]}>{children}</Animated.View>;
}

/**
 * The system "reduce motion" / "remove animations" setting. Ambient loops
 * stop under it (comfort, battery, and screen readers get a still screen).
 */
export function useReduceMotion() {
  const [reduce, setReduce] = useState(false);
  useEffect(() => {
    let live = true;
    AccessibilityInfo.isReduceMotionEnabled()
      .then((r) => live && setReduce(r))
      .catch(() => {});
    const sub = AccessibilityInfo.addEventListener('reduceMotionChanged', setReduce);
    return () => {
      live = false;
      sub.remove();
    };
  }, []);
  return reduce;
}

/** Repeating 0→1 driver for ambient loops (breathe, scan, spin); still under reduce motion. */
export function useLoop(ms: number, delay = 0) {
  const v = useRef(new Animated.Value(0)).current;
  const still = useReduceMotion();
  useEffect(() => {
    if (still) {
      v.setValue(0);
      return;
    }
    const a = Animated.loop(Animated.timing(v, { toValue: 1, duration: ms, delay, easing: Easing.linear, useNativeDriver: native }));
    a.start();
    return () => a.stop();
  }, [v, ms, delay, still]);
  return v;
}

/** Live "breathing" status dot. */
export function LiveDot({ tint = color.goodDot, size = 8 }: { tint?: string; size?: number }) {
  const v = useLoop(2000);
  const scale = v.interpolate({ inputRange: [0, 0.5, 1], outputRange: [1, 1.8, 1] });
  const opacity = v.interpolate({ inputRange: [0, 0.5, 1], outputRange: [1, 0.25, 1] });
  return (
    <View style={{ width: size, height: size }}>
      <View style={{ position: 'absolute', width: size, height: size, borderRadius: size, backgroundColor: tint }} />
      <Animated.View style={{ position: 'absolute', width: size, height: size, borderRadius: size, backgroundColor: tint, opacity, transform: [{ scale }] }} />
    </View>
  );
}

/** Spinner ring (design: border-top spin). */
export function Spinner({ size = 26 }: { size?: number }) {
  const v = useLoop(800);
  const rotate = v.interpolate({ inputRange: [0, 1], outputRange: ['0deg', '360deg'] });
  return (
    <Animated.View
      style={{ width: size, height: size, borderRadius: size, borderWidth: 2.5, borderColor: '#E6E3DC', borderTopColor: color.ink, transform: [{ rotate }] }}
    />
  );
}

// ===== Layout =====

export function Screen({
  title,
  onBack,
  right,
  children,
  scroll = true,
  footer,
  big,
  dark,
  backIcon = 'left',
  contentStyle,
}: {
  title?: string;
  onBack?: () => void;
  right?: ReactNode;
  children: ReactNode;
  scroll?: boolean;
  footer?: ReactNode;
  /** Tab screens: large display title, no back button, room for the floating tab bar. */
  big?: boolean;
  dark?: boolean;
  backIcon?: IconName;
  contentStyle?: StyleProp<ViewStyle>;
}) {
  const fg = dark ? '#fff' : color.ink;
  const body = (
    <View style={[{ paddingHorizontal: space.lg, paddingTop: space.xs, paddingBottom: big ? 120 : 30, gap: 14 }, contentStyle]}>{children}</View>
  );
  return (
    <Appear kind={big ? 'in' : 'right'} style={[s.screen, dark ? { backgroundColor: color.ink } : null]}>
      {big ? (
        <View style={{ paddingHorizontal: 20, paddingTop: 6, paddingBottom: 6, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' }}>
          <Text style={type.hero} accessibilityRole="header">
            {title}
          </Text>
          {right}
        </View>
      ) : (
        <View style={s.header}>
          {onBack ? (
            <Pressable onPress={onBack} accessibilityRole="button" accessibilityLabel="Back" style={s.iconBtn} hitSlop={6}>
              <Icon name={backIcon} size={24} tint={fg} />
            </Pressable>
          ) : null}
          {title ? (
            <Text style={[type.title, { flex: 1, color: fg }]} numberOfLines={1} accessibilityRole="header">
              {title}
            </Text>
          ) : (
            <View style={{ flex: 1 }} />
          )}
          {right}
        </View>
      )}
      {scroll ? (
        <ScrollView style={{ flex: 1 }} keyboardShouldPersistTaps="handled" showsVerticalScrollIndicator={false}>
          {body}
        </ScrollView>
      ) : (
        <View style={{ flex: 1 }}>{children}</View>
      )}
      {footer}
    </Appear>
  );
}

export function HeaderButton({ icon, label, onPress, tone = 'plain' }: { icon: IconName; label: string; onPress: () => void; tone?: 'plain' | 'lime' | 'soft' | 'ink' }) {
  const bg = { plain: 'transparent', lime: color.lime, soft: color.chip, ink: color.ink }[tone];
  const fg = tone === 'ink' ? color.lime : color.ink;
  const big = tone === 'lime';
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      hitSlop={6}
      style={({ pressed }) => [
        { width: big ? 48 : 40, height: big ? 48 : 40, borderRadius: big ? 17 : 14, backgroundColor: bg, alignItems: 'center', justifyContent: 'center' },
        pressed ? { transform: [{ scale: 0.94 }] } : null,
      ]}
    >
      <Icon name={icon} size={big ? 22 : 19} tint={fg} />
    </Pressable>
  );
}

export function Eyebrow({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  return (
    <View style={[{ paddingHorizontal: 6 }, style]}>
      <Text style={type.eyebrow}>{typeof children === 'string' ? children.toUpperCase() : children}</Text>
    </View>
  );
}

// ===== Controls =====

type Variant = 'primary' | 'lime' | 'secondary' | 'danger' | 'ghost' | 'light';

export function Button({
  title,
  onPress,
  variant = 'primary',
  disabled,
  busy,
  icon,
  style,
  size = 'lg',
  label,
}: {
  title: string;
  onPress: () => void;
  variant?: Variant;
  disabled?: boolean;
  busy?: boolean;
  icon?: IconName;
  style?: StyleProp<ViewStyle>;
  size?: 'lg' | 'md' | 'sm';
  label?: string;
}) {
  const bg = { primary: color.ink, lime: color.lime, secondary: color.surface, danger: color.surface, ghost: 'transparent', light: color.surface }[variant];
  const fg = { primary: '#fff', lime: color.ink, secondary: color.ink, danger: color.danger, ghost: color.ink, light: color.ink }[variant];
  const iconTint = variant === 'primary' ? color.lime : fg;
  const border = variant === 'secondary' ? color.line : variant === 'danger' ? color.dangerLine : undefined;
  const h = { lg: 56, md: 48, sm: 36 }[size];
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled || busy}
      accessibilityRole="button"
      accessibilityLabel={label ?? title}
      accessibilityState={{ disabled: !!(disabled || busy), busy: !!busy }}
      style={({ pressed }) => [
        s.button,
        { height: h, backgroundColor: bg, opacity: disabled ? 0.45 : 1, paddingHorizontal: size === 'sm' ? 14 : 22 },
        border ? { borderWidth: 1.5, borderColor: border } : null,
        pressed ? { transform: [{ scale: 0.98 }] } : null,
        style,
      ]}
    >
      {busy ? <ActivityIndicator color={fg} /> : icon ? <Icon name={icon} tint={iconTint} size={size === 'sm' ? 15 : 18} /> : null}
      <Text style={{ color: fg, fontFamily: font.semi, fontSize: size === 'lg' ? 16 : size === 'md' ? 15 : 13 }}>{title}</Text>
    </Pressable>
  );
}

export function Field(props: TextInputProps & { label: string; hint?: string; error?: string | null; mono?: boolean; hideLabel?: boolean }) {
  const { label, hint, error, style, mono, hideLabel, onFocus, onBlur, ...rest } = props;
  const [focus, setFocus] = useState(false);
  return (
    <View style={{ gap: 8, flex: style && StyleSheet.flatten(style)?.flex ? 1 : undefined }}>
      {hideLabel ? null : <Text style={{ fontFamily: font.semi, fontSize: 13, color: color.ink }}>{label}</Text>}
      <TextInput
        accessibilityLabel={label}
        placeholderTextColor={color.textFaint}
        autoCorrect={false}
        autoCapitalize="none"
        importantForAutofill="no"
        onFocus={(e) => {
          setFocus(true);
          onFocus?.(e);
        }}
        onBlur={(e) => {
          setFocus(false);
          onBlur?.(e);
        }}
        style={[
          s.input,
          mono ? { fontFamily: font.monoSemi, fontSize: 20, letterSpacing: 5 } : null,
          focus ? { borderColor: color.ink } : null,
          error ? { borderColor: color.dangerLine } : null,
          style,
        ]}
        {...rest}
      />
      {error ? <Text style={[type.small, { color: color.danger, fontSize: 12 }]}>{error}</Text> : hint ? <Text style={[type.small, { fontSize: 12 }]}>{hint}</Text> : null}
    </View>
  );
}

/** Pill switch from the design: ink track + lime knob when on. */
export function Toggle({ value, onChange, label, disabled, onDark }: { value: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean; onDark?: boolean }) {
  const v = useRef(new Animated.Value(value ? 1 : 0)).current;
  useEffect(() => {
    Animated.spring(v, { toValue: value ? 1 : 0, friction: 7, tension: 160, useNativeDriver: false }).start();
  }, [v, value]);
  // On a dark card (High-Risk): the same ink/lime switch on a pale track,
  // ringed so it stays visible against the ink.
  const bg = v.interpolate({ inputRange: [0, 1], outputRange: [onDark ? '#D6D2C9' : color.off, color.ink] });
  const knob = value ? color.lime : '#fff';
  return (
    <Pressable
      onPress={() => !disabled && onChange(!value)}
      accessibilityRole="switch"
      accessibilityLabel={label}
      accessibilityState={{ checked: value, disabled: !!disabled }}
      // React Native Web does not map accessibilityState.checked for a switch.
      aria-checked={value}
      aria-disabled={!!disabled}
      hitSlop={6}
      style={{ opacity: disabled ? 0.45 : 1 }}
    >
      <Animated.View style={[{ width: 52, height: 32, borderRadius: 99, padding: 3, backgroundColor: bg }, onDark ? { borderWidth: 1.5, borderColor: 'rgba(255,255,255,0.3)', padding: 1.5 } : null]}>
        <Animated.View
          style={{
            width: 26,
            height: 26,
            borderRadius: 13,
            backgroundColor: knob,
            transform: [{ translateX: v.interpolate({ inputRange: [0, 1], outputRange: [0, 20] }) }],
          }}
        />
      </Animated.View>
    </Pressable>
  );
}

// ===== Containers =====

export function Card({ children, style, dark }: { children: ReactNode; style?: StyleProp<ViewStyle>; dark?: boolean }) {
  return <View style={[s.card, dark ? { backgroundColor: color.ink } : null, style]}>{children}</View>;
}

export function IconTile({ icon, bg = color.chip2, fg = color.ink, size = 40, iconSize }: { icon: IconName; bg?: string; fg?: string; size?: number; iconSize?: number }) {
  return (
    <View style={{ width: size, height: size, borderRadius: Math.ceil(size / 3), backgroundColor: bg, alignItems: 'center', justifyContent: 'center' }}>
      <Icon name={icon} size={iconSize ?? Math.round(size * 0.48)} tint={fg} />
    </View>
  );
}

export function Row({
  title,
  subtitle,
  left,
  right,
  onPress,
  label,
  icon,
  danger,
  first,
}: {
  title: string;
  subtitle?: string;
  left?: ReactNode;
  right?: ReactNode;
  onPress?: () => void;
  label?: string;
  icon?: IconName;
  danger?: boolean;
  /** No divider above (first row in a card). */
  first?: boolean;
}) {
  return (
    <Pressable
      onPress={onPress}
      disabled={!onPress}
      accessibilityRole={onPress ? 'button' : undefined}
      accessibilityLabel={label ?? [title, subtitle].filter(Boolean).join(', ')}
      style={({ pressed }) => [s.row, first ? null : s.rowLine, pressed && onPress ? { opacity: 0.6 } : null]}
    >
      {left ?? (icon ? <Icon name={icon} size={19} tint={danger ? color.danger : color.ink} /> : null)}
      <View style={{ flex: 1, gap: 2 }}>
        <Text style={{ fontFamily: font.medium, fontSize: 14.5, color: danger ? color.danger : color.ink }} numberOfLines={1}>
          {title}
        </Text>
        {subtitle ? (
          <Text style={{ fontFamily: font.regular, fontSize: 12, lineHeight: 16, color: color.textMuted }} numberOfLines={2}>
            {subtitle}
          </Text>
        ) : null}
      </View>
      {right}
    </Pressable>
  );
}

export function ToggleRow({
  title,
  subtitle,
  value,
  onChange,
  disabled,
  icon,
  first,
}: {
  title: string;
  subtitle?: string;
  value: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  icon?: IconName;
  first?: boolean;
}) {
  return (
    <Row
      title={title}
      subtitle={subtitle}
      icon={icon}
      first={first}
      label={`${title}, ${value ? 'on' : 'off'}`}
      // The whole row toggles: a bigger target.
      onPress={disabled ? undefined : () => onChange(!value)}
      right={<Toggle value={value} onChange={onChange} disabled={disabled} label={title} />}
    />
  );
}

export function Section({ title, children, style }: { title?: string; children: ReactNode; style?: StyleProp<ViewStyle> }) {
  const items = React.Children.toArray(children).filter(Boolean);
  return (
    <View style={{ gap: 8 }}>
      {title ? <Eyebrow>{title}</Eyebrow> : null}
      <View style={[s.section, style]}>
        {items.map((c, i) => (React.isValidElement(c) && i === 0 && typeof c.type !== 'string' ? React.cloneElement(c as React.ReactElement<{ first?: boolean }>, { first: true }) : c))}
      </View>
    </View>
  );
}

// ===== Identity =====

function fnv(s: string) {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const AVATAR_PALETTE: [string, string][] = [
  ['#D9F57E', '#2F4A00'],
  ['#CFDDFF', '#2442A8'],
  ['#FFDCC8', '#9A3A0B'],
  ['#D3EFE3', '#0C6B49'],
  ['#E6DBFF', '#5A33B8'],
  ['#FFE8A8', '#7A5200'],
];

/** Identicon from a stable seed (identity key, conversation id). People have no photos. */
export function Avatar({ seed, size = 44, kind = 'person' }: { seed: string; size?: number; kind?: 'person' | 'group' | 'channel' | 'me' }) {
  const h = fnv(seed);
  const [bg, fg] = AVATAR_PALETTE[h % AVATAR_PALETTE.length];
  const box: ViewStyle = { width: size, height: size, borderRadius: Math.round(size * 0.34), alignItems: 'center', justifyContent: 'center', overflow: 'hidden' };
  if (kind === 'group') {
    return (
      <View style={[box, { backgroundColor: bg }]} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
        <Icon name="users" size={size * 0.46} tint={fg} />
      </View>
    );
  }
  if (kind === 'channel') {
    return (
      <View style={[box, { backgroundColor: color.ink }]} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
        <Icon name="hash" size={size * 0.46} tint={color.lime} />
      </View>
    );
  }
  const r = rng(h);
  const cells: [number, number][] = [];
  for (let y = 0; y < 5; y++)
    for (let x = 0; x < 3; x++) {
      if (r() > 0.48) {
        cells.push([x, y]);
        if (x < 2) cells.push([4 - x, y]);
      }
    }
  return (
    <View style={[box, { backgroundColor: bg }]} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      <Svg width={size * 0.6} height={size * 0.6} viewBox="0 0 5 5">
        {cells.map(([x, y], i) => (
          <Rect key={i} x={x + 0.06} y={y + 0.06} width={0.88} height={0.88} rx={0.22} fill={fg} />
        ))}
      </Svg>
    </View>
  );
}

const TRUST = {
  Verified: { label: 'Verified', icon: 'shieldOk' as const, fg: color.good, bg: color.goodBg, bd: color.goodBg, dash: false },
  Unverified: { label: 'Not verified', icon: 'shield' as const, fg: color.grey, bg: '#F3F1EC', bd: color.disabled, dash: true },
  KeyChanged: { label: 'Key changed', icon: 'shieldX' as const, fg: color.danger, bg: color.dangerBg, bd: color.dangerBg, dash: false },
};

/** Security state: shaped icon + words, never colour alone (section 17). */
export function TrustBadge({ trust, compact, large }: { trust: Trust; compact?: boolean; large?: boolean }) {
  const t = TRUST[trust];
  const lg = !!large && !compact;
  return (
    <View
      accessibilityRole="image"
      accessibilityLabel={`Trust: ${t.label}`}
      style={{
        flexDirection: 'row',
        alignItems: 'center',
        // Large badges sit in centred layouts (paired, contact detail).
        alignSelf: lg ? 'center' : 'flex-start',
        gap: lg ? 6 : 4,
        paddingVertical: lg ? 7 : 3,
        paddingLeft: lg ? 10 : 6,
        paddingRight: lg ? 13 : 8,
        borderRadius: 99,
        backgroundColor: t.bg,
        borderWidth: 1.5,
        borderColor: t.bd,
        borderStyle: t.dash ? 'dashed' : 'solid',
      }}
    >
      <Icon name={t.icon} size={lg ? 17 : 13} tint={t.fg} />
      <Text style={{ fontFamily: font.semi, fontSize: lg ? 14 : 11.5, color: t.fg }}>{t.label}</Text>
    </View>
  );
}

export function TrustGlyph({ trust }: { trust: Trust }) {
  const t = TRUST[trust];
  return (
    <View accessibilityRole="image" accessibilityLabel={t.label}>
      <Icon name={t.icon} size={15} tint={t.fg} />
    </View>
  );
}

export function Pill({ icon, text, tint = color.textMuted, bg = color.chip }: { icon?: IconName; text: string; tint?: string; bg?: string }) {
  return (
    <View style={[s.badge, { backgroundColor: bg }]} accessibilityLabel={text}>
      {icon ? <Icon name={icon} tint={tint} size={13} /> : null}
      <Text style={{ color: tint, fontSize: 11.5, fontFamily: font.semi }}>{text}</Text>
    </View>
  );
}

export function Tag({ text, tone = 'ink' }: { text: string; tone?: 'ink' | 'soft' }) {
  return (
    <View style={{ paddingHorizontal: 6, paddingVertical: 3, borderRadius: 6, backgroundColor: tone === 'ink' ? color.ink : color.chip2 }}>
      <Text style={{ fontFamily: font.monoSemi, fontSize: 10.5, letterSpacing: 0.4, color: tone === 'ink' ? color.lime : color.inkSoft }}>{text}</Text>
    </View>
  );
}

/** How messages to this conversation travel right now; morphs when the carrier changes. */
export function CarrierPill({ carrier, highRisk }: { carrier: Carrier; highRisk?: boolean }) {
  const c = CARRIER[carrier];
  const label = highRisk && carrier === 'bt' ? 'Nearby · Bluetooth · internet off' : c.label;
  const v = useRef(new Animated.Value(1)).current;
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    v.setValue(0);
    Animated.timing(v, { toValue: 1, duration: 360, easing: EASE, useNativeDriver: native }).start();
  }, [v, carrier, highRisk]);
  return (
    <View
      accessibilityRole="text"
      accessibilityLabel={`Link: ${label}`}
      accessibilityLiveRegion="polite"
      style={{ height: 30, borderRadius: 99, backgroundColor: c.bg, flexDirection: 'row', alignItems: 'center', gap: 7, paddingLeft: 10, paddingRight: 13, overflow: 'hidden', alignSelf: 'flex-start' }}
    >
      <Animated.View style={{ transform: [{ translateY: v.interpolate({ inputRange: [0, 1], outputRange: [22, 0] }) }], opacity: v }}>
        <Icon name={c.icon} size={15} tint={c.fg} />
      </Animated.View>
      <Animated.Text
        style={{
          fontFamily: font.semi,
          fontSize: 12.5,
          color: c.fg,
          opacity: v,
          transform: [{ translateX: v.interpolate({ inputRange: [0, 1], outputRange: [-8, 0] }) }],
        }}
        numberOfLines={1}
      >
        {label}
      </Animated.Text>
    </View>
  );
}

// ===== Feedback =====

export function Banner({
  tone,
  icon,
  title,
  text,
  action,
}: {
  tone: 'warning' | 'danger' | 'info' | 'dark';
  icon: IconName;
  title?: string;
  text: string;
  action?: ReactNode;
}) {
  const bg = { warning: color.warnBg, danger: color.dangerBg, info: color.chip2, dark: color.ink }[tone];
  const fg = { warning: color.warnInk, danger: color.dangerDeep, info: color.inkSoft, dark: 'rgba(255,255,255,0.75)' }[tone];
  const head = tone === 'dark' ? '#fff' : color.ink;
  const tileBg = { warning: color.ink, danger: color.surface, info: color.surface, dark: color.dangerStrong }[tone];
  const tileFg = { warning: color.warnBright, danger: color.danger, info: color.ink, dark: '#fff' }[tone];
  return (
    <Appear kind="up">
      <View style={[s.banner, { backgroundColor: bg }]} accessibilityRole="alert">
        {title ? <IconTile icon={icon} bg={tileBg} fg={tileFg} size={38} /> : <Icon name={icon} tint={tone === 'dark' ? color.lime : fg} size={18} />}
        <View style={{ flex: 1, gap: 2 }}>
          {title ? <Text style={{ fontFamily: font.semi, fontSize: 14.5, color: head }}>{title}</Text> : null}
          <Text style={{ fontFamily: title ? font.regular : font.medium, fontSize: title ? 12.5 : 13, lineHeight: 18, color: title ? fg : tone === 'dark' ? '#fff' : fg }}>{text}</Text>
        </View>
        {action}
      </View>
    </Appear>
  );
}

export function ErrorText({ error }: { error: string | null }) {
  if (!error) return null;
  return (
    <Appear kind="up">
      <View accessibilityRole="alert" style={{ flexDirection: 'row', gap: 10, alignItems: 'flex-start', backgroundColor: color.dangerBg, borderRadius: 14, paddingVertical: 12, paddingHorizontal: 14 }}>
        <Icon name="alert" size={17} tint={color.dangerDeep} />
        <Text style={{ flex: 1, fontFamily: font.medium, fontSize: 13.5, lineHeight: 19, color: color.dangerDeep }}>{error}</Text>
      </View>
    </Appear>
  );
}

export function Empty({ icon, title, text, action }: { icon: IconName; title: string; text: string; action?: ReactNode }) {
  return (
    <Appear kind="up" style={{ alignItems: 'center', gap: 12, paddingVertical: 32, paddingHorizontal: 24 }}>
      <IconTile icon={icon} size={64} bg={color.chip2} />
      <Text style={[type.heading, { fontSize: 22, textAlign: 'center' }]}>{title}</Text>
      <Text style={[type.small, { textAlign: 'center', fontSize: 14, lineHeight: 20, maxWidth: 280 }]}>{text}</Text>
      {action}
    </Appear>
  );
}

// ===== QR, PIN, hold =====

export function QrView({ matrix, size = 240 }: { matrix: QrMatrix; size?: number }) {
  const modules = b64decode(matrix.modules);
  const n = matrix.size;
  let d = '';
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      if (modules[y * n + x]) d += `M${x + 2} ${y + 2}h1v1h-1z`;
    }
  }
  return (
    <View accessibilityLabel="QR code" style={{ backgroundColor: '#fff', alignSelf: 'center' }}>
      <Svg width={size} height={size} viewBox={`0 0 ${n + 4} ${n + 4}`}>
        <Rect x={0} y={0} width={n + 4} height={n + 4} fill="#fff" />
        <Path d={d} fill={color.ink} />
      </Svg>
    </View>
  );
}

/** One PIN dot: fills with a 180 ms overshoot (design: cubic-bezier(.3,1.4,.5,1)). */
function PinDot({ on }: { on: boolean }) {
  const v = useRef(new Animated.Value(on ? 1 : 0)).current;
  useEffect(() => {
    // Colour cannot use the native driver, so the whole dot animates in JS.
    Animated.timing(v, { toValue: on ? 1 : 0, duration: 180, easing: Easing.bezier(0.3, 1.4, 0.5, 1), useNativeDriver: false }).start();
  }, [on, v]);
  const fill = v.interpolate({ inputRange: [0, 1], outputRange: ['rgba(21,23,28,0)', color.ink], extrapolate: 'clamp' });
  return (
    <Animated.View
      style={[
        s.pinDot,
        {
          borderColor: v.interpolate({ inputRange: [0, 1], outputRange: [color.disabled, color.ink], extrapolate: 'clamp' }),
          backgroundColor: fill,
          transform: [{ scale: v.interpolate({ inputRange: [0, 1], outputRange: [0.82, 1] }) }],
        },
      ]}
    />
  );
}

/**
 * Keypad from the design: white round digits, fingerprint and delete keys.
 * `split` lays the dots at the top and the keys at the bottom with flexible
 * space between (the unlock screen); the default stacks them together.
 */
export function PinPad({
  length = 6,
  onComplete,
  disabled,
  onBiometric,
  shake = 0,
  split,
}: {
  length?: number;
  onComplete: (pin: string) => void;
  disabled?: boolean;
  onBiometric?: () => void;
  /** Bump to shake the dots (wrong PIN). */
  shake?: number;
  /** Dots on top, keys pinned to the bottom; fills the parent's remaining height. */
  split?: boolean;
}) {
  const [pin, setPin] = useState('');
  const x = useRef(new Animated.Value(0)).current;
  useEffect(() => {
    if (!shake) return;
    Animated.sequence(
      [-10, 9, -6, 4, 0].map((toValue) => Animated.timing(x, { toValue, duration: 80, useNativeDriver: native })),
    ).start();
  }, [shake, x]);
  const press = (k: string) => {
    if (disabled) return;
    if (k === 'del') return setPin((p) => p.slice(0, -1));
    if (k === 'bio') return onBiometric?.();
    // A full PIN is already on its way: extra taps must not submit it twice.
    if (pin.length >= length) return;
    const next = pin + k;
    setPin(next);
    if (next.length === length) {
      // Let the last dot fill before checking (design: 260 ms).
      setTimeout(() => {
        onComplete(next);
        setPin('');
      }, 260);
    }
  };
  const keys = ['1', '2', '3', '4', '5', '6', '7', '8', '9', onBiometric ? 'bio' : '', '0', 'del'];
  const dots = (
    <Animated.View
      style={[s.pinDots, split ? { marginTop: 14 } : null, { transform: [{ translateX: x }] }]}
      accessibilityLabel={`${pin.length} of ${length} digits entered`}
    >
      {Array.from({ length }).map((_, i) => (
        <PinDot key={i} on={i < pin.length} />
      ))}
    </Animated.View>
  );
  const pad = (
    <View style={s.pinKeys}>
      {keys.map((k, i) =>
        k ? (
          <Pressable
            key={i}
            onPress={() => press(k)}
            accessibilityRole="button"
            accessibilityLabel={k === 'del' ? 'Delete digit' : k === 'bio' ? 'Use fingerprint' : `Digit ${k}`}
            style={({ pressed }) => [s.key, { backgroundColor: pressed ? '#E6E3DC' : /\d/.test(k) ? '#fff' : 'transparent' }]}
          >
            {k === 'del' ? (
              <Icon name="del" size={26} />
            ) : k === 'bio' ? (
              <Icon name="finger" size={28} />
            ) : (
              <Text style={{ color: color.ink, fontSize: 30, fontFamily: font.medium }}>{k}</Text>
            )}
          </Pressable>
        ) : (
          <View key={i} style={{ width: 76, height: 76 }} />
        ),
      )}
    </View>
  );
  return split ? (
    <View style={{ flex: 1, alignItems: 'center', alignSelf: 'stretch' }}>
      {dots}
      <View style={{ flex: 1 }} />
      {pad}
    </View>
  ) : (
    <View style={{ alignItems: 'center', gap: 28, width: '100%' }}>
      {dots}
      {pad}
    </View>
  );
}

/** Press-and-hold confirmation for destructive actions (panic wipe: 3 s). */
export function HoldButton({
  title,
  ms,
  onDone,
  icon = 'trash',
  tone = 'danger',
}: {
  title: string;
  ms: number;
  onDone: () => void;
  icon?: IconName;
  /** danger: filled peach (panic wipe). outline: white with orange border (remove contact). */
  tone?: 'danger' | 'outline';
}) {
  const p = useRef(new Animated.Value(0)).current;
  const anim = useRef<Animated.CompositeAnimation | null>(null);
  const [holding, setHolding] = useState(false);
  const start = () => {
    setHolding(true);
    anim.current = Animated.timing(p, { toValue: 1, duration: ms, easing: Easing.linear, useNativeDriver: false });
    anim.current.start(({ finished }) => {
      if (finished) {
        p.setValue(0);
        setHolding(false);
        onDone();
      }
    });
  };
  const stop = () => {
    anim.current?.stop();
    setHolding(false);
    Animated.timing(p, { toValue: 0, duration: 180, useNativeDriver: false }).start();
  };
  const filled = tone === 'danger';
  return (
    <Pressable
      onPressIn={start}
      onPressOut={stop}
      accessibilityRole="button"
      accessibilityLabel={`${title}. Press and hold for ${ms / 1000} seconds`}
      style={[
        s.button,
        { height: filled ? 60 : 54, overflow: 'hidden', backgroundColor: filled ? color.dangerBg : '#fff' },
        filled ? null : { borderWidth: 1.5, borderColor: color.dangerLine },
      ]}
    >
      <Animated.View
        style={[StyleSheet.absoluteFill, { width: p.interpolate({ inputRange: [0, 1], outputRange: ['0%', '100%'] }), backgroundColor: filled ? color.dangerStrong : color.dangerBg }]}
      />
      <Icon name={icon} tint={filled ? color.dangerDeep : color.danger} size={18} />
      <Text style={{ color: filled ? color.dangerDeep : color.danger, fontFamily: font.semi, fontSize: 15.5 }}>{holding ? 'Keep holding…' : title}</Text>
    </Pressable>
  );
}

/** Segmented control (radio mode, pair tabs). */
export function Choice<T extends string>({ options, value, onChange }: { options: { value: T; label: string }[]; value: T; onChange: (v: T) => void }) {
  return (
    <View style={{ flexDirection: 'row', backgroundColor: color.chip2, borderRadius: 14, padding: 4 }} accessibilityRole="radiogroup">
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
            style={[
              { flex: 1, minHeight: 38, borderRadius: 11, alignItems: 'center', justifyContent: 'center', paddingHorizontal: 6 },
              on ? { backgroundColor: '#fff', shadowColor: '#000', shadowOpacity: 0.12, shadowRadius: 3, shadowOffset: { width: 0, height: 1 }, elevation: 1 } : null,
            ]}
          >
            <Text style={{ color: color.ink, fontFamily: on ? font.semi : font.medium, fontSize: 13 }}>{o.label}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

/** Option grid inside sheets (timers). */
export function OptionGrid<T extends string | number>({ options, value, onChange, columns = 2 }: { options: { value: T; label: string }[]; value?: T; onChange: (v: T) => void; columns?: number }) {
  return (
    <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }} accessibilityRole="radiogroup">
      {options.map((o) => {
        const on = o.value === value;
        return (
          <Pressable
            key={String(o.value)}
            onPress={() => onChange(o.value)}
            accessibilityRole="radio"
            accessibilityState={{ checked: on }}
            aria-checked={on}
            accessibilityLabel={o.label}
            style={{
              width: `${100 / columns - 2}%`,
              flexGrow: 1,
              height: 50,
              borderRadius: 16,
              backgroundColor: on ? color.ink : color.chip,
              flexDirection: 'row',
              alignItems: 'center',
              justifyContent: 'center',
              gap: 6,
            }}
          >
            {on ? <Icon name="check" size={15} tint={color.lime} /> : null}
            <Text style={{ fontFamily: font.semi, fontSize: 14, color: on ? '#fff' : color.ink }}>{o.label}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

// ===== Sheet =====

/** Bottom sheet with scrim. Render as the last child of a full-screen view. */
/**
 * A bottom sheet: anchored to the bottom edge, never taller than ~88% of the
 * screen (the dimmed screen stays visible above it), its content scrolls when
 * it does not fit, and the handle can be dragged down to close it.
 */
export function Sheet({ open, onClose, title, subtitle, children }: { open: boolean; onClose: () => void; title?: string; subtitle?: string; children: ReactNode }) {
  const v = useRef(new Animated.Value(0)).current;
  const drag = useRef(new Animated.Value(0)).current;
  const [mounted, setMounted] = useState(open);
  const { height } = useWindowDimensions();
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    if (open) {
      setMounted(true);
      drag.setValue(0);
    }
    Animated.timing(v, { toValue: open ? 1 : 0, duration: open ? 380 : 260, easing: EASE, useNativeDriver: native }).start(({ finished }) => {
      if (finished && !open) setMounted(false);
    });
  }, [open, v, drag]);
  const pan = useRef(
    PanResponder.create({
      onStartShouldSetPanResponder: () => true,
      onMoveShouldSetPanResponder: (_, g) => g.dy > 4,
      onPanResponderMove: (_, g) => drag.setValue(Math.max(0, g.dy)),
      onPanResponderRelease: (_, g) => {
        if (g.dy > 110 || g.vy > 1.1) close.current();
        else Animated.spring(drag, { toValue: 0, friction: 8, useNativeDriver: native }).start();
      },
      onPanResponderTerminate: () => Animated.spring(drag, { toValue: 0, friction: 8, useNativeDriver: native }).start(),
    }),
  ).current;
  if (!mounted) return null;
  return (
    <View style={[StyleSheet.absoluteFill, { zIndex: 50 }]} pointerEvents={open ? 'auto' : 'none'}>
      <Animated.View style={[StyleSheet.absoluteFill, { backgroundColor: color.overlay, opacity: v }]}>
        <Pressable style={{ flex: 1 }} onPress={onClose} accessibilityRole="button" accessibilityLabel="Close" />
      </Animated.View>
      <Animated.View
        accessibilityViewIsModal
        style={[
          s.sheet,
          { maxHeight: Math.round(height * 0.88) },
          { transform: [{ translateY: Animated.add(v.interpolate({ inputRange: [0, 1], outputRange: [height, 0] }), drag) }] },
        ]}
      >
        <View {...pan.panHandlers} accessibilityRole="adjustable" accessibilityLabel="Drag down to close" style={s.sheetGrip}>
          <View style={{ width: 40, height: 5, borderRadius: 9, backgroundColor: color.line }} />
        </View>
        <ScrollView style={{ flexShrink: 1 }} bounces={false} showsVerticalScrollIndicator={false} keyboardShouldPersistTaps="handled" contentContainerStyle={s.sheetBody}>
          {title ? <Text style={[type.heading, { paddingHorizontal: 4 }]}>{title}</Text> : null}
          {subtitle ? <Text style={[type.small, { paddingHorizontal: 4 }]}>{subtitle}</Text> : null}
          {children}
        </ScrollView>
      </Animated.View>
    </View>
  );
}

export function SheetAction({ icon, label, onPress, danger, hint }: { icon: IconName; label: string; onPress: () => void; danger?: boolean; hint?: string }) {
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={({ pressed }) => ({ height: 52, borderRadius: 16, paddingHorizontal: 12, flexDirection: 'row', alignItems: 'center', gap: 12, backgroundColor: pressed ? (danger ? color.dangerBg : color.chip) : 'transparent' })}
    >
      <Icon name={icon} size={19} tint={danger ? color.danger : color.ink} />
      <Text style={{ fontFamily: font.semi, fontSize: 15, color: danger ? color.danger : color.ink }}>{label}</Text>
      {hint ? <Text style={{ marginLeft: 'auto', fontFamily: font.medium, fontSize: 12, color: color.textFaint }}>{hint}</Text> : null}
    </Pressable>
  );
}

export const s = StyleSheet.create({
  screen: { flex: 1, backgroundColor: color.bg },
  header: { flexDirection: 'row', alignItems: 'center', gap: 6, paddingLeft: 6, paddingRight: 14, paddingTop: 4, paddingBottom: 8, minHeight: 52 },
  iconBtn: { width: 44, height: 44, alignItems: 'center', justifyContent: 'center' },
  button: { flexDirection: 'row', gap: 8, alignItems: 'center', justifyContent: 'center', borderRadius: 99 },
  input: {
    height: 54,
    backgroundColor: '#fff',
    borderColor: color.line,
    borderWidth: 1.5,
    borderRadius: 16,
    paddingHorizontal: 16,
    color: color.ink,
    fontSize: 16,
    fontFamily: font.medium,
    // Web only: the border change shows focus; drop the browser's blue ring.
    ...(Platform.OS === 'web' ? ({ outlineStyle: 'none' } as object) : null),
  },
  card: { backgroundColor: '#fff', borderRadius: 24, padding: 16, gap: 12 },
  row: { flexDirection: 'row', alignItems: 'center', gap: 12, paddingVertical: 13, minHeight: 52 },
  rowLine: { borderTopWidth: 1, borderTopColor: color.lineSoft },
  section: { backgroundColor: '#fff', borderRadius: 24, paddingHorizontal: 16, paddingVertical: 2 },
  badge: { flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 8, paddingVertical: 3, borderRadius: 99, alignSelf: 'flex-start' },
  banner: { flexDirection: 'row', alignItems: 'center', gap: 12, padding: 14, borderRadius: 20 },
  key: { width: 76, height: 76, borderRadius: 38, alignItems: 'center', justifyContent: 'center' },
  pinDot: { width: 16, height: 16, borderRadius: 8, borderWidth: 2 },
  pinDots: { flexDirection: 'row', gap: 14 },
  pinKeys: { flexDirection: 'row', flexWrap: 'wrap', width: 3 * 76 + 2 * 26, columnGap: 26, rowGap: 14 },
  sheet: {
    position: 'absolute',
    left: 0,
    right: 0,
    bottom: 0,
    backgroundColor: '#fff',
    borderTopLeftRadius: 28,
    borderTopRightRadius: 28,
    shadowColor: '#000',
    shadowOpacity: 0.18,
    shadowRadius: 30,
    elevation: 12,
  },
  // The grip is a generous drag target above the scrolling content.
  sheetGrip: { alignItems: 'center', justifyContent: 'center', paddingTop: 10, paddingBottom: 6 },
  sheetBody: { paddingHorizontal: 16, paddingTop: 6, paddingBottom: 28, gap: 12 },
});

// Design tokens for the light redesign (Meshtrike.dc.html). One source of truth for
// colour, type, spacing and radius. Security states always pair colour with an
// icon and words, never colour alone (section 17).

export const ink = '#15171C';
export const lime = '#CDF25C';

export const color = {
  // Surfaces
  bg: '#F6F5F1',
  sand: '#E9E6DF',
  surface: '#FFFFFF',
  chip: '#F4F2ED',
  chip2: '#EEEBE4',
  sunk: '#EDEBE5',
  line: '#DDD9D0',
  lineSoft: '#F0EDE7',
  border: '#E8E5DE',
  dash: '#C9C5BB',
  off: '#D6D2C9',
  // Ink
  ink,
  inkSoft: '#41454D',
  text: ink,
  textMuted: '#5D616B',
  textFaint: '#7A766D',
  grey: '#4E525B',
  disabled: '#B5B1A7',
  // Accent
  lime,
  limeInk: '#3E5200',
  limeGlow: 'rgba(205,242,92,0.35)',
  // Semantic
  good: '#0B6E4A',
  goodBg: '#DCF2E6',
  goodDot: '#22A36B',
  danger: '#A3360A',
  dangerBg: '#FDE4D5',
  dangerLine: '#E0703B',
  dangerStrong: '#C2410C',
  dangerDeep: '#7A2806',
  warnBg: '#FFF1C9',
  warnInk: '#5A4A12',
  warnIcon: '#7A5200',
  warnBright: '#FFD54A',
  purpleBg: '#E6DBFF',
  purple: '#5A33B8',
  overlay: 'rgba(21,23,28,0.38)',
  scrim: 'rgba(21,23,28,0.45)',
  onInkMuted: 'rgba(255,255,255,0.72)',
  // Names used before the redesign, mapped onto the new palette.
  surfaceAlt: '#F4F2ED',
  raised: '#FFFFFF',
  primary: ink,
  primaryText: '#FFFFFF',
  accent: ink,
  verified: '#0B6E4A',
  unverified: '#4E525B',
  dangerText: '#FFFFFF',
  warning: '#7A5200',
  warningBg: '#FFF1C9',
  infoBg: '#EEEBE4',
  bubbleOut: ink,
  bubbleIn: '#FFFFFF',
} as const;

export const font = {
  display: 'BricolageGrotesque_700Bold',
  displaySemi: 'BricolageGrotesque_600SemiBold',
  regular: 'Geist_400Regular',
  medium: 'Geist_500Medium',
  semi: 'Geist_600SemiBold',
  bold: 'Geist_700Bold',
  mono: 'GeistMono_500Medium',
  monoSemi: 'GeistMono_600SemiBold',
} as const;

export const space = { xs: 4, sm: 8, md: 12, lg: 16, xl: 24, xxl: 32 } as const;
export const radius = { sm: 10, md: 16, lg: 22, xl: 26, pill: 999 } as const;

export const type = {
  // Big screen titles ("Chats", "Network", "Settings").
  hero: { fontFamily: font.display, fontSize: 36, lineHeight: 38, letterSpacing: -1.2, color: ink },
  // Pushed-screen titles ("New chat", "Add in person").
  title: { fontFamily: font.display, fontSize: 24, letterSpacing: -0.5, color: ink },
  display: { fontFamily: font.display, fontSize: 32, lineHeight: 34, letterSpacing: -1, color: ink },
  heading: { fontFamily: font.display, fontSize: 20, color: ink },
  label: { fontFamily: font.semi, fontSize: 15, color: ink },
  body: { fontFamily: font.regular, fontSize: 15.5, lineHeight: 22, color: ink },
  small: { fontFamily: font.regular, fontSize: 13, lineHeight: 18, color: '#5D616B' },
  tiny: { fontFamily: font.medium, fontSize: 11.5, color: '#7A766D' },
  eyebrow: { fontFamily: font.monoSemi, fontSize: 11, letterSpacing: 0.9, color: '#7A766D' },
  mono: { fontFamily: font.monoSemi, fontSize: 15, color: ink, letterSpacing: 1 },
};

export type Carrier = 'bt' | 'wifi' | 'lora' | 'qr' | 'net' | 'mesh' | 'none';

/** How a conversation's messages travel right now (header link pill, Network). */
export const CARRIER: Record<Carrier, { label: string; short: string; icon: 'bt' | 'wifi' | 'radio' | 'qr' | 'globe' | 'mesh' | 'clock'; fg: string; bg: string }> = {
  bt: { label: 'Nearby · Bluetooth', short: 'Bluetooth', icon: 'bt', fg: '#2D57E0', bg: '#E4EBFF' },
  wifi: { label: 'Nearby · Wi-Fi (fast)', short: 'Wi-Fi', icon: 'wifi', fg: '#08736E', bg: '#D9F3EF' },
  lora: { label: 'Long range · LoRa', short: 'LoRa', icon: 'radio', fg: '#B13F0A', bg: '#FDE7D9' },
  qr: { label: 'Camera handoff', short: 'QR', icon: 'qr', fg: ink, bg: '#E9E6DF' },
  net: { label: 'Internet · private route', short: 'Internet', icon: 'globe', fg: '#6236D0', bg: '#ECE5FF' },
  mesh: { label: 'Carried by the mesh', short: 'Mesh', icon: 'mesh', fg: '#4E525B', bg: '#EDEBE6' },
  none: { label: 'Waiting for a nearby phone', short: 'Waiting', icon: 'clock', fg: '#4E525B', bg: '#EDEBE6' },
};

/** A live link kind (Contact.link) as a carrier. */
export const LINK_CARRIER: Record<'Ble' | 'WifiP2p' | 'Lora', Carrier> = { Ble: 'bt', WifiP2p: 'wifi', Lora: 'lora' };

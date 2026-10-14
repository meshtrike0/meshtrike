import React, { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { Animated, Easing, FlatList, Modal, PanResponder, Platform, Pressable, StyleSheet, Text, TextInput, Vibration, useWindowDimensions, View } from 'react-native';
import Svg, { Defs, LinearGradient, Pattern, Rect, Stop } from 'react-native-svg';
import { meshCoreNative } from '../../../modules/mesh-core/src/MeshCoreModule';
import { b64decode, b64encode } from '../../core/client';
import { ConversationSummary, DeliveryState, GroupMember, Message, RgbaImage, SendWhen, Via } from '../../core/types';
import { Icon, IconName } from '../icons';
import { Appear, Avatar, Button, CarrierPill, EASE, ErrorText, IconTile, native, OptionGrid, Sheet, SheetAction, Spinner, TrustBadge, useLoop, useReduceMotion } from '../kit';
import { RgbaView } from '../RgbaView';
import { DISAPPEAR_OPTIONS, disappearLabel, disappearShort, errorText, timeAgo, useApp, useCoreEvents } from '../state';
import { CARRIER, Carrier, color, font, LINK_CARRIER, type } from '../theme';

const STATE: Partial<Record<DeliveryState, { icon: IconName; label: string }>> = {
  Held: { icon: 'pause', label: 'Held' },
  Queued: { icon: 'clock', label: 'Waiting' },
  Sent: { icon: 'check', label: 'Sent' },
  Delivered: { icon: 'checks', label: 'Delivered' },
  Read: { icon: 'checks', label: 'Read' },
  Failed: { icon: 'alertC', label: 'Not delivered' },
};

const LATER: { value: SendWhen; label: string }[] = [
  { value: 'In10Min', label: 'In 10 minutes' },
  { value: 'In30Min', label: 'In 30 minutes' },
  { value: 'In2Hours', label: 'In 2 hours' },
  { value: 'Hold', label: 'Hold until I release' },
];

// Core-recorded carrier of a message (Message.via) → display key and words.
const VIA_CARRIER: Record<Via, Carrier> = { Ble: 'bt', WifiP2p: 'wifi', Lora: 'lora', Qr: 'qr', Internet: 'net' };
const VIA_LABEL: Record<Via, string> = { Ble: 'Bluetooth', WifiP2p: 'Wi-Fi', Lora: 'LoRa', Qr: 'QR handoff', Internet: 'the internet route' };
// Live link to a contact (Contact.link) → header carrier.

/** Factual words for a change of the header carrier while the chat is open. */
function carrierEventText(to: Carrier, direct: boolean, from: Carrier | null): string {
  switch (to) {
    case 'bt':
      return from === 'wifi' || from === 'mesh' || from === 'none' ? 'Back on Bluetooth' : 'Switched to Bluetooth';
    case 'wifi':
      return 'Switched to Wi-Fi for faster transfer';
    case 'lora':
      return 'Switched to LoRa, long range';
    case 'mesh':
      return direct ? 'Recipient out of range · carried by the mesh' : 'Other phones nearby will carry messages';
    case 'net':
      return 'Switched to the private internet route';
    case 'qr':
      return 'Switched to camera handoff';
    default:
      return 'No phones nearby · messages wait';
  }
}

type Sheets = null | 'attach' | 'timer' | 'later' | 'actions' | 'info' | 'mic' | 'micoff' | 'hrcall' | 'video' | 'routes';
type CarrierEvent = { id: string; at: number; carrier: Carrier; text: string };
type Item =
  | { k: 'msg'; m: Message }
  | { k: 'carr'; e: CarrierEvent }
  | { k: 'day'; id: string; label: string }
  | { k: 'prep'; name: string; size: number };

/** "Today", "Yesterday", or the date, for the divider above a day's messages. */
function dayLabel(ms: number) {
  const d = new Date(ms);
  const today = new Date();
  const y = new Date(today);
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return 'Today';
  if (d.toDateString() === y.toDateString()) return 'Yesterday';
  return d.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
}

/** The internet add-on is installed (Android; elsewhere the route is simulated). */
function readAddon(): boolean {
  if (Platform.OS !== 'android') return true;
  try {
    return meshCoreNative().internetAddonStatus().installed;
  } catch {
    return false;
  }
}

/** The rows of "How messages reach …": each way a message can travel, and whether it works now. */
function routeRows(o: {
  isDm: boolean;
  isChannel: boolean;
  first: string;
  link: string | null;
  highRisk: boolean;
  addon: boolean;
  netOn: boolean;
  netOnline: boolean;
  theirAddress: boolean;
  neighbours: number;
}) {
  const rows: { name: string; desc: string; ok: boolean }[] = [];
  if (o.isDm) {
    const near = o.link === 'Ble' || o.link === 'WifiP2p' || o.link === 'Lora';
    rows.push({
      name: o.link === 'WifiP2p' ? 'Wi-Fi Direct' : o.link === 'Lora' ? 'LoRa radio' : 'Bluetooth',
      ok: near,
      desc: near ? 'in range · linked straight to their phone' : 'not in range (10–30 m)',
    });
  }
  if (!o.isChannel) {
    const routeOk = o.netOn && o.netOnline && !o.highRisk && o.addon;
    rows.push({
      name: 'Your internet route',
      ok: routeOk,
      desc: o.highRisk
        ? 'off in High-Risk'
        : !o.addon
          ? 'add-on needed'
          : !o.netOn
            ? 'off · turn it on in Settings'
            : o.netOnline
              ? 'connected directly'
              : 'connecting…',
    });
    rows.push({
      name: o.isDm ? `${o.first}’s internet address` : 'Members’ internet addresses',
      ok: o.theirAddress,
      desc: o.theirAddress ? 'shared' : 'not shared yet. They need Internet route on, and to be near you once.',
    });
  }
  rows.push({
    name: 'Nearby phones',
    ok: o.neighbours > 0,
    desc: o.neighbours > 0 ? `${o.neighbours} can carry it` : 'none in range right now',
  });
  return rows;
}

/** Same limit as the core (MAX_FILE_BYTES). */
const MAX_FILE_BYTES = 5 * 1024 * 1024;

function bytesLabel(n: number | null) {
  if (n == null) return '';
  return n > 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.ceil(n / 1024)} KB`;
}

function countdown(ms: number) {
  const s = Math.max(0, Math.floor(ms / 1000));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${m}m`;
  return `${Math.max(1, m)}m`;
}

export function Chat({ conv }: { conv: string }) {
  const { convs, contacts, call, pop, push, settings, network, client, notify, refresh } = useApp();
  const summary = convs.find((c) => c.id === conv);
  const [messages, setMessages] = useState<Message[]>([]);
  const [members, setMembers] = useState<GroupMember[]>([]);
  const [text, setText] = useState('');
  const [when, setWhen] = useState<SendWhen>('Now');
  const [sheet, setSheet] = useState<Sheets>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Message | null>(null);
  const [busy, setBusy] = useState(false);
  // A picked file being read and sealed into parts (can take a while).
  const [preparing, setPreparing] = useState<{ name: string; size: number } | null>(null);
  const [viewer, setViewer] = useState<{ m: Message; img: RgbaImage } | null>(null);
  // Swipe to reply: the message being answered, and a briefly highlighted
  // original after tapping a quote.
  const [replyTo, setReplyTo] = useState<Message | null>(null);
  const [highlight, setHighlight] = useState<string | null>(null);
  const listRef = useRef<FlatList<Item>>(null);
  const inputRef = useRef<TextInput>(null);
  // ===== Voice notes =====
  const voice = client.hooks.voice;
  const [rec, setRec] = useState<Rec | null>(null);
  const recRef = useRef<Rec | null>(null);
  recRef.current = rec;
  // Fast-changing voice state (levels, playback position) lives outside
  // React state: only the voice widgets redraw, not the whole chat.
  const [live] = useState(createLive);
  const [speed, setSpeed] = useState(1);
  const itemsRef = useRef<Item[]>([]);
  // While the mic is held: how far the finger slid (cancel left, lock up).
  const drag = useRef(new Animated.ValueXY()).current;
  // What the microphone explainer is for, and the video awaiting "Open".
  const [micFor, setMicFor] = useState<'voice' | 'call'>('voice');
  const [videoMsg, setVideoMsg] = useState<Message | null>(null);
  // Internet add-on present (Android); read when the routes sheet opens.
  const [addonOk, setAddonOk] = useState(true);
  useEffect(() => {
    if (!voice) return;
    const offLevel = voice.onLevel((e) => {
      const r = recRef.current;
      if (!r || r.phase === 'review' || r.phase === 'paused') return;
      live.setRec({ elapsed: e.elapsedMs, levels: [...live.rec().levels.slice(-35), e.level] });
    });
    const offLimit = voice.onLimit(async () => {
      const st = await voice.stop();
      notify('Voice notes stop at 5 minutes');
      setRec((r) => (r ? { ...r, phase: 'review', durationMs: st.durationMs, waveform: st.waveform } : r));
    });
    const offPlay = voice.onPlayback((e) => {
      if (!e.ended) {
        live.setPlayback({ id: e.message, pos: e.positionMs, dur: e.durationMs, playing: e.playing });
        return;
      }
      live.setPlayback(IDLE);
      // Auto-play the next new voice note from the other side.
      const list = itemsRef.current;
      const at = list.findIndex((it) => it.k === 'msg' && it.m.id === e.message);
      for (let i = at - 1; at > 0 && i >= 0; i--) {
        const it = list[i];
        if (it.k !== 'msg') continue;
        const m = it.m;
        if (m.kind === 'Voice' && !m.outgoing && m.voice && !m.voice.listened && m.chunks_have >= m.chunks_total) {
          playVoice(m);
          break;
        }
      }
    });
    return () => {
      offLevel();
      offLimit();
      offPlay();
      voice.stopPlayback().catch(() => {});
      if (recRef.current) voice.cancel().catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [voice]);
  const playVoice = async (m: Message, fromMs = 0) => {
    if (!voice) return;
    try {
      await voice.play(m.id, fromMs, speed);
      if (!m.outgoing && m.voice && !m.voice.listened) await call('voice_listened', { message: m.id }).catch(() => {});
    } catch (e) {
      notify(errorText(e));
    }
  };
  const voiceUi: VoiceUi = {
    live,
    speed,
    toggle: (m) => {
      if (!voice) return;
      const pb = live.playback();
      if (pb.id === m.id) {
        // Optimistic: the button flips at once, the player confirms.
        live.setPlayback({ ...pb, playing: !pb.playing });
        (pb.playing ? voice.pause() : voice.resume()).catch(() => {});
      } else playVoice(m);
    },
    seek: (m, frac) => {
      if (!voice) return;
      const pb = live.playback();
      const total = pb.id === m.id && pb.dur > 0 ? pb.dur : (m.voice?.duration_ms ?? 0);
      if (pb.id === m.id) {
        live.setPlayback({ ...pb, pos: frac * total });
        voice.seek(frac * total).catch(() => {});
      } else playVoice(m, frac * total);
    },
    cycleSpeed: () => {
      const next = speed === 1 ? 1.5 : speed === 1.5 ? 2 : 1;
      setSpeed(next);
      if (voice && live.playback().id) voice.setSpeed(next).catch(() => {});
    },
  };
  // The microphone: explained before Android asks, and a way back to
  // settings once it was refused for good.
  const needMic = (purpose: 'voice' | 'call') => {
    setMicFor(purpose);
    setSheet(voice?.micBlocked() ? 'micoff' : 'mic');
  };
  const micStart = async () => {
    if (!voice || recRef.current) return false;
    if (voiceWhy) {
      notify(voiceWhy);
      return false;
    }
    if (!voice.micGranted()) {
      needMic('voice');
      return false;
    }
    try {
      await voice.stopPlayback().catch(() => {});
      await voice.start();
      drag.setValue({ x: 0, y: 0 });
      Vibration.vibrate(12);
      // Set the ref now too: a release that came while the microphone was
      // starting is handled right after this returns, before any render.
      const r: Rec = { phase: 'hold', durationMs: 0, waveform: [] };
      live.setRec({ elapsed: 0, levels: [] });
      recRef.current = r;
      setRec(r);
      return true;
    } catch (e) {
      notify(errorText(e));
      return false;
    }
  };
  const micCancel = async (why?: string) => {
    recRef.current = null;
    setRec(null);
    await voice?.cancel().catch(() => {});
    if (why) notify(why);
  };
  const micSend = async () => {
    if (!voice || !recRef.current) return;
    setBusy(true);
    try {
      if (recRef.current.phase !== 'review') {
        const st = await voice.stop();
        if (st.durationMs < 800 || !st.ok) {
          await micCancel('Hold to record');
          return;
        }
      }
      await voice.stopPlayback().catch(() => {});
      await voice.send(conv, { when }, replyTo?.id ?? null);
      setRec(null);
      setReplyTo(null);
      setWhen('Now');
      await load();
    } catch (e) {
      setRec(null);
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const micReview = async () => {
    if (!voice || !recRef.current) return;
    const st = await voice.stop();
    if (!st.ok) return micCancel('Recording failed');
    setRec({ ...recRef.current, phase: 'review', durationMs: st.durationMs, waveform: st.waveform });
  };
  const micPause = async () => {
    const r = recRef.current;
    if (!voice || !r || (r.phase !== 'locked' && r.phase !== 'paused')) return;
    try {
      const pausing = r.phase === 'locked';
      const elapsed = await voice.pauseRecording(pausing);
      live.setRec({ ...live.rec(), elapsed });
      setRec((x) => (x ? { ...x, phase: pausing ? 'paused' : 'locked' } : x));
    } catch (e) {
      notify(errorText(e));
    }
  };
  const startReply = (m: Message) => {
    if (m.kind === 'System' || m.kind === 'Call') return;
    setReplyTo(m);
    setTimeout(() => inputRef.current?.focus(), 50);
  };
  const [events, setEvents] = useState<CarrierEvent[]>([]);

  const load = useCallback(async () => {
    try {
      setMessages(await call<Message[]>('messages', { conversation: conv, limit: 300 }));
      await call('mark_read', { conversation: conv });
    } catch (e) {
      setError(errorText(e));
    }
  }, [call, conv]);

  useEffect(() => {
    load();
  }, [load]);

  const loadMembers = useCallback(() => {
    if (summary?.kind !== 'Group') return;
    call<GroupMember[]>('group_members', { conversation: conv })
      .then(setMembers)
      .catch((e) => setError(errorText(e)));
  }, [call, conv, summary?.kind]);

  useEffect(() => {
    loadMembers();
  }, [loadMembers]);

  useCoreEvents(
    (e) => {
      if (e.event === 'MessageAdded' && e.conversation === conv) load();
      if (e.event === 'MessageState' || e.event === 'MessageDeleted') load();
      // Membership and contact names change the header count and "Not in your contacts" tags.
      if (e.event === 'ConversationsChanged' || e.event === 'ContactChanged') loadMembers();
    },
    [conv, load, loadMembers],
  );

  // Disappearing messages vanish on their own.
  useEffect(() => {
    const t = setInterval(() => {
      if (messages.some((m) => m.expires_at && m.expires_at <= Date.now())) load();
    }, 5000);
    return () => clearInterval(t);
  }, [messages, load]);

  const contact = contacts.find((x) => x.conversation === conv);
  // Header carrier from core facts: the live link to this contact (Contact.link);
  // with no direct link, other phones in range carry messages (mesh); else wait.
  // Out of range, a contact who shared an internet address is reached over
  // the internet route (Contact.internet). Groups have no single link: mesh,
  // or the internet if a member who is our contact is reachable over it.
  // Channels never use the internet route (the core never sends them there).
  const neighbours = network?.neighbours ?? 0;
  const direct = summary?.kind === 'Direct';
  const groupOnline =
    summary?.kind === 'Group' && members.some((m) => !m.is_me && m.contact && contacts.some((c) => c.id === m.contact && c.internet));
  const carrier: Carrier =
    direct && contact?.link
      ? LINK_CARRIER[contact.link]
      : direct && contact?.internet
        ? 'net'
        : neighbours > 0
          ? 'mesh'
          : groupOnline
            ? 'net'
            : 'none';
  // Why an outgoing direct message still waits, when the internet route
  // could have carried it: say which side is missing.
  const netOn = !!network?.internet_on && !settings?.high_risk;
  const waitWhy = !netOn || !direct
    ? 'Waiting on this phone · leaves when a phone comes in range'
    : !network?.internet_online
      ? 'Waiting · your internet route is not connected yet (Settings, Internet route)'
      : contact && !contact.internet
        ? `Waiting · ${contact.nickname} has not shared an internet address yet. They need Internet route on, and to be in range once.`
        : 'Waiting on this phone · leaves over the internet route or when a phone comes in range';

  // Carrier switches while the screen is open: a timeline chip and a sweep under the header.
  const prevCarrier = useRef<Carrier | null>(null);
  const reduceMotion = useReduceMotion();
  const sweep = useRef(new Animated.Value(0)).current;
  const [headerW, setHeaderW] = useState(360);
  useEffect(() => {
    if (prevCarrier.current && prevCarrier.current !== carrier) {
      const from = prevCarrier.current;
      setEvents((ev) => [...ev, { id: `c${Date.now()}`, at: Date.now(), carrier, text: carrierEventText(carrier, direct, from) }]);
      sweep.setValue(0);
      if (!reduceMotion) Animated.timing(sweep, { toValue: 1, duration: 700, easing: Easing.out(Easing.quad), useNativeDriver: native }).start();
    }
    prevCarrier.current = carrier;
  }, [carrier, direct, sweep, reduceMotion]);

  const byId = useMemo(() => new Map(messages.map((m) => [m.id, m])), [messages]);
  const items = useMemo<Item[]>(() => {
    const all: (Item & { at: number })[] = [
      ...messages.map((m) => ({ k: 'msg' as const, m, at: m.sent_at })),
      ...events.map((e) => ({ k: 'carr' as const, e, at: e.at })),
    ];
    all.sort((a, b) => a.at - b.at);
    // A divider wherever the day changes.
    const out: Item[] = [];
    let day = '';
    for (const it of all) {
      const d = new Date(it.at).toDateString();
      if (d !== day) {
        day = d;
        out.push({ k: 'day', id: `day-${d}`, label: dayLabel(it.at) });
      }
      out.push(it);
    }
    // A picked file being sealed shows where it will appear.
    if (preparing) out.push({ k: 'prep', name: preparing.name, size: preparing.size });
    return out.reverse();
  }, [messages, events, preparing]);

  const sendScale = useRef(new Animated.Value(0.9)).current;
  const ready = text.trim().length > 0;
  useEffect(() => {
    Animated.spring(sendScale, { toValue: ready ? 1 : 0.9, friction: 5, tension: 180, useNativeDriver: native }).start();
  }, [ready, sendScale]);

  if (!summary) {
    return (
      <View style={{ flex: 1, backgroundColor: color.bg }}>
        <View style={s.headTop}>
          <Pressable onPress={pop} accessibilityRole="button" accessibilityLabel="Back" style={s.back}>
            <Icon name="left" size={24} />
          </Pressable>
        </View>
        <Text style={[type.body, { padding: 20 }]}>This conversation no longer exists.</Text>
      </View>
    );
  }

  const isDm = summary.kind === 'Direct';
  const isGroup = summary.kind === 'Group';
  const isChannel = summary.kind === 'Channel';
  const blocked = summary.trust === 'KeyChanged';
  const highRisk = !!settings?.high_risk;
  const attachBlocked = isChannel || highRisk;
  // Voice notes: Android only; not in channels or High-Risk mode.
  const voiceWhy = isChannel
    ? 'Voice notes are off in public channels so nothing identifying leaks to strangers.'
    : highRisk
      ? 'High-Risk mode allows text only'
      : null;
  const recPanel = !!rec && rec.phase !== 'hold';
  // Calls: a paired contact in radio range, not in High-Risk mode.
  const callOk = isDm && !!contact?.link && !highRisk && !blocked;
  const startCall = async () => {
    if (!contact || !client.hooks.call) return;
    if (blocked) return notify('Sending paused: key changed · re-pair in person to call');
    if (highRisk) return setSheet('hrcall');
    if (!contact.link) return notify('Calls work only when you’re near each other');
    if (!client.hooks.call.micGranted()) return needMic('call');
    call('start_call', { contact: contact.id }).catch((e) => notify(errorText(e)));
  };
  const bytes = new TextEncoder().encode(text).length;
  const sub = isGroup
    ? `${members.length || '…'} members${summary.is_admin ? ' · you are admin' : ''}`
    : isChannel
      ? `Public channel${summary.my_handle ? ` · you are ${summary.my_handle}` : ''}`
      : '';

  const send = async (w: SendWhen = when) => {
    if (!text.trim() || bytes > 3500) return;
    setError(null);
    setBusy(true);
    try {
      await call('send_text', { conversation: conv, text, options: { when: w }, ...(replyTo ? { reply_to: replyTo.id } : {}) });
      setText('');
      setReplyTo(null);
      setWhen('Now');
      setSheet(null);
      if (w === 'Hold') notify('Held until you release it');
      else if (w !== 'Now') notify(`Will send ${LATER.find((l) => l.value === w)?.label.toLowerCase()}`);
      await load();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const attachImage = async () => {
    setSheet(null);
    if (attachBlocked) return;
    if (!client.hooks.pickImage) return notify('Choosing photos is not available on this device');
    setBusy(true);
    try {
      const img = await client.hooks.pickImage();
      if (!img) return;
      if (img.kind === 'rgba') {
        await call('send_image', { conversation: conv, rgba: b64encode(img.rgba), width: img.width, height: img.height, options: { when } });
      } else {
        await call('send_jpeg', { conversation: conv, jpeg: img.base64, options: { when } });
      }
      await load();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const attachFile = async () => {
    setSheet(null);
    if (attachBlocked) return;
    if (!client.hooks.pickFile && !client.hooks.pickFileRef) return notify('Choosing files is not available on this device');
    setBusy(true);
    try {
      if (client.hooks.pickFileRef && client.hooks.sendFileRef) {
        const f = await client.hooks.pickFileRef();
        if (!f) return;
        if (f.size > MAX_FILE_BYTES) return notify(`Files can be up to 5 MB · this one is ${bytesLabel(f.size)}`);
        setPreparing({ name: f.name, size: f.size });
        await client.hooks.sendFileRef(conv, f, { when });
      } else {
        const f = await client.hooks.pickFile!();
        if (!f) return;
        setPreparing({ name: f.name, size: f.bytes.length });
        await call('send_file', { conversation: conv, name: f.name, bytes: b64encode(f.bytes), options: { when } });
      }
      await load();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPreparing(null);
      setBusy(false);
    }
  };

  const openDetail = () => {
    if (isGroup) push({ name: 'GroupDetail', conv });
    else if (isDm && contact) push({ name: 'ContactDetail', contact: contact.id });
  };

  const del = async (everyone: boolean) => {
    const m = selected;
    setSheet(null);
    if (!m) return;
    try {
      await call('delete_message', { message: m.id, for_everyone: everyone });
      notify(everyone ? 'Deleted for everyone' : 'Deleted for you');
      load();
    } catch (e) {
      notify(errorText(e));
    }
  };

  const saveFile = async (m: Message) => {
    // Files are never opened or previewed in the app (12.3).
    try {
      const r = await call<{ name: string; bytes: string }>('file_bytes', { message: m.id });
      if (!client.hooks.saveFile) throw new Error('Saving files is not available on this device');
      await client.hooks.saveFile(r.name, b64decode(r.bytes));
      notify('Saved to Files · open with care');
    } catch (e) {
      notify(errorText(e));
    }
  };

  const unknownSender = (name: string) => isGroup && members.some((x) => !x.is_me && x.nickname === name && !x.contact);

  itemsRef.current = items;
  return (
    <Appear kind="right" style={{ flex: 1, backgroundColor: color.bg }}>
      {/* Header */}
      <View style={s.header} onLayout={(e) => setHeaderW(e.nativeEvent.layout.width)}>
        <View style={s.headTop}>
          <Pressable onPress={pop} accessibilityRole="button" accessibilityLabel="Back" style={s.back}>
            <Icon name="left" size={24} />
          </Pressable>
          <Pressable
            onPress={openDetail}
            disabled={isChannel}
            accessibilityRole={isChannel ? 'header' : 'button'}
            accessibilityLabel={isChannel ? `#${summary.title}` : summary.title}
            style={{ flex: 1, minWidth: 0, flexDirection: 'row', alignItems: 'center', gap: 10 }}
          >
            <Avatar seed={contact?.identity_key ?? conv} size={40} kind={isGroup ? 'group' : isChannel ? 'channel' : 'person'} />
            <View style={{ flex: 1, minWidth: 0, gap: 4 }}>
              <Text style={{ fontFamily: font.semi, fontSize: 16.5, color: color.ink }} numberOfLines={1}>
                {isChannel ? `#${summary.title}` : summary.title}
              </Text>
              {isDm && summary.trust ? <TrustBadge trust={summary.trust} compact /> : <Text style={{ fontFamily: font.medium, fontSize: 12, color: color.textMuted }}>{sub}</Text>}
            </View>
          </Pressable>
          {isDm && contact && client.hooks.call ? (
            <Pressable
              onPress={startCall}
              accessibilityRole="button"
              accessibilityLabel={callOk ? 'Call' : 'Call unavailable'}
              accessibilityState={{ disabled: !callOk }}
              style={({ pressed }) => [s.details, { backgroundColor: callOk ? color.lime : color.chip }, pressed ? { transform: [{ scale: 0.94 }] } : null]}
            >
              <Icon name="call" size={18} tint={callOk ? color.ink : '#9A968C'} />
            </Pressable>
          ) : null}
          {!isChannel ? (
            <Pressable onPress={openDetail} accessibilityRole="button" accessibilityLabel="Conversation details" style={s.details}>
              <Icon name="info" size={19} />
            </Pressable>
          ) : null}
        </View>
        <View style={s.headRow}>
          <Pressable
            onPress={() => {
              setAddonOk(readAddon());
              setSheet('routes');
            }}
            accessibilityRole="button"
            accessibilityLabel="How messages travel"
            style={{ flexDirection: 'row', alignItems: 'center', gap: 2, minWidth: 0 }}
          >
            <CarrierPill carrier={carrier} highRisk={highRisk} />
            <Icon name="right" size={14} tint="#9A968C" />
          </Pressable>
          <Pressable onPress={() => setSheet('timer')} accessibilityRole="button" accessibilityLabel={`Disappearing messages, ${disappearLabel(summary.disappear_secs)}`} style={s.timerBtn}>
            <Icon name="timer" size={14} tint={color.textMuted} />
            <Text style={{ fontFamily: font.medium, fontSize: 12, color: color.textMuted }}>Disappears · {disappearShort(summary.disappear_secs)}</Text>
          </Pressable>
        </View>
        <View style={s.sweepTrack} pointerEvents="none">
          <Animated.View
            style={{
              width: headerW * 0.45,
              height: 2,
              backgroundColor: CARRIER[carrier].fg,
              opacity: sweep.interpolate({ inputRange: [0, 0.1, 0.85, 1], outputRange: [0, 1, 1, 0] }),
              transform: [{ translateX: sweep.interpolate({ inputRange: [0, 1], outputRange: [-headerW * 0.45, headerW * 1.2] }) }],
            }}
          />
        </View>
      </View>

      {isChannel ? (
        <View style={s.channelBar}>
          <Icon name="eye" size={16} tint={color.lime} />
          <Text style={{ flex: 1, fontFamily: font.medium, fontSize: 12.5, lineHeight: 17, color: '#fff' }}>Anyone who knows this name can read it. You post as a random name here.</Text>
        </View>
      ) : null}

      {/* Timeline, newest at the bottom */}
      <FlatList
        ref={listRef}
        inverted
        data={items}
        onScrollToIndexFailed={(info) => {
          listRef.current?.scrollToOffset({ offset: info.averageItemLength * info.index, animated: true });
        }}
        style={{ flex: 1 }}
        keyExtractor={(it) => (it.k === 'msg' ? it.m.id : it.k === 'carr' ? it.e.id : it.k === 'day' ? it.id : 'preparing')}
        contentContainerStyle={{ paddingHorizontal: 14, paddingTop: 12, paddingBottom: 16 }}
        keyboardShouldPersistTaps="handled"
        renderItem={({ item }) =>
          item.k === 'carr' ? (
            <CarrierChip carrier={item.e.carrier} text={item.e.text} />
          ) : item.k === 'day' ? (
            <View style={s.sysPill} accessibilityRole="header">
              <Text style={{ fontFamily: font.medium, fontSize: 12, lineHeight: 17, color: color.textMuted, textAlign: 'center' }}>{item.label}</Text>
            </View>
          ) : item.k === 'prep' ? (
            <Appear kind="bubble" style={s.prepCard}>
              <View accessible accessibilityLiveRegion="polite" accessibilityLabel={`Preparing ${item.name}`}>
                <Spinner size={22} />
              </View>
              <View style={{ flex: 1, minWidth: 0, gap: 3 }}>
                <Text style={{ fontFamily: font.semi, fontSize: 14, color: color.ink }} numberOfLines={1}>
                  Preparing {item.name}
                </Text>
                <Text style={{ fontFamily: font.regular, fontSize: 12.5, lineHeight: 17, color: color.textMuted }}>
                  {bytesLabel(item.size)} · sealing it into parts. It appears in the chat when ready.
                </Text>
              </View>
            </Appear>
          ) : (
            <SwipeToReply enabled={!blocked && item.m.kind !== 'System' && item.m.kind !== 'Call'} onReply={() => startReply(item.m)}>
              <Bubble
                m={item.m}
                orig={item.m.reply ? byId.get(item.m.reply.to) : undefined}
                onOpenVideo={() => {
                  setVideoMsg(item.m);
                  setSheet('video');
                }}
                kind={summary.kind}
                peerName={summary.title}
                waitWhy={waitWhy}
                highlighted={highlight === item.m.id}
                voiceUi={voiceUi}
                onOpenPhoto={(img) => setViewer({ m: item.m, img })}
                onJumpTo={(id) => {
                  const index = items.findIndex((it) => it.k === 'msg' && it.m.id === id);
                  if (index < 0) return notify('That message is no longer in this chat');
                  listRef.current?.scrollToIndex({ index, viewPosition: 0.5, animated: true });
                  setHighlight(id);
                  setTimeout(() => setHighlight((h) => (h === id ? null : h)), 1600);
                }}
                unknownSender={unknownSender(item.m.sender)}
                onLongPress={() => {
                  setSelected(item.m);
                  setSheet('actions');
                }}
              />
            </SwipeToReply>
          )
        }
      />

      {/* Composer, or the key-changed block */}
      {blocked ? (
        <View style={{ paddingHorizontal: 12, paddingTop: 12, paddingBottom: 26, backgroundColor: color.bg }}>
          <View accessibilityRole="alert" style={s.blockCard}>
            <View style={{ flexDirection: 'row', gap: 12, alignItems: 'flex-start' }}>
              <IconTile icon="shieldX" bg={color.dangerBg} fg={color.danger} size={40} />
              <View style={{ flex: 1, gap: 3 }}>
                <Text style={{ fontFamily: font.semi, fontSize: 15, color: color.ink }}>Sending paused: key changed</Text>
                <Text style={{ fontFamily: font.regular, fontSize: 13, lineHeight: 18, color: color.textMuted }}>
                  A different key appeared under this name. It could be a new phone, or someone pretending. Meet and pair again to continue.
                </Text>
              </View>
            </View>
            <Pressable onPress={() => push({ name: 'Pair' })} accessibilityRole="button" accessibilityLabel="Re-pair in person" style={s.repair}>
              <Icon name="qr" size={17} tint={color.lime} />
              <Text style={{ fontFamily: font.semi, fontSize: 14.5, color: '#fff' }}>Re-pair in person</Text>
            </Pressable>
          </View>
        </View>
      ) : (
        <View style={{ backgroundColor: color.bg }}>
          {error ? (
            <View style={{ paddingHorizontal: 12, paddingTop: 6 }}>
              <ErrorText error={error} />
            </View>
          ) : null}
          {when !== 'Now' ? (
            <View style={s.whenRow}>
              <Icon name={when === 'Hold' ? 'pause' : 'clock'} size={14} tint={color.textMuted} />
              <Text style={{ flex: 1, fontFamily: font.medium, fontSize: 12, color: color.textMuted }}>
                Next message: {LATER.find((l) => l.value === when)?.label.toLowerCase()}
              </Text>
              <Pressable onPress={() => setWhen('Now')} accessibilityRole="button" accessibilityLabel="Send now instead" hitSlop={8}>
                <Icon name="x" size={14} tint={color.textMuted} />
              </Pressable>
            </View>
          ) : null}
          {replyTo ? (
            <Appear kind="up" style={s.replyBar}>
              <View style={s.replyTile}>
                <Icon name="reply" size={16} tint={color.ink} />
              </View>
              <View style={{ flex: 1, minWidth: 0, gap: 1 }} accessibilityLiveRegion="polite">
                <Text style={{ fontFamily: font.semi, fontSize: 13, color: color.ink }} numberOfLines={1}>
                  {replyTo.outgoing ? 'Replying to yourself' : `Replying to ${isDm ? summary.title.split(' ')[0] : replyTo.sender}`}
                </Text>
                <View style={{ flexDirection: 'row', alignItems: 'center', gap: 5, minWidth: 0 }}>
                  {quoteIcon(replyTo) ? <Icon name={quoteIcon(replyTo)!} size={13} tint={color.textMuted} /> : null}
                  <Text style={{ flexShrink: 1, fontFamily: font.regular, fontSize: 13, color: color.textMuted }} numberOfLines={1}>
                    {quotePreview(replyTo)}
                  </Text>
                </View>
              </View>
              <Pressable onPress={() => setReplyTo(null)} accessibilityRole="button" accessibilityLabel="Cancel reply" style={s.replyCancel}>
                <Icon name="x" size={16} tint={color.ink} />
              </Pressable>
            </Appear>
          ) : null}
          {rec && recPanel ? (
            <RecPanel
              rec={rec}
              live={live}
              voiceUi={voiceUi}
              busy={busy}
              onDelete={() => micCancel('Recording deleted')}
              onPause={micPause}
              onStop={micReview}
              onSend={micSend}
              onPlayDraft={() => voiceUi.toggle({ id: 'draft' } as Message)}
              onSeekDraft={(f) => voiceUi.seek({ id: 'draft', outgoing: true, voice: { duration_ms: rec.durationMs, listened: true } } as Message, f)}
            />
          ) : null}
          {recPanel ? <View style={{ height: 24 }} /> : (
          <View style={s.composer}>
            {rec ? (
              <HoldRow live={live} drag={drag} />
            ) : (
            <>
            <Pressable onPress={() => setSheet('attach')} accessibilityRole="button" accessibilityLabel="Attach" style={s.attach}>
              <Icon name="plus" size={20} tint={attachBlocked ? color.disabled : color.ink} />
            </Pressable>
            <View style={s.inputPill}>
              <TextInput
                ref={inputRef}
                value={text}
                onChangeText={setText}
                placeholder={replyTo ? 'Reply…' : isChannel ? 'Post to everyone nearby' : 'Message'}
                placeholderTextColor={color.textFaint}
                multiline
                autoCorrect={!highRisk}
                spellCheck={!highRisk}
                importantForAutofill="no"
                accessibilityLabel="Message text"
                style={s.input}
              />
              <Pressable onPress={() => setSheet('timer')} accessibilityRole="button" accessibilityLabel="Disappearing timer" style={s.timerChip}>
                <Icon name="timer" size={14} />
                <Text style={{ fontFamily: font.semi, fontSize: 11.5, color: color.ink }}>{disappearShort(summary.disappear_secs)}</Text>
              </Pressable>
              <Pressable onPress={() => setSheet('later')} accessibilityRole="button" accessibilityLabel="Send later" style={s.laterBtn}>
                <Icon name="clock" size={18} tint={when !== 'Now' ? color.limeInk : color.ink} />
              </Pressable>
            </View>
            </>
            )}
            {voice && !ready ? (
              <MicButton
                recording={!!rec}
                blocked={!!voiceWhy}
                drag={drag}
                onStart={micStart}
                onRelease={micSend}
                onCancel={() => micCancel('Recording deleted')}
                onLock={() => {
                  Vibration.vibrate(12);
                  setRec((r) => (r ? { ...r, phase: 'locked' } : r));
                }}
              />
            ) : (
            <Pressable
              onPress={() => send()}
              disabled={!ready || bytes > 3500 || busy}
              accessibilityRole="button"
              accessibilityLabel={when === 'Now' ? 'Send' : 'Schedule'}
              accessibilityState={{ disabled: !ready || bytes > 3500 || busy }}
            >
              <Animated.View style={[s.send, { backgroundColor: ready ? color.ink : '#E3E0D8', transform: [{ scale: sendScale }] }]}>
                {busy ? <Spinner size={20} /> : <Icon name={when === 'Now' ? 'send' : 'clock'} size={21} tint={ready ? color.lime : '#9A968C'} />}
              </Animated.View>
            </Pressable>
            )}
          </View>
          )}
          {bytes > 3000 ? <Text style={[type.tiny, { textAlign: 'right', paddingRight: 20, marginTop: -16, paddingBottom: 8 }]}>{bytes}/3500 bytes</Text> : null}
        </View>
      )}

      {/* Sheets */}
      <Sheet open={sheet === 'attach'} onClose={() => setSheet(null)} title="Attach">
        {attachBlocked ? (
          <View style={s.blockedWhy}>
            <Icon name="lock" size={17} tint={color.inkSoft} />
            <Text style={{ flex: 1, fontFamily: font.medium, fontSize: 13, lineHeight: 18, color: color.inkSoft }}>
              {isChannel ? 'Photos and files are off in public channels so nothing identifying leaks to strangers.' : 'High-Risk mode allows text only.'}
            </Text>
          </View>
        ) : null}
        <View style={{ flexDirection: 'row', gap: 10 }}>
          <AttachTile icon="image" title="Photo" sub="Resized, metadata stripped, max 256 KB" disabled={attachBlocked} onPress={attachImage} />
          <AttachTile icon="file" title="File" sub="Up to 5 MB, saved not opened" disabled={attachBlocked} onPress={attachFile} />
        </View>
      </Sheet>

      <Sheet open={sheet === 'timer'} onClose={() => setSheet(null)} title="Disappearing messages" subtitle="The timer starts when a message is first shown.">
        <OptionGrid
          options={DISAPPEAR_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
          value={summary.disappear_secs}
          onChange={async (v) => {
            setSheet(null);
            try {
              await call('set_disappearing', { conversation: conv, secs: v });
              await refresh();
              notify(`Messages now disappear after ${disappearLabel(v)}`);
            } catch (e) {
              notify(errorText(e));
            }
          }}
        />
      </Sheet>

      <Sheet
        open={sheet === 'later'}
        onClose={() => setSheet(null)}
        title="Send later"
        subtitle={ready ? `“${text.trim().slice(0, 40)}”` : 'Pick when your next message leaves. It waits on this phone until then.'}
      >
        <View accessibilityRole="radiogroup" style={{ gap: 8 }}>
          {LATER.map((o) => (
            <Pressable
              key={o.value}
              accessibilityRole="radio"
              accessibilityState={{ checked: when === o.value }}
              aria-checked={when === o.value}
              accessibilityLabel={o.label}
              onPress={() => {
                // With a draft it leaves right away on that schedule; without one, the
                // choice applies to the next message (the send button turns into Schedule).
                if (ready) send(o.value);
                else {
                  setWhen(o.value);
                  setSheet(null);
                }
              }}
              style={({ pressed }) => [s.laterOpt, when === o.value ? { backgroundColor: color.ink } : null, pressed ? { opacity: 0.7 } : null]}
            >
              <Icon name={o.value === 'Hold' ? 'pause' : 'clock'} size={18} tint={when === o.value ? color.lime : color.ink} />
              <Text style={{ fontFamily: font.semi, fontSize: 15, color: when === o.value ? '#fff' : color.ink }}>{o.label}</Text>
            </Pressable>
          ))}
        </View>
      </Sheet>

      <Sheet open={sheet === 'actions'} onClose={() => setSheet(null)}>
        {selected ? (
          <>
            <View style={s.selPreview}>
              <Text style={{ fontFamily: font.regular, fontSize: 14, color: color.inkSoft }} numberOfLines={1}>
                {quotePreview(selected)}
              </Text>
            </View>
            {!blocked && selected.kind !== 'System' ? (
              <SheetAction
                icon="reply"
                label="Reply"
                onPress={() => {
                  setSheet(null);
                  startReply(selected);
                }}
              />
            ) : null}
            {selected.kind === 'Text' ? (
              <SheetAction
                icon="copy"
                label="Copy"
                hint="clears after 60 s"
                onPress={() => {
                  setSheet(null);
                  if (!client.hooks.copyText) return notify('Copying is not available on this device');
                  client.hooks.copyText(selected.text);
                  notify('Copied · clipboard clears in 60 s');
                }}
              />
            ) : null}
            {selected.kind === 'File' && !selected.outgoing && selected.chunks_have >= selected.chunks_total ? (
              <SheetAction
                icon="download"
                label="Save to Files"
                onPress={() => {
                  setSheet(null);
                  saveFile(selected);
                }}
              />
            ) : null}
            {selected.outgoing ? <SheetAction icon="info" label="Delivery info" onPress={() => setSheet('info')} /> : null}
            <SheetAction icon="trash" label="Delete for me" onPress={() => del(false)} />
            {selected.outgoing ? <SheetAction icon="trash" label="Delete for everyone" danger onPress={() => del(true)} /> : null}
          </>
        ) : null}
      </Sheet>

      <Sheet open={sheet === 'info'} onClose={() => setSheet(null)} title="Delivery timeline">
        {selected ? <Timeline m={selected} /> : null}
      </Sheet>
      <Sheet open={sheet === 'routes'} onClose={() => setSheet(null)}>
        <Text style={{ fontFamily: font.display, fontSize: 22, lineHeight: 25, color: color.ink, paddingHorizontal: 4 }}>
          {isDm ? `How messages reach ${summary.title.split(' ')[0]}` : isChannel ? `How posts reach #${summary.title}` : `How messages reach ${summary.title}`}
        </Text>
        <Text style={{ fontFamily: font.regular, fontSize: 14, lineHeight: 21, color: color.textMuted, paddingHorizontal: 4 }}>
          The pill shows the best route right now. When several work, the higher row wins.
        </Text>
        <View style={s.routesCard}>
          {routeRows({
            isDm,
            isChannel,
            first: isDm ? summary.title.split(' ')[0] : 'Members',
            link: isDm ? (contact?.link ?? null) : null,
            highRisk,
            addon: addonOk,
            netOn: !!network?.internet_on,
            netOnline: !!network?.internet_online,
            theirAddress: isDm ? !!contact?.internet : groupOnline,
            neighbours,
          }).map((r, i, all) => (
            <View key={r.name} style={[s.routeRow, i === all.length - 1 ? { borderBottomWidth: 0 } : null]} accessible accessibilityLabel={`${r.name}: ${r.desc}`}>
              <View style={[s.routeDot, { backgroundColor: r.ok ? '#DCF2E6' : '#EDEBE6' }]}>
                <Icon name={r.ok ? 'check' : 'x'} size={15} tint={r.ok ? '#0B6E4A' : '#4E525B'} />
              </View>
              <Text style={{ flex: 1, fontFamily: font.regular, fontSize: 13.5, lineHeight: 19.5, color: color.inkSoft, paddingTop: 4 }}>
                <Text style={{ fontFamily: font.semi, color: color.ink }}>{r.name}</Text> · {r.desc}
              </Text>
            </View>
          ))}
        </View>
      </Sheet>
      <Sheet open={sheet === 'mic'} onClose={() => setSheet(null)}>
        <Notice
          icon="mic"
          tileBg={color.lime}
          title={micFor === 'call' ? 'Talk in nearby calls' : 'Record voice notes'}
          body={
            micFor === 'call'
              ? 'Meshtrike needs the microphone only while a call is on. Call audio is sealed end to end and never stored.'
              : 'Meshtrike needs the microphone only while you hold the mic button. Recordings are sealed like messages.'
          }
        />
        <Button
          title="Continue"
          onPress={async () => {
            setSheet(null);
            const ok = await (voice?.askMic() ?? client.hooks.call?.askMic() ?? Promise.resolve(false));
            if (!ok) {
              if (voice?.micBlocked()) setSheet('micoff');
              return;
            }
            if (micFor === 'call') startCall();
            else notify('Microphone on · hold the mic to record');
          }}
        />
        <Button title="Don’t allow" variant="ghost" size="md" onPress={() => setSheet(null)} />
      </Sheet>
      <Sheet open={sheet === 'micoff'} onClose={() => setSheet(null)}>
        <Notice
          icon="micOff"
          tileBg={color.dangerBg}
          tileFg={color.danger}
          title="Microphone is off for Meshtrike"
          body={micFor === 'call' ? 'Turn it on in Android settings to talk in nearby calls.' : 'Turn it on in Android settings to record voice notes.'}
        />
        <Button
          title="Open settings"
          onPress={() => {
            setSheet(null);
            client.hooks.openAppSettings?.();
          }}
        />
        <Button title="Not now" variant="ghost" size="md" onPress={() => setSheet(null)} />
      </Sheet>
      <Sheet open={sheet === 'hrcall'} onClose={() => setSheet(null)}>
        <Notice
          icon="zap"
          tileBg={color.ink}
          tileFg={color.lime}
          title="Calls are off in High-Risk mode"
          body="A call keeps your radio transmitting the whole time, which equipment nearby can use to locate you. Send a message instead."
        />
        <Button title="OK" onPress={() => setSheet(null)} />
      </Sheet>
      <Sheet open={sheet === 'video'} onClose={() => setSheet(null)}>
        <Notice
          icon="video"
          tileBg={color.chip2}
          title="Open in your phone’s video player?"
          body="Meshtrike never plays videos itself. The video leaves Meshtrike’s protection while it is open in another app."
        />
        <Button
          title="Open"
          onPress={async () => {
            setSheet(null);
            if (!videoMsg || !client.hooks.video) return notify('Not available on this device');
            try {
              await client.hooks.video.open(videoMsg.id);
            } catch (e) {
              notify(errorText(e));
            }
          }}
        />
        <Button title="Cancel" variant="ghost" size="md" onPress={() => setSheet(null)} />
      </Sheet>
      {viewer ? <PhotoViewer m={viewer.m} img={viewer.img} who={viewer.m.outgoing ? 'You' : isDm ? summary.title.split(' ')[0] : viewer.m.sender} onClose={() => setViewer(null)} /> : null}
    </Appear>
  );
}

function AttachTile({ icon, title, sub, disabled, onPress }: { icon: IconName; title: string; sub: string; disabled: boolean; onPress: () => void }) {
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={title}
      accessibilityState={{ disabled }}
      style={({ pressed }) => [s.attachTile, pressed && !disabled ? { opacity: 0.75 } : null]}
    >
      <Icon name={icon} size={24} tint={disabled ? color.disabled : color.ink} />
      <Text style={{ fontFamily: font.semi, fontSize: 15, color: disabled ? color.disabled : color.ink }}>{title}</Text>
      <Text style={{ fontFamily: font.regular, fontSize: 12, lineHeight: 16, color: color.textMuted }}>{sub}</Text>
    </Pressable>
  );
}

function CarrierChip({ carrier, text }: { carrier: Carrier; text: string }) {
  const c = CARRIER[carrier];
  return (
    <Appear kind="pop" style={{ alignSelf: 'center', marginVertical: 5 }}>
      <View accessibilityRole="text" style={[s.carrChip, { backgroundColor: c.bg }]}>
        <Icon name={c.icon} size={14} tint={c.fg} />
        <Text style={{ fontFamily: font.semi, fontSize: 12, color: c.fg }}>{text}</Text>
      </View>
    </Appear>
  );
}

function BlinkDots() {
  const v = useLoop(1400);
  const op = (d: number) => v.interpolate({ inputRange: [0, d, d + 0.2, Math.min(1, d + 0.4), 1], outputRange: [0.2, 0.2, 1, 0.2, 0.2], extrapolate: 'clamp' });
  return (
    <View style={{ flexDirection: 'row', gap: 3 }}>
      {[0.1, 0.25, 0.4].map((d) => (
        <Animated.View key={d} style={{ width: 5, height: 5, borderRadius: 3, backgroundColor: color.textMuted, opacity: op(d) }} />
      ))}
    </View>
  );
}

function Meta({ m, onInk }: { m: Message; onInk: boolean }) {
  const st = m.outgoing ? STATE[m.state] : undefined;
  const metaFg = onInk ? 'rgba(255,255,255,0.7)' : color.textFaint;
  const stFg = m.state === 'Read' ? (onInk ? color.lime : color.limeInk) : m.state === 'Failed' ? (onInk ? '#FFB38A' : color.danger) : metaFg;
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!m.expires_at) return;
    const t = setInterval(() => setNow(Date.now()), 30000);
    return () => clearInterval(t);
  }, [m.expires_at]);
  // Only the carrier the core recorded for this message; nothing when unknown.
  const c = m.via ? CARRIER[VIA_CARRIER[m.via]] : null;
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 5, alignSelf: 'flex-end' }}>
      {m.expires_at && !m.outgoing ? (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 3 }}>
          <Icon name="timer" size={12} tint={metaFg} />
          <Text style={{ fontFamily: font.medium, fontSize: 11, color: metaFg }}>{countdown(m.expires_at - now)} ·</Text>
        </View>
      ) : null}
      {m.outgoing && c && m.via ? (
        <View style={{ opacity: 0.85 }} accessibilityLabel={`Sent via ${VIA_LABEL[m.via]}`}>
          <Icon name={c.icon} size={12} tint={metaFg} />
        </View>
      ) : null}
      <Text style={{ fontFamily: font.medium, fontSize: 11, color: metaFg }}>{timeAgo(m.sent_at)}</Text>
      {st ? (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 3 }}>
          <Icon name={st.icon} size={14} tint={stFg} />
          <Text style={{ fontFamily: font.medium, fontSize: 11, color: stFg }}>{m.kind === 'Voice' && m.state === 'Read' ? 'Listened' : st.label}</Text>
        </View>
      ) : null}
    </View>
  );
}

function Bubble({
  m,
  orig,
  onOpenVideo,
  kind,
  peerName,
  waitWhy,
  highlighted,
  voiceUi,
  onOpenPhoto,
  onJumpTo,
  unknownSender,
  onLongPress,
}: {
  m: Message;
  /** The message this one replies to, when it is on this phone. */
  orig?: Message;
  onOpenVideo: () => void;
  kind: ConversationSummary['kind'];
  peerName: string;
  waitWhy: string;
  highlighted: boolean;
  voiceUi: VoiceUi;
  onOpenPhoto: (img: RgbaImage) => void;
  onJumpTo: (id: string) => void;
  unknownSender: boolean;
  onLongPress: () => void;
}) {
  const { call, notify, client } = useApp();
  const [img, setImg] = useState<RgbaImage | null>(null);
  const [imgError, setImgError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState(false);
  const complete = m.chunks_have >= m.chunks_total;
  const showImage = useCallback(async () => {
    setRevealed(true);
    try {
      setImg(await call<RgbaImage>('image_pixels', { message: m.id }));
    } catch (e) {
      setImgError(errorText(e));
    }
  }, [call, m.id]);
  useEffect(() => {
    // Photos from verified contacts (and our own) display automatically; others need a tap.
    if (m.kind === 'Image' && complete && (m.outgoing || m.sender_verified) && !img) showImage();
  }, [m.kind, complete, m.outgoing, m.sender_verified, img, showImage]);

  if (m.kind === 'System') {
    return (
      <View style={s.sysPill}>
        <Text style={{ fontFamily: font.medium, fontSize: 12, lineHeight: 17, color: color.textMuted, textAlign: 'center' }}>{m.text}</Text>
      </View>
    );
  }
  if (m.kind === 'Call') {
    const missed = m.text.startsWith('Missed');
    const fg = missed ? color.danger : color.inkSoft;
    const text = missed ? `${m.text} · ${timeAgo(m.sent_at)}` : m.text;
    return (
      <Appear kind="pop" style={[s.callPill, { backgroundColor: missed ? color.dangerBg : color.sunk }]}>
        <Icon name={missed ? 'callMissed' : 'call'} size={14} tint={fg} />
        <Text style={{ fontFamily: font.semi, fontSize: 12, color: fg }} accessibilityLabel={text}>
          {text}
        </Text>
      </Appear>
    );
  }

  const st = m.outgoing ? STATE[m.state] : undefined;
  const held = m.state === 'Held';
  const failed = m.state === 'Failed';
  const video = isVideo(m);
  const what =
    m.kind === 'Text'
      ? m.text
      : m.kind === 'Image'
        ? 'Photo'
        : m.kind === 'Voice'
          ? `Voice note ${fmtMs(m.voice?.duration_ms ?? 0)}`
          : video
            ? `Video ${m.text}`
            : `File ${m.text}`;
  const a11y = `${m.outgoing ? 'You' : m.sender}: ${what}${st ? `, ${st.label}` : ''}`;

  const release = () => call('release_held', { message: m.id }).catch((e) => notify(errorText(e)));
  const retry = async () => {
    // Retry = a new message with a new ID (15.3).
    try {
      await call('send_text', { conversation: m.conversation, text: m.text });
      await call('delete_message', { message: m.id, for_everyone: false });
      notify('Trying again');
    } catch (e) {
      notify(errorText(e));
    }
  };
  const save = async () => {
    // Files are never opened or previewed in the app (12.3).
    try {
      const r = await call<{ name: string; bytes: string }>('file_bytes', { message: m.id });
      if (!client.hooks.saveFile) throw new Error('Saving files is not available on this device');
      await client.hooks.saveFile(r.name, b64decode(r.bytes));
      notify('Saved to Files · open with care');
    } catch (e) {
      notify(errorText(e));
    }
  };

  const fromLine =
    !m.outgoing && kind !== 'Direct' ? (
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, paddingLeft: 12 }}>
        <Text style={{ fontFamily: font.semi, fontSize: 12.5, color: color.inkSoft }}>{m.sender}</Text>
        {kind === 'Group' && m.sender_verified ? <Icon name="shieldOk" size={13} tint={color.good} /> : null}
        {unknownSender ? (
          <View style={s.note}>
            <Text style={{ fontFamily: font.medium, fontSize: 11, color: color.grey }}>Not in your contacts</Text>
          </View>
        ) : null}
      </View>
    ) : null;

  let body: React.ReactNode;
  const quote = m.reply ? <QuoteBlock reply={m.reply} orig={orig} onInk={m.outgoing && !held} onJumpTo={onJumpTo} /> : null;
  if (m.kind === 'Voice') {
    body = <VoiceBubble m={m} ui={voiceUi} held={held} quote={quote} />;
  } else if (video) {
    body = <VideoCard m={m} onOpen={onOpenVideo} />;
  } else if (m.kind === 'Image') {
    const photoW = 236;
    if (m.outgoing) {
      body = (
        <View style={s.photoOut}>
          <View style={s.photoInner}>
            {img ? (
              <Pressable onPress={() => onOpenPhoto(img)} accessibilityRole="imagebutton" accessibilityLabel="Open photo">
                <RgbaView image={img} maxWidth={photoW - 8} />
              </Pressable>
            ) : (
              <View style={{ height: 220 }} />
            )}
            {!img ? <Text style={s.photoTag}>{`PHOTO${m.file_size ? ` · ${bytesLabel(m.file_size)}` : ''}`}</Text> : null}
            {!complete ? <PartsOverlay have={m.chunks_have} total={m.chunks_total} via={m.via} /> : null}
          </View>
          <View style={{ paddingHorizontal: 10, paddingBottom: 4 }}>
            <Meta m={m} onInk />
          </View>
        </View>
      );
    } else if (!complete) {
      body = (
        <View style={[s.photoHidden, { justifyContent: 'flex-end', padding: 10 }]}>
          <PartsOverlay have={m.chunks_have} total={m.chunks_total} via={m.via} />
        </View>
      );
    } else if (img) {
      body = (
        <Pressable
          onPress={() => onOpenPhoto(img)}
          accessibilityRole="imagebutton"
          accessibilityLabel="Open photo"
          style={{ borderRadius: 24, borderBottomLeftRadius: 6, overflow: 'hidden' }}
        >
          <RgbaView image={img} maxWidth={photoW} />
        </Pressable>
      );
    } else {
      body = (
        <Pressable onPress={showImage} accessibilityRole="button" accessibilityLabel="Tap to show photo" style={s.photoHidden}>
          <Svg pointerEvents="none" style={StyleSheet.absoluteFill}>
            <Defs>
              <Pattern id="stripes" width="20" height="20" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                <Rect width="20" height="20" fill="#ECE9E2" />
                <Rect width="10" height="20" fill="#E3E0D8" />
              </Pattern>
            </Defs>
            <Rect width="100%" height="100%" fill="url(#stripes)" />
          </Svg>
          <View style={s.eyeDisc}>
            <Icon name="eye" size={24} />
          </View>
          <Text style={{ fontFamily: font.semi, fontSize: 14, color: color.ink }}>{revealed && !imgError ? 'Loading…' : 'Tap to show'}</Text>
          <Text style={{ fontFamily: font.regular, fontSize: 12, lineHeight: 16, color: color.textMuted, maxWidth: 180, textAlign: 'center' }}>
            {imgError ?? `Hidden because ${kind === 'Direct' ? peerName : m.sender} isn’t verified`}
          </Text>
        </Pressable>
      );
    }
  } else if (m.kind === 'File') {
    const ext = (m.text.split('.').pop() ?? '').slice(0, 4).toUpperCase();
    const onInk = m.outgoing;
    body = (
      <View style={[s.fileCard, onInk ? { width: 262, backgroundColor: color.ink, borderColor: color.ink, borderBottomRightRadius: 6, borderBottomLeftRadius: 22, gap: 8 } : null]}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 11 }}>
          <View style={[s.fileIcon, onInk ? { backgroundColor: 'rgba(255,255,255,0.12)' } : null]}>
            <Icon name="file" size={20} tint={onInk ? '#fff' : color.ink} />
            <Text style={{ position: 'absolute', bottom: 5, fontFamily: font.monoSemi, fontSize: 8, color: onInk ? '#fff' : color.ink }}>{ext}</Text>
          </View>
          <View style={{ flex: 1, minWidth: 0, gap: 3 }}>
            <Text style={{ fontFamily: font.semi, fontSize: 14, color: onInk ? '#fff' : color.ink }} numberOfLines={1}>
              {m.text}
            </Text>
            <Text style={{ fontFamily: font.medium, fontSize: 12, color: onInk ? 'rgba(255,255,255,0.7)' : color.textMuted }}>
              {complete ? (m.outgoing ? bytesLabel(m.file_size) : `${bytesLabel(m.file_size)} · never opened in-app`) : receivingLabel(m)}
            </Text>
          </View>
        </View>
        {!complete ? <ProgressBar have={m.chunks_have} total={m.chunks_total} tint={onInk ? color.lime : color.ink} track={onInk ? 'rgba(255,255,255,0.18)' : color.chip2} /> : null}
        {onInk ? (
          <Meta m={m} onInk />
        ) : (
          <View style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' }}>
            {complete ? (
              <Pressable onPress={save} accessibilityRole="button" accessibilityLabel="Save to Files" style={s.saveBtn}>
                <Icon name="download" size={14} tint="#fff" />
                <Text style={{ fontFamily: font.semi, fontSize: 12.5, color: '#fff' }}>Save to Files</Text>
              </Pressable>
            ) : (
              <View />
            )}
            <Text style={{ fontFamily: font.medium, fontSize: 11, color: color.textFaint }}>{timeAgo(m.sent_at)}</Text>
          </View>
        )}
      </View>
    );
  } else {
    body = (
      <View
        style={[
          m.outgoing ? s.out : s.in,
          m.outgoing && held ? s.outHeld : null,
          m.outgoing && failed ? { borderColor: color.dangerLine } : null,
        ]}
      >
        {quote}
        <Text style={{ fontFamily: font.regular, fontSize: 15.5, lineHeight: 22, color: m.outgoing && !held ? '#fff' : color.ink }}>{m.text}</Text>
        <Meta m={m} onInk={m.outgoing && !held} />
      </View>
    );
  }

  return (
    <Appear kind="bubble" style={{ alignItems: m.outgoing ? 'flex-end' : 'flex-start', gap: 5, marginVertical: 5 }}>
      {fromLine}
      <Pressable
        onLongPress={onLongPress}
        delayLongPress={400}
        accessibilityLabel={a11y}
        accessibilityHint="Swipe right to reply. Long-press for actions"
        style={[{ maxWidth: m.kind === 'Voice' ? '84%' : '80%', borderRadius: 27, borderWidth: 3, borderColor: highlighted ? color.lime : 'transparent', margin: -5, padding: 2 }]}
      >
        {body}
      </Pressable>
      {m.can_start_chat ? (
        <Pressable
          onPress={async () => {
            try {
              await call('pair_from_post', { message: m.id });
              notify(`Request sent to ${m.sender}`);
            } catch (e) {
              notify(errorText(e));
            }
          }}
          accessibilityRole="button"
          accessibilityLabel="Start private chat"
          style={{ flexDirection: 'row', alignItems: 'center', gap: 5, marginLeft: 8, padding: 4 }}
        >
          <Icon name="chat" size={13} />
          <Text style={{ fontFamily: font.semi, fontSize: 12, color: color.ink }}>Start private chat</Text>
        </Pressable>
      ) : null}
      {held ? (
        <View style={s.statusLine}>
          <Text style={s.statusText}>{m.release_at ? `Sends at ${timeAgo(m.release_at)}` : 'Held until you release it'}</Text>
          <Pressable onPress={release} accessibilityRole="button" accessibilityLabel="Release now" style={s.releaseBtn}>
            <Text style={{ fontFamily: font.semi, fontSize: 12, color: color.ink }}>Release now</Text>
          </Pressable>
        </View>
      ) : null}
      {m.outgoing && m.state === 'Sent' && m.via ? (
        <View style={s.statusLine}>
          <Icon name={CARRIER[VIA_CARRIER[m.via]].icon} size={13} tint={color.textMuted} />
          <Text style={s.statusText}>Sent via {VIA_LABEL[m.via]}</Text>
        </View>
      ) : null}
      {m.outgoing && m.state === 'Queued' ? (
        <View style={s.statusLine}>
          <BlinkDots />
          <Text style={[s.statusText, { flexShrink: 1, textAlign: 'right' }]}>{waitWhy}</Text>
        </View>
      ) : null}
      {failed ? (
        <View style={s.statusLine}>
          <Text style={[s.statusText, { color: color.danger }]}>Not delivered · expired before it reached them</Text>
          {m.kind === 'Text' ? (
            <Pressable onPress={retry} accessibilityRole="button" accessibilityLabel="Retry" style={s.retryBtn}>
              <Icon name="retry" size={13} tint={color.danger} />
              <Text style={{ fontFamily: font.semi, fontSize: 12, color: color.danger }}>Retry</Text>
            </Pressable>
          ) : null}
        </View>
      ) : null}
    </Appear>
  );
}

// ===== Voice notes =====

type Playback = { id: string | null; pos: number; dur: number; playing: boolean };
const IDLE: Playback = { id: null, pos: 0, dur: 0, playing: false };
type RecLive = { elapsed: number; levels: number[] };
type Rec = { phase: 'hold' | 'locked' | 'paused' | 'review'; durationMs: number; waveform: number[] };

/**
 * Voice state that changes many times a second (recording levels, playback
 * position). Kept out of React state so only the voice widgets redraw, not
 * the whole chat.
 */
function createLive() {
  let playback = IDLE;
  let rec: RecLive = { elapsed: 0, levels: [] };
  const subs = new Set<() => void>();
  const ping = () => subs.forEach((f) => f());
  return {
    sub: (f: () => void) => {
      subs.add(f);
      return () => {
        subs.delete(f);
      };
    },
    playback: () => playback,
    setPlayback: (p: Playback) => {
      playback = p;
      ping();
    },
    rec: () => rec,
    setRec: (r: RecLive) => {
      rec = r;
      ping();
    },
  };
}
type Live = ReturnType<typeof createLive>;

/** Playback of one voice note: idle unless it is the one playing. */
function usePlayback(live: Live, id: string) {
  return useSyncExternalStore(live.sub, () => {
    const p = live.playback();
    return p.id === id ? p : IDLE;
  });
}

function useRecLive(live: Live) {
  return useSyncExternalStore(live.sub, live.rec);
}

type VoiceUi = {
  live: Live;
  speed: number;
  toggle: (m: Message) => void;
  seek: (m: Message, frac: number) => void;
  cycleSpeed: () => void;
};

function fmtMs(ms: number) {
  const t = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`;
}

/** A waveform squeezed (or stretched) into `n` bars, loudest per slot. */
function fitBars(src: number[], n: number): number[] {
  if (n <= 0) return [];
  if (!src.length) return Array.from({ length: n }, () => 50);
  return Array.from({ length: n }, (_, i) => {
    const a = Math.floor((i * src.length) / n);
    const b = Math.max(a + 1, Math.floor(((i + 1) * src.length) / n));
    return Math.max(...src.slice(a, b));
  });
}

/**
 * Waveform bars that always fit the width they get (3 px bars, 2 px gaps);
 * tap or drag along them to seek, with the played part following the finger.
 */
function SeekBars({
  bars,
  played,
  on,
  rest,
  barH = 21,
  disabled,
  onSeek,
}: {
  bars: number[];
  played: number;
  on: string;
  rest: string;
  barH?: number;
  disabled?: boolean;
  onSeek: (frac: number) => void;
}) {
  const [w, setW] = useState(0);
  const [dragFrac, setDragFrac] = useState<number | null>(null);
  const width = useRef(1);
  const seek = useRef(onSeek);
  seek.current = onSeek;
  const off = useRef(disabled);
  off.current = disabled;
  const n = w ? Math.max(8, Math.floor((w + 2) / 5)) : 0;
  const shown = useMemo(() => fitBars(bars, n), [bars, n]);
  const pan = useMemo(() => {
    const frac = (x: number) => Math.min(1, Math.max(0, x / width.current));
    return PanResponder.create({
      onStartShouldSetPanResponder: () => !off.current,
      onMoveShouldSetPanResponder: (_, g) => !off.current && Math.abs(g.dx) > 4,
      onPanResponderTerminationRequest: () => false,
      onPanResponderGrant: (e) => setDragFrac(frac(e.nativeEvent.locationX)),
      onPanResponderMove: (e) => setDragFrac(frac(e.nativeEvent.locationX)),
      onPanResponderRelease: (e) => {
        setDragFrac(null);
        seek.current(frac(e.nativeEvent.locationX));
      },
      onPanResponderTerminate: () => setDragFrac(null),
    });
  }, []);
  const p = dragFrac ?? played;
  return (
    <View
      {...pan.panHandlers}
      onLayout={(e) => {
        width.current = Math.max(1, e.nativeEvent.layout.width);
        setW(e.nativeEvent.layout.width);
      }}
      accessibilityRole="adjustable"
      accessibilityLabel="Seek"
      accessibilityValue={{ min: 0, max: 100, now: Math.round(p * 100) }}
      style={{ flex: 1, minWidth: 0, height: 32, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' }}
    >
      {shown.map((b, i) => (
        // Bars ignore touches, so positions are measured on the whole strip.
        <View key={i} pointerEvents="none" style={{ width: 3, borderRadius: 1.5, height: Math.round(4 + (b / 255) * barH), backgroundColor: (i + 0.5) / shown.length <= p ? on : rest }} />
      ))}
    </View>
  );
}

/** A voice note: play/pause, waveform to tap or drag, time, speed, "new" dot. */
function VoiceBubble({ m, ui, held, quote }: { m: Message; ui: VoiceUi; held: boolean; quote: React.ReactNode }) {
  const pb = usePlayback(ui.live, m.id);
  const onInk = m.outgoing && !held;
  const complete = m.chunks_have >= m.chunks_total;
  const active = pb.id === m.id;
  const dur = active && pb.dur > 0 ? pb.dur : (m.voice?.duration_ms ?? 0);
  const played = active && dur > 0 ? pb.pos / dur : 0;
  const playing = active && pb.playing;
  const fresh = !m.outgoing && !!m.voice && !m.voice.listened && complete;
  // Longer notes get a slightly wider bubble (up to two minutes).
  const bubbleW = Math.round(212 + (Math.min(dur / 1000, 120) / 120) * 48);
  const sub = onInk ? 'rgba(255,255,255,0.7)' : color.textFaint;
  return (
    <View
      accessibilityLabel={`Voice note, ${Math.round(dur / 1000)} seconds${m.outgoing ? '' : fresh ? ', not played' : ', played'}`}
      style={[onInk ? s.vOut : s.vIn, m.outgoing && held ? s.outHeld : null, { width: bubbleW }]}
    >
      {quote}
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
        <Pressable
          onPress={() => complete && ui.toggle(m)}
          disabled={!complete}
          accessibilityRole="button"
          accessibilityLabel={playing ? 'Pause' : 'Play'}
          hitSlop={6}
          style={({ pressed }) => [s.voicePlay, { backgroundColor: onInk ? color.lime : color.ink, opacity: complete ? 1 : 0.4 }, pressed ? { transform: [{ scale: 0.92 }] } : null]}
        >
          <Icon name={playing ? 'pz' : 'play'} size={15} tint={onInk ? color.ink : color.lime} />
        </Pressable>
        {complete ? (
          <>
            <SeekBars
              bars={m.voice?.waveform ?? []}
              played={played}
              on={onInk ? color.lime : color.ink}
              rest={onInk ? 'rgba(255,255,255,0.32)' : '#C9C5BB'}
              onSeek={(f) => ui.seek(m, f)}
            />
            <Pressable
              onPress={ui.cycleSpeed}
              accessibilityRole="button"
              accessibilityLabel={`Playback speed ${ui.speed} times`}
              hitSlop={6}
              style={[s.speedPill, { backgroundColor: onInk ? 'rgba(255,255,255,0.14)' : color.chip }]}
            >
              <Text style={{ fontFamily: font.monoSemi, fontSize: 11.5, color: onInk ? '#fff' : color.ink }}>{ui.speed}×</Text>
            </Pressable>
          </>
        ) : (
          <View style={{ flex: 1, minWidth: 0, gap: 6 }}>
            <ProgressBar have={m.chunks_have} total={m.chunks_total} tint={onInk ? color.lime : color.ink} track={onInk ? 'rgba(255,255,255,0.18)' : color.chip2} />
            <Text style={{ fontFamily: font.medium, fontSize: 12, color: sub }} numberOfLines={1}>
              {`Receiving ${Math.round((m.chunks_have / Math.max(1, m.chunks_total)) * 100)}%${m.via ? ` · via ${VIA_LABEL[m.via]}` : ''}`}
            </Text>
          </View>
        )}
      </View>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, paddingLeft: 50 }}>
        <Text style={{ fontFamily: font.mono, fontSize: 11, color: sub }}>{active && (playing || pb.pos > 0) ? `${fmtMs(pb.pos)} / ${fmtMs(dur)}` : fmtMs(dur)}</Text>
        {fresh ? (
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
            <View style={s.voiceNew} />
            <Text style={{ fontFamily: font.semi, fontSize: 11, color: color.limeInk }}>new</Text>
          </View>
        ) : null}
        <View style={{ flex: 1 }} />
        <Meta m={m} onInk={onInk} />
      </View>
    </View>
  );
}

const VIDEO_EXT = /\.(mp4|m4v|mov|3gp|webm|mkv)$/i;
function isVideo(m: Message) {
  return m.kind === 'File' && VIDEO_EXT.test(m.text);
}
// Lengths read once per video (by Android's media service), for the bubble.
const videoLength = new Map<string, number>();

/** A video file: never played in the app; tap to hand it to a player. */
function VideoCard({ m, onOpen }: { m: Message; onOpen: () => void }) {
  const { client } = useApp();
  const complete = m.chunks_have >= m.chunks_total;
  const [dur, setDur] = useState<number | undefined>(videoLength.get(m.id));
  useEffect(() => {
    if (!complete || dur !== undefined || !client.hooks.video) return;
    let live = true;
    client.hooks.video
      .info(m.id)
      .then((i) => {
        videoLength.set(m.id, i.durationMs);
        if (live) setDur(i.durationMs);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [complete, dur, client, m.id]);
  const label = `${dur ? `${fmtMs(dur)} · ` : ''}${bytesLabel(m.file_size)}`;
  return (
    <View style={{ gap: 5, alignItems: m.outgoing ? 'flex-end' : 'flex-start' }}>
      <Pressable
        onPress={complete ? onOpen : undefined}
        disabled={!complete}
        accessibilityRole="button"
        accessibilityLabel={`Video, ${label}`}
        style={[s.video, m.outgoing ? { borderBottomRightRadius: 6, borderBottomLeftRadius: 24 } : null]}
      >
        <Svg pointerEvents="none" style={StyleSheet.absoluteFill}>
          <Defs>
            <LinearGradient id="vg" x1="0.3" y1="0" x2="0.7" y2="1">
              <Stop offset="0" stopColor="#5B6B5E" />
              <Stop offset="0.55" stopColor="#2A302B" />
              <Stop offset="1" stopColor="#15171C" />
            </LinearGradient>
          </Defs>
          <Rect width="100%" height="100%" fill="url(#vg)" />
        </Svg>
        <Text style={s.videoTag}>VIDEO</Text>
        {complete ? (
          <>
            <View style={s.videoPlay}>
              <Icon name="play" size={20} tint={color.ink} />
            </View>
            <Text style={s.videoMeta}>{label}</Text>
          </>
        ) : (
          <PartsOverlay have={m.chunks_have} total={m.chunks_total} via={m.via} />
        )}
      </Pressable>
      {m.outgoing ? <Meta m={m} onInk={false} /> : <Text style={{ fontFamily: font.medium, fontSize: 11, color: color.textFaint, paddingLeft: 10 }}>{timeAgo(m.sent_at)}</Text>}
    </View>
  );
}

/**
 * The mic: hold to record and release to send; slide left to cancel, up to
 * lock. The button follows the finger along one axis (whichever it moves
 * first), the lock pill fills in as it nears, and everything springs back on
 * release. Greyed where voice is off.
 */
function MicButton({
  recording,
  blocked,
  drag,
  onStart,
  onRelease,
  onCancel,
  onLock,
}: {
  recording: boolean;
  blocked: boolean;
  drag: Animated.ValueXY;
  onStart: () => Promise<boolean>;
  onRelease: () => void;
  onCancel: () => void;
  onLock: () => void;
}) {
  const cb = useRef({ onStart, onRelease, onCancel, onLock });
  cb.current = { onStart, onRelease, onCancel, onLock };
  const phase = useRef<'idle' | 'pending' | 'released' | 'rec' | 'done'>('idle');
  // Where the finger is (also while the microphone is still starting), and
  // which way it is going: cancel and lock never mix.
  const last = useRef({ dx: 0, dy: 0 });
  const axis = useRef<'x' | 'y' | null>(null);
  const pan = useMemo(() => {
    // Back to rest without overshoot (a bounce could leave it off its spot).
    const settle = () => Animated.timing(drag, { toValue: { x: 0, y: 0 }, duration: 160, easing: Easing.out(Easing.quad), useNativeDriver: false }).start();
    const follow = () => {
      const { dx, dy } = last.current;
      if (!axis.current && (Math.abs(dx) > 8 || Math.abs(dy) > 8)) axis.current = Math.abs(dx) > Math.abs(dy) ? 'x' : 'y';
      const x = axis.current === 'x' ? Math.max(-130, Math.min(0, dx)) : 0;
      const y = axis.current === 'y' ? Math.max(-90, Math.min(0, dy)) : 0;
      drag.setValue({ x, y });
      if (x <= -110) {
        phase.current = 'done';
        settle();
        cb.current.onCancel();
      } else if (y <= -70) {
        phase.current = 'done';
        settle();
        cb.current.onLock();
      }
    };
    return PanResponder.create({
      onStartShouldSetPanResponder: () => true,
      onPanResponderTerminationRequest: () => false,
      onPanResponderGrant: async () => {
        phase.current = 'pending';
        last.current = { dx: 0, dy: 0 };
        axis.current = null;
        const ok = await cb.current.onStart();
        if ((phase.current as string) === 'released') {
          // Let go while the microphone was starting: lock, cancel or send
          // according to where the finger went.
          phase.current = 'done';
          if (!ok) return;
          if (last.current.dy < -70 && Math.abs(last.current.dy) > Math.abs(last.current.dx)) cb.current.onLock();
          else if (last.current.dx < -110) cb.current.onCancel();
          else cb.current.onRelease();
        } else {
          phase.current = ok ? 'rec' : 'idle';
          if (ok) follow();
        }
      },
      onPanResponderMove: (_, g) => {
        last.current = { dx: g.dx, dy: g.dy };
        if (phase.current === 'rec') follow();
      },
      onPanResponderRelease: () => {
        if (phase.current === 'rec') {
          phase.current = 'done';
          settle();
          cb.current.onRelease();
        } else if (phase.current === 'pending') {
          phase.current = 'released';
        }
      },
      onPanResponderTerminate: () => {
        if (phase.current === 'rec') cb.current.onCancel();
        phase.current = 'done';
        settle();
      },
    });
  }, [drag]);
  // Not recording (idle, or back after a locked note was sent): always at
  // rest, whatever the last gesture left behind.
  useEffect(() => {
    if (!recording) {
      drag.stopAnimation();
      drag.setValue({ x: 0, y: 0 });
    }
  }, [recording, drag]);
  const moveX = drag.x.interpolate({ inputRange: [-130, 0], outputRange: [-130, 0], extrapolate: 'clamp' });
  const moveY = drag.y.interpolate({ inputRange: [-90, 0], outputRange: [-90, 0], extrapolate: 'clamp' });
  const lockY = drag.y.interpolate({ inputRange: [-90, 0], outputRange: [-46, 0], extrapolate: 'clamp' });
  const lockBg = drag.y.interpolate({ inputRange: [-70, -40, 0], outputRange: [color.lime, '#fff', '#fff'], extrapolate: 'clamp' });
  const arrowOp = drag.y.interpolate({ inputRange: [-60, 0], outputRange: [0, 1], extrapolate: 'clamp' });
  return (
    <View>
      {recording ? (
        <Animated.View pointerEvents="none" style={[s.lockHint, { backgroundColor: lockBg, transform: [{ translateY: lockY }] }]}>
          <Icon name="lock" size={17} tint={color.ink} />
          <Animated.View style={{ opacity: arrowOp }}>
            <Icon name="up" size={15} tint={color.textFaint} />
          </Animated.View>
        </Animated.View>
      ) : null}
      <Animated.View
        {...pan.panHandlers}
        accessible
        accessibilityRole="button"
        accessibilityLabel={blocked ? 'Voice notes unavailable' : 'Hold to record a voice note'}
        accessibilityHint={blocked ? undefined : 'Release to send. Slide left to cancel, up to lock.'}
        style={recording ? { transform: [{ translateX: moveX }, { translateY: moveY }] } : null}
      >
        {recording ? <View pointerEvents="none" style={s.micRing} /> : null}
        <View style={[s.send, { backgroundColor: recording ? color.lime : blocked ? '#E3E0D8' : color.ink, transform: [{ scale: recording ? 1.3 : 1 }] }]}>
          <Icon name={blocked ? 'micOff' : 'mic'} size={21} tint={recording ? color.ink : blocked ? '#9A968C' : '#fff'} />
        </View>
      </Animated.View>
    </View>
  );
}

const REC_RED = '#C2410C';

function BlinkDot({ size = 10 }: { size?: number }) {
  const blink = useLoop(1200);
  const op = blink.interpolate({ inputRange: [0, 0.4, 0.8, 1], outputRange: [0.2, 1, 0.2, 0.2] });
  return <Animated.View style={{ width: size, height: size, borderRadius: size / 2, backgroundColor: REC_RED, opacity: op }} />;
}

/** The last few seconds of sound while recording, newest on the right. */
function LiveBars({ live }: { live: Live }) {
  const { levels } = useRecLive(live);
  const [w, setW] = useState(0);
  const n = w ? Math.max(8, Math.floor((w + 2) / 5)) : 0;
  const pad = [...Array(Math.max(0, n - levels.length)).fill(15), ...levels.slice(-n)];
  return (
    <View
      onLayout={(e) => setW(e.nativeEvent.layout.width)}
      style={{ flex: 1, minWidth: 0, height: 30, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', overflow: 'hidden' }}
    >
      {pad.map((l, i) => (
        <View key={i} style={{ width: 3, borderRadius: 1.5, height: Math.round(4 + (l / 255) * 22), backgroundColor: color.ink }} />
      ))}
    </View>
  );
}

function RecClock({ live, size }: { live: Live; size: number }) {
  const { elapsed } = useRecLive(live);
  return <Text style={{ fontFamily: font.monoSemi, fontSize: size, color: color.ink, minWidth: size * 2.6 }}>{fmtMs(elapsed)}</Text>;
}

/** Holding the mic: the timer and "slide to cancel", where the text field was. */
function HoldRow({ live, drag }: { live: Live; drag: Animated.ValueXY }) {
  const cancelX = drag.x.interpolate({ inputRange: [-130, 0], outputRange: [-70, 0], extrapolate: 'clamp' });
  const cancelOp = drag.x.interpolate({ inputRange: [-110, 0], outputRange: [0.3, 1], extrapolate: 'clamp' });
  const near = drag.x.interpolate({ inputRange: [-110, -70, 0], outputRange: [1, 0, 0], extrapolate: 'clamp' });
  return (
    <Appear kind="in" style={[s.inputPill, s.holdRow]}>
      <BlinkDot />
      <RecClock live={live} size={15} />
      <View style={{ flex: 1, alignItems: 'flex-end', justifyContent: 'center' }}>
        <Animated.Text numberOfLines={1} style={{ fontFamily: font.medium, fontSize: 13.5, color: color.textMuted, opacity: cancelOp, transform: [{ translateX: cancelX }] }}>
          ‹ slide to cancel
        </Animated.Text>
        <Animated.View pointerEvents="none" style={{ position: 'absolute', right: 0, opacity: near }}>
          <Icon name="trash" size={18} tint={color.danger} />
        </Animated.View>
      </View>
    </Appear>
  );
}

function RoundAction({ icon, label, bg, fg, size = 46, onPress, disabled, busy }: { icon: IconName; label: string; bg: string; fg: string; size?: number; onPress: () => void; disabled?: boolean; busy?: boolean }) {
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled || busy}
      accessibilityRole="button"
      accessibilityLabel={label}
      hitSlop={6}
      style={({ pressed }) => [{ width: size, height: size, borderRadius: size / 2, backgroundColor: bg, alignItems: 'center', justifyContent: 'center' }, pressed ? { transform: [{ scale: 0.94 }] } : null]}
    >
      {busy ? <Spinner size={20} /> : <Icon name={icon} size={size > 46 ? 21 : 19} tint={fg} />}
    </Pressable>
  );
}

function RecWarn({ live }: { live: Live }) {
  const { elapsed } = useRecLive(live);
  if (elapsed < 270_000) return null;
  return (
    <View accessibilityRole="alert" style={s.recWarn}>
      <Icon name="timer" size={14} tint="#7A5200" />
      <Text style={{ fontFamily: font.semi, fontSize: 12.5, color: '#7A5200' }}>{Math.max(0, Math.ceil((300_000 - elapsed) / 1000))} seconds left</Text>
    </View>
  );
}

/** Locked or paused recording, or the finished one to listen to before sending. */
function RecPanel({
  rec,
  live,
  voiceUi,
  busy,
  onDelete,
  onPause,
  onStop,
  onSend,
  onPlayDraft,
  onSeekDraft,
}: {
  rec: Rec;
  live: Live;
  voiceUi: VoiceUi;
  busy: boolean;
  onDelete: () => void;
  onPause: () => void;
  onStop: () => void;
  onSend: () => void;
  onPlayDraft: () => void;
  onSeekDraft: (frac: number) => void;
}) {
  const pb = usePlayback(live, 'draft');
  const send = <RoundAction icon="send" label="Send voice note" bg={color.ink} fg={color.lime} size={52} onPress={onSend} busy={busy} />;
  const del = <RoundAction icon="trash" label="Delete recording" bg={color.dangerBg} fg={color.danger} onPress={onDelete} disabled={busy} />;
  if (rec.phase === 'review') {
    const active = pb.id === 'draft';
    const played = active && pb.dur > 0 ? pb.pos / pb.dur : 0;
    return (
      <Appear kind="up" style={s.recPanel}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <Pressable onPress={onPlayDraft} accessibilityRole="button" accessibilityLabel={active && pb.playing ? 'Pause' : 'Play recording'} hitSlop={6} style={[s.voicePlay, { backgroundColor: color.ink }]}>
            <Icon name={active && pb.playing ? 'pz' : 'play'} size={15} tint={color.lime} />
          </Pressable>
          <SeekBars bars={rec.waveform} played={played} on={color.ink} rest="#C9C5BB" barH={22} onSeek={onSeekDraft} />
          <Text style={{ fontFamily: font.monoSemi, fontSize: 12.5, color: color.ink }}>
            {fmtMs(active ? pb.pos : 0)} / {fmtMs(rec.durationMs)}
          </Text>
        </View>
        <View style={s.recActions}>
          {del}
          <Text style={{ fontFamily: font.medium, fontSize: 13, color: color.textMuted }}>Listen before sending</Text>
          {send}
        </View>
      </Appear>
    );
  }
  const paused = rec.phase === 'paused';
  return (
    <Appear kind="up" style={s.recPanel}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10, paddingHorizontal: 4 }}>
        {paused ? <Text style={{ fontFamily: font.monoSemi, fontSize: 11, letterSpacing: 0.7, color: color.textMuted }}>PAUSED</Text> : <BlinkDot />}
        <RecClock live={live} size={16} />
        <LiveBars live={live} />
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
          <Icon name="lock" size={13} tint={color.limeInk} />
          <Text style={{ fontFamily: font.semi, fontSize: 11, color: color.limeInk }}>Locked</Text>
        </View>
      </View>
      <RecWarn live={live} />
      <View style={s.recActions}>
        {del}
        <RoundAction icon={paused ? 'mic' : 'pz'} label={paused ? 'Resume' : 'Pause'} bg={color.chip} fg={color.ink} onPress={onPause} disabled={busy} />
        <RoundAction icon="stopF" label="Stop and review" bg={color.chip} fg={color.ink} onPress={onStop} disabled={busy} />
        {send}
      </View>
    </Appear>
  );
}

/** What a reply bar shows of the message being answered. */
function quotePreview(m: Message) {
  if (m.kind === 'Image') return 'Photo';
  if (isVideo(m)) {
    const d = videoLength.get(m.id);
    return d ? `Video · ${fmtMs(d)}` : `Video · ${m.text}`;
  }
  if (m.kind === 'File') return m.text;
  if (m.kind === 'Voice') return `Voice note · ${fmtMs(m.voice?.duration_ms ?? 0)}`;
  return m.text.split(/\s+/).join(' ');
}

/** The kind of a quoted message, as a small icon (none for text). */
function quoteIcon(m: Message): IconName | null {
  if (m.kind === 'Image') return 'image';
  if (m.kind === 'Voice') return 'mic';
  if (isVideo(m)) return 'video';
  if (m.kind === 'File') return 'file';
  return null;
}

/** The quoted original inside a reply; tap to jump to it when it is here. */
function QuoteBlock({ reply, orig, onInk, onJumpTo }: { reply: NonNullable<Message['reply']>; orig?: Message; onInk: boolean; onJumpTo: (id: string) => void }) {
  const icon = orig ? quoteIcon(orig) : null;
  const nameFg = onInk ? color.lime : color.ink;
  // The sender's snippet stays readable even when the original is gone here.
  const text = reply.quote || 'Original message unavailable';
  return (
    <Pressable
      onPress={() => reply.found && onJumpTo(reply.to)}
      disabled={!reply.found}
      accessibilityRole={reply.found ? 'button' : 'text'}
      accessibilityLabel={`Reply to ${reply.author ?? 'a message'}: ${text}${reply.found ? '. Tap to show it' : ', not on this phone'}`}
      style={[s.quote, { backgroundColor: onInk ? 'rgba(255,255,255,0.12)' : color.chip }]}
    >
      {icon ? <Icon name={icon} size={15} tint={nameFg} /> : null}
      <View style={{ flex: 1, minWidth: 0, gap: 1 }}>
        <Text style={{ fontFamily: font.semi, fontSize: 12.5, color: nameFg }} numberOfLines={1}>
          {reply.author ?? 'Earlier message'}
        </Text>
        <Text
          style={{ fontFamily: font.regular, fontSize: 13, color: onInk ? 'rgba(255,255,255,0.75)' : color.textMuted, fontStyle: reply.found ? 'normal' : 'italic' }}
          numberOfLines={1}
        >
          {text}
        </Text>
      </View>
    </Pressable>
  );
}

/**
 * Swipe a bubble right to reply (like other messengers): it follows the
 * finger, a reply disc grows in, turns lime with a tick of vibration at the
 * threshold, and on release the reply bar opens. Vertical scrolls are left
 * alone.
 */
function SwipeToReply({ enabled, onReply, children }: { enabled: boolean; onReply: () => void; children: React.ReactNode }) {
  const x = useRef(new Animated.Value(0)).current;
  const fired = useRef(false);
  const [armed, setArmed] = useState(false);
  const reduce = useReduceMotion();
  const pan = useMemo(
    () =>
      PanResponder.create({
        onMoveShouldSetPanResponder: (_, g) => enabled && g.dx > 10 && Math.abs(g.dx) > Math.abs(g.dy) * 2,
        onPanResponderGrant: () => {
          fired.current = false;
        },
        onPanResponderMove: (_, g) => {
          const d = Math.max(0, Math.min(96, g.dx * 0.85));
          x.setValue(d);
          if (d >= 64 && !fired.current) {
            fired.current = true;
            Vibration.vibrate(10);
            setArmed(true);
          } else if (d < 64 && fired.current) {
            fired.current = false;
            setArmed(false);
          }
        },
        onPanResponderRelease: () => {
          if (fired.current) onReply();
          fired.current = false;
          setArmed(false);
          Animated.spring(x, { toValue: 0, useNativeDriver: native, friction: 7 }).start();
        },
        onPanResponderTerminate: () => {
          fired.current = false;
          setArmed(false);
          Animated.spring(x, { toValue: 0, useNativeDriver: native, friction: 7 }).start();
        },
      }),
    [enabled, onReply, x],
  );
  const k = x.interpolate({ inputRange: [0, 64], outputRange: [0, 1], extrapolate: 'clamp' });
  return (
    <View {...pan.panHandlers}>
      <View pointerEvents="none" style={s.swipeIconBox}>
        <Animated.View
          style={[
            s.swipeIcon,
            { backgroundColor: armed ? color.lime : '#fff', opacity: k, transform: [{ scale: k.interpolate({ inputRange: [0, 1], outputRange: [0.6, 1] }) }] },
          ]}
        >
          <Icon name="reply" size={17} tint={color.ink} />
        </Animated.View>
      </View>
      <Animated.View style={{ transform: [{ translateX: reduce ? 0 : x }] }}>{children}</Animated.View>
    </View>
  );
}

/** "Receiving 18% · 0.9 MB of 4.8 MB" for a file still arriving. */
function receivingLabel(m: Message) {
  const pct = m.chunks_total ? Math.min(1, m.chunks_have / m.chunks_total) : 0;
  const sizes = m.file_size ? ` · ${bytesLabel(Math.round(m.file_size * pct))} of ${bytesLabel(m.file_size)}` : '';
  return `Receiving ${Math.round(pct * 100)}%${sizes}${m.via ? ` · via ${VIA_LABEL[m.via]}` : ''}`;
}

function ProgressBar({ have, total, tint, track }: { have: number; total: number; tint: string; track: string }) {
  const pct = total ? Math.min(1, have / total) : 0;
  const w = useRef(new Animated.Value(pct)).current;
  useEffect(() => {
    Animated.timing(w, { toValue: pct, duration: 250, useNativeDriver: false }).start();
  }, [pct, w]);
  return (
    <View
      accessibilityRole="progressbar"
      accessibilityValue={{ min: 0, max: 100, now: Math.round(pct * 100) }}
      style={{ height: 5, backgroundColor: track, borderRadius: 9, overflow: 'hidden' }}
    >
      <Animated.View style={{ height: 5, borderRadius: 9, backgroundColor: tint, width: w.interpolate({ inputRange: [0, 1], outputRange: ['0%', '100%'] }) }} />
    </View>
  );
}

/** Full-screen photo with Save and Share (a fresh JPEG from its pixels); double-tap to zoom. */
function PhotoViewer({ m, img, who, onClose }: { m: Message; img: RgbaImage; who: string; onClose: () => void }) {
  const { client, notify } = useApp();
  const { width } = useWindowDimensions();
  const [busy, setBusy] = useState<null | 'save' | 'share'>(null);
  const zoom = useRef(new Animated.Value(1)).current;
  const zoomed = useRef(false);
  const lastTap = useRef(0);
  const tap = () => {
    const now = Date.now();
    if (now - lastTap.current < 300) {
      zoomed.current = !zoomed.current;
      Animated.spring(zoom, { toValue: zoomed.current ? 2 : 1, friction: 8, useNativeDriver: native }).start();
      lastTap.current = 0;
    } else lastTap.current = now;
  };
  const act = async (what: 'save' | 'share') => {
    const fn = what === 'save' ? client.hooks.savePhoto : client.hooks.sharePhoto;
    if (!fn) return notify('Not available on this device');
    setBusy(what);
    try {
      await fn(m.id);
      if (what === 'save') notify('Saved to Photos · other apps on this phone can see it');
    } catch (e) {
      notify(errorText(e));
    } finally {
      setBusy(null);
    }
  };
  return (
    <Modal visible animationType="fade" onRequestClose={onClose} statusBarTranslucent>
      <View style={s.viewer} accessibilityViewIsModal>
        <View style={s.viewerTop}>
          <Pressable onPress={onClose} accessibilityRole="button" accessibilityLabel="Close photo" style={s.viewerBtnRound}>
            <Icon name="x" size={20} tint="#fff" />
          </Pressable>
          <Text style={{ fontFamily: font.semi, fontSize: 15, color: '#fff' }} numberOfLines={1}>
            {who}
          </Text>
          <Text style={{ fontFamily: font.regular, fontSize: 13, color: 'rgba(255,255,255,0.65)' }}>· {timeAgo(m.sent_at)}</Text>
        </View>
        <Pressable onPress={tap} accessibilityRole="image" accessibilityLabel="Photo" accessibilityHint="Double-tap to zoom" style={{ flex: 1, minHeight: 0, alignItems: 'center', justifyContent: 'center', overflow: 'hidden', paddingVertical: 12 }}>
          <Animated.View style={{ transform: [{ scale: zoom }] }}>
            <RgbaView image={img} maxWidth={width} />
          </Animated.View>
        </Pressable>
        <Text style={{ textAlign: 'center', fontFamily: font.medium, fontSize: 12, color: 'rgba(255,255,255,0.55)' }}>Double-tap to zoom</Text>
        <View style={s.viewerBar}>
          <Pressable onPress={() => act('save')} disabled={!!busy} accessibilityRole="button" accessibilityLabel="Save to Photos" style={[s.viewerBtn, { backgroundColor: '#fff' }]}>
            {busy === 'save' ? <Spinner size={18} /> : <Icon name="download" size={18} tint={color.ink} />}
            <Text style={[s.viewerBtnText, { color: color.ink }]}>Save to Photos</Text>
          </Pressable>
          <Pressable onPress={() => act('share')} disabled={!!busy} accessibilityRole="button" accessibilityLabel="Share" style={s.viewerBtn}>
            {busy === 'share' ? <Spinner size={18} /> : <Icon name="share" size={18} tint="#fff" />}
            <Text style={s.viewerBtnText}>Share</Text>
          </Pressable>
        </View>
      </View>
    </Modal>
  );
}

/** Icon, headline and words at the top of an explaining sheet. */
function Notice({ icon, tileBg, tileFg = color.ink, title, body }: { icon: IconName; tileBg: string; tileFg?: string; title: string; body: string }) {
  return (
    <View style={{ gap: 12 }}>
      <View style={{ marginLeft: 4 }}>
        <IconTile icon={icon} bg={tileBg} fg={tileFg} size={48} />
      </View>
      <Text style={{ fontFamily: font.display, fontSize: 22, lineHeight: 25, color: color.ink, paddingHorizontal: 4 }}>{title}</Text>
      <Text style={{ fontFamily: font.regular, fontSize: 14, lineHeight: 21, color: color.textMuted, paddingHorizontal: 4 }}>{body}</Text>
    </View>
  );
}

function PartsOverlay({ have, total, via }: { have: number; total: number; via: Via | null }) {
  const pct = total ? Math.min(1, have / total) : 0;
  const w = useRef(new Animated.Value(pct)).current;
  useEffect(() => {
    Animated.timing(w, { toValue: pct, duration: 250, useNativeDriver: false }).start();
  }, [pct, w]);
  const fast = via === 'WifiP2p';
  const tint = fast ? '#5EEAD4' : color.lime;
  return (
    <View style={s.parts} accessibilityLabel={`Receiving, ${Math.round(pct * 100)} percent`}>
      <View style={{ flexDirection: 'row', justifyContent: 'space-between' }}>
        <Text style={{ fontFamily: font.semi, fontSize: 12, color: '#fff' }}>
          Receiving {Math.round(pct * 100)}%
        </Text>
        <Text style={{ fontFamily: font.semi, fontSize: 12, color: tint }}>{via ? VIA_LABEL[via] : ''}</Text>
      </View>
      <View style={{ height: 5, backgroundColor: 'rgba(255,255,255,0.18)', borderRadius: 9, overflow: 'hidden' }}>
        <Animated.View style={{ height: 5, borderRadius: 9, backgroundColor: tint, width: w.interpolate({ inputRange: [0, 1], outputRange: ['0%', '100%'] }) }} />
      </View>
    </View>
  );
}

function Timeline({ m }: { m: Message }) {
  const order: DeliveryState[] = ['Held', 'Queued', 'Sent', 'Delivered', 'Read'];
  const at = order.indexOf(m.state);
  const reached = (s: DeliveryState) => m.state !== 'Failed' && at >= order.indexOf(s);
  const rows: { label: string; icon: IconName; done: boolean; time: string }[] = [
    { label: 'Written', icon: 'pen', done: true, time: timeAgo(m.sent_at) },
    ...(m.state === 'Held' ? [{ label: 'Held by you', icon: 'pause' as IconName, done: true, time: m.release_at ? `until ${timeAgo(m.release_at)}` : '—' }] : []),
    {
      label: m.via ? `Left via ${VIA_LABEL[m.via]}` : 'Left the phone',
      icon: m.via ? CARRIER[VIA_CARRIER[m.via]].icon : 'send',
      done: reached('Sent'),
      time: reached('Sent') ? 'Done' : 'Pending',
    },
    { label: 'Delivered', icon: 'checks', done: reached('Delivered'), time: reached('Delivered') ? 'Done' : 'Pending' },
    // Receipts are optional (and off in groups), so an unread mark is not a failure.
    { label: 'Read', icon: 'eye', done: reached('Read'), time: reached('Read') ? 'Done' : reached('Delivered') ? 'If they share receipts' : 'Pending' },
  ];
  if (m.state === 'Failed') rows.push({ label: 'Not delivered · expired before it reached them', icon: 'alertC', done: true, time: '' });
  return (
    <View style={{ paddingHorizontal: 4 }}>
      {rows.map((r, i) => (
        <Appear key={r.label} kind="pop" delay={i * 60} style={{ flexDirection: 'row', alignItems: 'center', gap: 14, minHeight: 52 }}>
          <View
            style={{
              width: 34,
              height: 34,
              borderRadius: 17,
              borderWidth: 2,
              borderColor: r.done ? color.ink : color.dash,
              backgroundColor: r.done ? color.ink : '#fff',
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            <Icon name={r.icon} size={16} tint={r.done ? color.lime : '#9A968C'} />
          </View>
          <Text style={{ flex: 1, fontFamily: font.semi, fontSize: 15, color: r.done ? color.ink : '#8A867D' }}>{r.label}</Text>
          <Text style={{ fontFamily: font.mono, fontSize: 13, color: color.textFaint }}>{r.time}</Text>
        </Appear>
      ))}
    </View>
  );
}

const s = StyleSheet.create({
  voicePlay: { width: 40, height: 40, borderRadius: 20, alignItems: 'center', justifyContent: 'center' },
  voiceNew: { width: 7, height: 7, borderRadius: 4, backgroundColor: '#7FA81A' },
  speedPill: { height: 26, minWidth: 40, borderRadius: 99, paddingHorizontal: 8, alignItems: 'center', justifyContent: 'center' },
  vIn: { backgroundColor: '#fff', borderWidth: 1, borderColor: color.border, borderRadius: 22, borderBottomLeftRadius: 6, paddingTop: 10, paddingRight: 12, paddingBottom: 8, paddingLeft: 10, gap: 6, maxWidth: '100%' },
  vOut: { backgroundColor: color.ink, borderWidth: 1.5, borderColor: color.ink, borderRadius: 22, borderBottomRightRadius: 6, paddingTop: 10, paddingRight: 12, paddingBottom: 8, paddingLeft: 10, gap: 6, maxWidth: '100%' },
  video: { width: 236, height: 176, borderRadius: 24, borderBottomLeftRadius: 6, overflow: 'hidden', backgroundColor: '#2A302B' },
  videoTag: { position: 'absolute', left: 12, top: 12, fontFamily: font.monoSemi, fontSize: 10.5, letterSpacing: 0.6, color: 'rgba(255,255,255,0.6)' },
  videoPlay: { position: 'absolute', left: 90, top: 60, width: 56, height: 56, borderRadius: 28, backgroundColor: 'rgba(255,255,255,0.92)', alignItems: 'center', justifyContent: 'center' },
  videoMeta: { position: 'absolute', left: 12, bottom: 10, fontFamily: font.monoSemi, fontSize: 12, color: '#fff' },
  callPill: { alignSelf: 'center', flexDirection: 'row', alignItems: 'center', gap: 7, paddingVertical: 6, paddingLeft: 10, paddingRight: 12, borderRadius: 99, marginVertical: 5 },
  holdRow: { height: 46, minHeight: 46, paddingHorizontal: 16, gap: 10, overflow: 'hidden' },
  micRing: { position: 'absolute', left: -10, top: -10, width: 66, height: 66, borderRadius: 33, backgroundColor: 'rgba(205,242,92,0.3)', transform: [{ scale: 1.3 }] },
  lockHint: { position: 'absolute', bottom: 64, left: 3, width: 40, paddingVertical: 10, borderRadius: 99, backgroundColor: '#fff', alignItems: 'center', gap: 6, elevation: 6, shadowColor: '#000', shadowOpacity: 0.3, shadowRadius: 9, shadowOffset: { width: 0, height: 6 } },
  recPanel: { marginHorizontal: 10, marginTop: 6, backgroundColor: '#fff', borderRadius: 24, padding: 12, gap: 12, elevation: 1, shadowColor: '#000', shadowOpacity: 0.06, shadowRadius: 2 },
  recActions: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' },
  recWarn: { flexDirection: 'row', alignItems: 'center', gap: 8, backgroundColor: '#FFF1C9', borderRadius: 12, paddingVertical: 8, paddingHorizontal: 12 },
  replyBar: { flexDirection: 'row', alignItems: 'center', gap: 10, marginHorizontal: 10, marginTop: 6, paddingTop: 8, paddingBottom: 8, paddingLeft: 8, paddingRight: 6, borderRadius: 18, backgroundColor: '#fff', elevation: 1, shadowColor: '#000', shadowOpacity: 0.06, shadowRadius: 2 },
  replyTile: { width: 32, height: 32, borderRadius: 11, backgroundColor: color.lime, alignItems: 'center', justifyContent: 'center' },
  replyCancel: { width: 36, height: 36, borderRadius: 18, backgroundColor: color.chip, alignItems: 'center', justifyContent: 'center' },
  quote: { flexDirection: 'row', alignItems: 'center', gap: 8, borderRadius: 14, paddingVertical: 7, paddingHorizontal: 10, marginHorizontal: -6 },
  swipeIconBox: { position: 'absolute', left: 0, top: 0, bottom: 0, justifyContent: 'center' },
  swipeIcon: { width: 34, height: 34, borderRadius: 17, alignItems: 'center', justifyContent: 'center', elevation: 3, shadowColor: '#000', shadowOpacity: 0.25, shadowRadius: 5, shadowOffset: { width: 0, height: 2 } },
  highlighted: { backgroundColor: 'rgba(200,240,90,0.35)' },
  preparing: { flexDirection: 'row', alignItems: 'center', gap: 12, marginHorizontal: 12, marginTop: 8, padding: 12, borderRadius: 18, backgroundColor: '#fff', borderWidth: 1, borderColor: color.line },
  viewer: { flex: 1, backgroundColor: '#0B0C0F' },
  viewerTop: { flexDirection: 'row', alignItems: 'center', gap: 10, paddingHorizontal: 14, paddingTop: 52, paddingBottom: 4 },
  viewerBtnRound: { width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center', backgroundColor: 'rgba(255,255,255,0.1)' },
  viewerBar: { flexDirection: 'row', gap: 10, paddingHorizontal: 16, paddingTop: 14, paddingBottom: 34 },
  viewerBtn: { flex: 1, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 8, height: 52, borderRadius: 26, backgroundColor: 'rgba(255,255,255,0.12)' },
  viewerBtnText: { fontFamily: font.semi, fontSize: 15, color: '#fff' },
  header: { backgroundColor: '#fff', borderBottomWidth: 1, borderBottomColor: '#ECE9E2', paddingTop: 2, paddingBottom: 10, gap: 8, zIndex: 2 },
  headTop: { flexDirection: 'row', alignItems: 'center', gap: 6, paddingLeft: 4, paddingRight: 12 },
  back: { width: 44, height: 44, alignItems: 'center', justifyContent: 'center' },
  details: { width: 40, height: 40, borderRadius: 14, backgroundColor: color.chip, alignItems: 'center', justifyContent: 'center' },
  headRow: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: 8, paddingLeft: 16, paddingRight: 12 },
  timerBtn: { flexDirection: 'row', alignItems: 'center', gap: 5, padding: 6 },
  sweepTrack: { position: 'absolute', left: 0, right: 0, bottom: -1, height: 2, overflow: 'hidden' },
  channelBar: { backgroundColor: color.ink, paddingVertical: 10, paddingHorizontal: 16, flexDirection: 'row', alignItems: 'center', gap: 10 },
  sysPill: { alignSelf: 'center', maxWidth: '85%', paddingVertical: 5, paddingHorizontal: 12, borderRadius: 99, backgroundColor: color.sunk, marginVertical: 5 },
  carrChip: { flexDirection: 'row', alignItems: 'center', gap: 7, paddingVertical: 6, paddingLeft: 9, paddingRight: 12, borderRadius: 99 },
  in: {
    backgroundColor: '#fff',
    borderWidth: 1,
    borderColor: color.border,
    borderRadius: 22,
    borderBottomLeftRadius: 6,
    paddingTop: 10,
    paddingHorizontal: 14,
    paddingBottom: 8,
    gap: 6,
  },
  out: {
    backgroundColor: color.ink,
    borderWidth: 1.5,
    borderColor: color.ink,
    borderRadius: 22,
    borderBottomRightRadius: 6,
    paddingTop: 10,
    paddingHorizontal: 14,
    paddingBottom: 8,
    gap: 6,
  },
  outHeld: { backgroundColor: color.chip2, borderColor: '#A19D93', borderStyle: 'dashed' },
  note: { paddingHorizontal: 7, paddingVertical: 2, borderRadius: 99, borderWidth: 1.5, borderStyle: 'dashed', borderColor: color.disabled },
  statusLine: { flexDirection: 'row', alignItems: 'center', gap: 8, maxWidth: '88%' },
  prepCard: { alignSelf: 'flex-end', width: 262, marginVertical: 5, flexDirection: 'row', alignItems: 'flex-start', gap: 12, backgroundColor: '#fff', borderWidth: 1.5, borderStyle: 'dashed', borderColor: '#C9C5BB', borderRadius: 22, borderBottomRightRadius: 6, paddingVertical: 12, paddingHorizontal: 14 },
  routesCard: { backgroundColor: color.chip, borderRadius: 20, paddingHorizontal: 14, paddingVertical: 2 },
  routeRow: { flexDirection: 'row', alignItems: 'flex-start', gap: 12, paddingVertical: 12, borderBottomWidth: 1, borderBottomColor: '#E6E3DC' },
  routeDot: { width: 28, height: 28, borderRadius: 14, alignItems: 'center', justifyContent: 'center' },
  statusText: { fontFamily: font.medium, fontSize: 12, color: color.textMuted, flexShrink: 1 },
  releaseBtn: { height: 30, borderRadius: 99, backgroundColor: color.lime, paddingHorizontal: 12, alignItems: 'center', justifyContent: 'center' },
  retryBtn: { height: 30, borderRadius: 99, borderWidth: 1.5, borderColor: color.danger, backgroundColor: '#fff', paddingHorizontal: 11, flexDirection: 'row', alignItems: 'center', gap: 5 },
  photoOut: { width: 236, borderRadius: 24, borderBottomRightRadius: 6, backgroundColor: color.ink, padding: 4, gap: 6 },
  photoInner: { borderRadius: 20, borderBottomRightRadius: 4, overflow: 'hidden', backgroundColor: '#C9C2AE', minHeight: 120 },
  photoTag: { position: 'absolute', left: 12, top: 12, fontFamily: font.monoSemi, fontSize: 10.5, letterSpacing: 0.6, color: 'rgba(21,23,28,0.55)' },
  parts: { position: 'absolute', left: 10, right: 10, bottom: 10, backgroundColor: 'rgba(21,23,28,0.78)', borderRadius: 16, paddingVertical: 11, paddingHorizontal: 12, gap: 8 },
  photoHidden: {
    width: 236,
    height: 220,
    borderRadius: 24,
    borderBottomLeftRadius: 6,
    overflow: 'hidden',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 10,
  },
  eyeDisc: { width: 52, height: 52, borderRadius: 26, backgroundColor: '#fff', alignItems: 'center', justifyContent: 'center', elevation: 3, shadowColor: '#000', shadowOpacity: 0.2, shadowRadius: 8 },
  fileCard: { width: 268, backgroundColor: '#fff', borderWidth: 1, borderColor: color.border, borderRadius: 22, borderBottomLeftRadius: 6, padding: 12, gap: 10 },
  fileIcon: { width: 44, height: 52, borderRadius: 12, backgroundColor: color.chip2, alignItems: 'center', justifyContent: 'center' },
  saveBtn: { height: 34, borderRadius: 99, backgroundColor: color.ink, paddingHorizontal: 13, flexDirection: 'row', alignItems: 'center', gap: 6 },
  composer: { flexDirection: 'row', alignItems: 'flex-end', gap: 8, paddingTop: 8, paddingHorizontal: 10, paddingBottom: 24 },
  attach: { width: 46, height: 46, borderRadius: 23, backgroundColor: '#fff', alignItems: 'center', justifyContent: 'center', elevation: 1, shadowColor: '#000', shadowOpacity: 0.06, shadowRadius: 2 },
  inputPill: {
    flex: 1,
    minWidth: 0,
    minHeight: 46,
    backgroundColor: '#fff',
    borderRadius: 24,
    flexDirection: 'row',
    alignItems: 'center',
    paddingLeft: 16,
    paddingRight: 6,
    gap: 2,
    elevation: 1,
    shadowColor: '#000',
    shadowOpacity: 0.06,
    shadowRadius: 2,
  },
  input: { flex: 1, minWidth: 0, fontFamily: font.regular, fontSize: 15.5, color: color.ink, paddingVertical: 11, maxHeight: 120 },
  timerChip: { height: 34, borderRadius: 99, backgroundColor: color.chip, paddingHorizontal: 8, flexDirection: 'row', alignItems: 'center', gap: 3 },
  laterBtn: { width: 34, height: 34, borderRadius: 17, alignItems: 'center', justifyContent: 'center' },
  send: { width: 46, height: 46, borderRadius: 23, alignItems: 'center', justifyContent: 'center' },
  whenRow: { flexDirection: 'row', alignItems: 'center', gap: 6, marginHorizontal: 16, marginTop: 6 },
  blockCard: { backgroundColor: '#fff', borderWidth: 1.5, borderColor: '#F2B796', borderRadius: 24, padding: 16, gap: 12 },
  repair: { height: 48, borderRadius: 99, backgroundColor: color.ink, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 8 },
  blockedWhy: { flexDirection: 'row', gap: 10, backgroundColor: color.chip, borderRadius: 16, padding: 12, alignItems: 'flex-start' },
  attachTile: { flex: 1, borderRadius: 22, backgroundColor: color.chip, paddingVertical: 18, paddingHorizontal: 14, gap: 10 },
  laterOpt: { height: 52, borderRadius: 16, backgroundColor: color.chip, paddingHorizontal: 16, flexDirection: 'row', alignItems: 'center', gap: 10 },
  selPreview: { backgroundColor: color.chip, borderRadius: 16, paddingVertical: 12, paddingHorizontal: 14 },
});

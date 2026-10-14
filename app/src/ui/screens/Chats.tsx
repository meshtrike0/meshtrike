import React, { useEffect, useState } from 'react';
import { AppState, Linking, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useReachFirstOffer } from '../Reach';
import { meshCoreNative } from '../../../modules/mesh-core/src/MeshCoreModule';
import { Contact, ConversationSummary, Warning } from '../../core/types';
import { Icon } from '../icons';
import { Appear, Avatar, Banner, Empty, Eyebrow, HeaderButton, IconTile, LiveDot, Screen, Tag, TrustBadge, TrustGlyph } from '../kit';
import { timeAgo, useApp } from '../state';
import { color, font, type } from '../theme';

/** The phone's Bluetooth radio is switched off (Android; read the same way Network does). */
function readBluetoothOff(): boolean {
  if (Platform.OS !== 'android') return false;
  try {
    return !meshCoreNative().bluetoothEnabled();
  } catch {
    return false;
  }
}

/** Re-checked when the app comes back to the front and every few seconds while open. */
function useBluetoothOff(): boolean {
  const [off, setOff] = useState(readBluetoothOff);
  useEffect(() => {
    if (Platform.OS !== 'android') return;
    const check = () => setOff(readBluetoothOff());
    const sub = AppState.addEventListener('change', (st) => st === 'active' && check());
    const t = setInterval(check, 3000);
    return () => {
      sub.remove();
      clearInterval(t);
    };
  }, []);
  return off;
}

/** Ask Android to switch Bluetooth on; fall back to its Bluetooth settings page. */
function turnOnBluetooth() {
  if (Platform.OS !== 'android') return;
  Linking.sendIntent('android.bluetooth.adapter.action.REQUEST_ENABLE')
    .catch(() => Linking.sendIntent('android.settings.BLUETOOTH_SETTINGS'))
    .catch(() => {});
}

/** What the phone can reach right now, in plain words (taps through to Network). */
function ReachChip({ btOff }: { btOff: boolean }) {
  const { network, tab } = useApp();
  let label = 'Starting radio…';
  let dot: string = color.disabled;
  if (btOff) {
    label = network?.internet_online ? 'Bluetooth off · internet route only' : 'Bluetooth off · no reach';
    dot = color.dangerStrong;
  } else if (network) {
    const net = network.internet_online ? ' · internet route' : '';
    const phones = `${network.neighbours} ${network.neighbours === 1 ? 'phone' : 'phones'}`;
    if (network.neighbours === 0) {
      label = network.internet_online ? 'Nobody nearby · internet route on' : 'Nobody nearby · messages wait safely';
      dot = network.internet_online ? color.goodDot : '#9A968C';
    } else if (network.high_risk) {
      label = `Nearby only · ${phones}`;
      dot = color.goodDot;
    } else {
      label = `Nearby · ${phones}${network.lora_paired ? ' · LoRa ready' : ''}${net}`;
      dot = color.goodDot;
    }
  }
  return (
    <Pressable onPress={() => tab('Network')} accessibilityRole="button" accessibilityLabel={`Reach: ${label}`} style={[st.chip, st.chipWhite]}>
      <LiveDot tint={dot} />
      <Text style={st.chipText}>{label}</Text>
    </Pressable>
  );
}

function ModeChips() {
  const { network, settings, tab } = useApp();
  if (!network || !settings) return null;
  const mode = network.radio_mode === 'ListenOnly' ? 'Listen-only' : network.radio_mode;
  return (
    <>
      {settings.high_risk ? (
        <Appear kind="pop">
          <Pressable onPress={() => tab('Settings')} accessibilityRole="button" accessibilityLabel="High-Risk" style={[st.chip, { backgroundColor: color.ink, paddingLeft: 9 }]}>
            <Icon name="zap" size={14} tint={color.lime} />
            <Text style={[st.chipText, { fontFamily: font.semi, color: color.lime }]}>High-Risk</Text>
          </Pressable>
        </Appear>
      ) : null}
      {network.radio_mode !== 'Public' ? (
        <Appear kind="pop">
          <Pressable
            onPress={() => tab('Settings')}
            accessibilityRole="button"
            accessibilityLabel={`Radio mode: ${mode}`}
            style={[st.chip, { borderWidth: 1.5, borderColor: color.ink, paddingLeft: 9 }]}
          >
            <Icon name="ear" size={14} />
            <Text style={[st.chipText, { fontFamily: font.semi }]}>{mode}</Text>
          </Pressable>
        </Appear>
      ) : null}
    </>
  );
}

function warningText(w: Warning): { title: string; text: string; danger: boolean; sticky: boolean } {
  if (w === 'ClockSkew') return { title: 'Clock looks wrong', text: 'Nearby phones disagree with your clock. Check the date and time setting.', danger: false, sticky: false };
  if (w === 'StorageFull') return { title: 'Storage is full', text: 'Old messages carried for others are being dropped.', danger: false, sticky: false };
  if ('KeyChanged' in w) return { title: 'A contact’s key changed', text: 'Sending to them is paused until you meet and pair again.', danger: true, sticky: false };
  // Device-integrity warnings stay visible (section 12.3).
  return { title: 'Device integrity can’t be verified', text: w.DeviceIntegrity, danger: false, sticky: true };
}

function ConvRow({ c, contact, index, onPress }: { c: ConversationSummary; contact?: Contact; index: number; onPress: () => void }) {
  const unread = c.unread > 0;
  const changed = c.kind === 'Direct' && c.trust === 'KeyChanged';
  const title = c.kind === 'Channel' ? `#${c.title}` : c.title;
  const kind = c.kind === 'Group' ? 'group' : c.kind === 'Channel' ? 'channel' : 'person';
  const last = c.last_preview || (c.kind === 'Channel' ? 'Public channel' : 'No messages yet');
  return (
    <Appear kind="up" delay={Math.min(index, 8) * 40}>
      <Pressable
        onPress={onPress}
        accessibilityRole="button"
        accessibilityLabel={`${title}, ${c.unread} unread`}
        style={({ pressed }) => [st.row, pressed ? { transform: [{ scale: 0.985 }] } : null]}
      >
        <View>
          <Avatar seed={contact?.identity_key ?? c.id} size={52} kind={kind} />
          {contact?.nearby ? <View accessibilityLabel="Nearby" style={st.nearbyDot} /> : null}
        </View>
        <View style={{ flex: 1, minWidth: 0, gap: 4 }}>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
            <Text style={st.rowTitle} numberOfLines={1}>
              {title}
            </Text>
            {c.kind === 'Direct' && c.trust ? <TrustGlyph trust={c.trust} /> : null}
            {c.kind === 'Channel' ? <Tag text="PUBLIC" /> : null}
            {c.kind === 'Group' ? <Tag text="GROUP" tone="soft" /> : null}
            <View style={{ flex: 1 }} />
            <Text style={st.time}>{timeAgo(c.last_activity)}</Text>
          </View>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
            {changed ? <Icon name="alert" size={14} tint={color.danger} /> : null}
            <Text
              numberOfLines={1}
              style={{
                flex: 1,
                minWidth: 0,
                fontSize: 14,
                lineHeight: 18,
                fontFamily: unread ? font.semi : font.regular,
                color: unread ? color.ink : changed ? color.danger : color.textMuted,
              }}
            >
              {changed && !c.last_preview ? 'Key changed · sending paused' : last}
            </Text>
            {unread ? (
              <View style={st.unread}>
                <Text style={{ fontFamily: font.bold, fontSize: 12, color: color.lime }}>{c.unread}</Text>
              </View>
            ) : null}
          </View>
        </View>
      </Pressable>
    </Appear>
  );
}

export function Chats() {
  const { convs, contacts, push, interference, warnings, dismissWarning, network } = useApp();
  const btOff = useBluetoothOff();
  // First visit with the add-on missing: offer it once.
  useReachFirstOffer(true);
  const byConv = new Map(contacts.filter((x) => x.conversation).map((x) => [x.conversation!, x]));
  return (
    <Appear kind="in" style={st.screen}>
      {/* Title, reach and mode chips stay put; only the list scrolls. */}
      <View style={st.head}>
        <View style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' }}>
          <Text style={type.hero} accessibilityRole="header">
            Chats
          </Text>
          <View style={st.newChat}>
            <HeaderButton icon="pen" label="New chat" tone="lime" onPress={() => push({ name: 'NewChat' })} />
          </View>
        </View>
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
          <ReachChip btOff={btOff} />
          <ModeChips />
        </View>
      </View>
      <ScrollView style={{ flex: 1 }} contentContainerStyle={st.list} keyboardShouldPersistTaps="handled" showsVerticalScrollIndicator={false}>
        {btOff ? (
          <View style={st.card}>
            <Banner
              tone="dark"
              icon="bt"
              title="Bluetooth is off"
              text="Nothing can send or arrive until it’s back on."
              action={
                Platform.OS === 'android' ? (
                  <Pressable onPress={turnOnBluetooth} accessibilityRole="button" accessibilityLabel="Turn on Bluetooth" style={st.limeBtn}>
                    <Text style={{ fontFamily: font.semi, fontSize: 13, color: color.ink }}>Turn on</Text>
                  </Pressable>
                ) : undefined
              }
            />
          </View>
        ) : null}
        {interference ? (
          <View style={st.card}>
            <Banner
              tone="warning"
              icon="radio"
              title="Radio interference likely"
              text="Bluetooth went silent while you’re still."
              action={
                <Pressable onPress={() => push({ name: 'Handoff' })} accessibilityRole="button" accessibilityLabel="QR handoff" style={st.inkBtn}>
                  <Icon name="qr" size={14} tint="#fff" />
                  <Text style={{ fontFamily: font.semi, fontSize: 12.5, color: '#fff' }}>Handoff</Text>
                </Pressable>
              }
            />
          </View>
        ) : null}
        {warnings.map((w, i) => {
          const t = warningText(w);
          return (
            <View key={i} style={st.card}>
              <Banner
                tone={t.danger ? 'danger' : 'warning'}
                icon={t.danger ? 'shieldX' : 'cpu'}
                title={t.title}
                text={t.text}
                action={
                  t.sticky ? undefined : (
                    <Pressable onPress={() => dismissWarning(w)} accessibilityRole="button" accessibilityLabel="Dismiss" hitSlop={8}>
                      <Icon name="x" size={16} tint={color.textMuted} />
                    </Pressable>
                  )
                }
              />
            </View>
          );
        })}
        {convs.length === 0 ? (
          <Empty icon="chat" title="No chats yet" text="Add a contact in person by QR code, or join a public channel to meet people nearby." />
        ) : (
          convs.map((c, i) => <ConvRow key={c.id} c={c} contact={byConv.get(c.id)} index={i} onPress={() => push({ name: 'Chat', conv: c.id })} />)
        )}
        <Text style={st.footer}>
          {network?.internet_on
            ? 'Messages are end-to-end sealed. They travel between phones, and through the mixnet to contacts far away.'
            : 'Messages are end-to-end sealed and travel only between phones.'}
        </Text>
      </ScrollView>
    </Appear>
  );
}

export function NewChat() {
  const { contacts, push, pop } = useApp();
  return (
    <Screen title="New chat" onBack={pop}>
      <Appear kind="up">
        <Pressable
          onPress={() => push({ name: 'Pair' })}
          accessibilityRole="button"
          accessibilityLabel="Add contact in person, scan each other’s code"
          style={({ pressed }) => [st.hero, pressed ? { transform: [{ scale: 0.985 }] } : null]}
        >
          <View style={st.heroRing} />
          <IconTile icon="qr" bg={color.lime} size={46} />
          <View style={{ gap: 4 }}>
            <Text style={{ fontFamily: font.display, fontSize: 20, color: '#fff' }}>Add contact in person</Text>
            <Text style={{ fontFamily: font.regular, fontSize: 13.5, lineHeight: 19, color: color.onInkMuted }}>
              Scan each other’s code. Contacts added this way are Verified.
            </Text>
          </View>
        </Pressable>
      </Appear>
      <View style={{ flexDirection: 'row', gap: 10 }}>
        <Pressable onPress={() => push({ name: 'NewGroup' })} accessibilityRole="button" accessibilityLabel="New group, up to 50 members" style={st.tile}>
          <IconTile icon="users" bg={color.purpleBg} fg={color.purple} />
          <Text style={st.tileText}>New group</Text>
        </Pressable>
        <Pressable onPress={() => push({ name: 'Channels' })} accessibilityRole="button" accessibilityLabel="Public channels, anyone nearby can read" style={st.tile}>
          <IconTile icon="hash" bg={color.ink} fg={color.lime} />
          <Text style={st.tileText}>Public channels</Text>
        </Pressable>
      </View>
      <View style={{ gap: 8 }}>
        <Eyebrow>Contacts</Eyebrow>
        {contacts.length === 0 ? (
          <Text style={[st.footer, { textAlign: 'left', paddingHorizontal: 6 }]}>No contacts yet. Pair in person to add one.</Text>
        ) : null}
        {contacts.map((c, i) => (
          <Appear key={c.id} kind="up" delay={Math.min(i, 8) * 40}>
            <Pressable
              onPress={() => (c.conversation ? push({ name: 'Chat', conv: c.conversation }) : push({ name: 'ContactDetail', contact: c.id }))}
              accessibilityRole="button"
              accessibilityLabel={`${c.nickname}${c.nearby ? ', nearby' : ''}`}
              style={st.contact}
            >
              <Avatar seed={c.identity_key} size={44} />
              <View style={{ flex: 1, gap: 5, minWidth: 0 }}>
                <Text style={{ fontFamily: font.semi, fontSize: 15, color: color.ink }} numberOfLines={1}>
                  {c.nickname}
                </Text>
                <TrustBadge trust={c.trust} compact />
              </View>
              {c.nearby ? (
                <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
                  <LiveDot />
                  <Text style={{ fontFamily: font.semi, fontSize: 12, color: color.good }}>Nearby</Text>
                </View>
              ) : null}
            </Pressable>
          </Appear>
        ))}
      </View>
    </Screen>
  );
}

const st = StyleSheet.create({
  screen: { flex: 1, backgroundColor: color.bg },
  head: { paddingTop: 6, paddingHorizontal: 20, paddingBottom: 12, gap: 12 },
  // Design: 0 6px 16px -8px rgba(80,110,0,.6) under the lime New chat button.
  newChat: { borderRadius: 17, boxShadow: '0 6px 16px -8px rgba(80,110,0,0.6)' },
  // Room under the last chat for the floating tab bar.
  list: { paddingHorizontal: 12, paddingBottom: 120, gap: 8 },
  card: { marginHorizontal: 4, marginBottom: 4 },
  limeBtn: { height: 36, borderRadius: 99, backgroundColor: color.lime, paddingHorizontal: 14, alignItems: 'center', justifyContent: 'center' },
  chip: { height: 32, borderRadius: 99, paddingLeft: 10, paddingRight: 12, flexDirection: 'row', alignItems: 'center', gap: 8 },
  chipWhite: { backgroundColor: '#fff', shadowColor: '#000', shadowOpacity: 0.06, shadowRadius: 2, shadowOffset: { width: 0, height: 1 }, elevation: 1 },
  chipText: { fontFamily: font.medium, fontSize: 12.5, color: color.ink },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 13,
    paddingVertical: 11,
    paddingHorizontal: 12,
    borderRadius: 22,
    backgroundColor: '#fff',
    shadowColor: '#000',
    shadowOpacity: 0.04,
    shadowRadius: 2,
    shadowOffset: { width: 0, height: 1 },
    elevation: 1,
  },
  rowTitle: { fontFamily: font.semi, fontSize: 16, lineHeight: 19, color: color.ink, flexShrink: 1 },
  time: { fontFamily: font.medium, fontSize: 12, color: color.textFaint },
  nearbyDot: { position: 'absolute', right: -2, bottom: -2, width: 15, height: 15, borderRadius: 8, backgroundColor: color.goodDot, borderWidth: 3, borderColor: '#fff' },
  unread: { minWidth: 22, height: 22, paddingHorizontal: 7, borderRadius: 99, backgroundColor: color.ink, alignItems: 'center', justifyContent: 'center' },
  footer: { paddingVertical: 14, paddingHorizontal: 18, fontFamily: font.regular, fontSize: 12.5, lineHeight: 18, color: color.textFaint, textAlign: 'center' },
  inkBtn: { height: 36, borderRadius: 99, backgroundColor: color.ink, paddingHorizontal: 12, flexDirection: 'row', alignItems: 'center', gap: 6 },
  hero: { borderRadius: 26, backgroundColor: color.ink, padding: 20, gap: 14, overflow: 'hidden' },
  heroRing: { position: 'absolute', right: -30, top: -30, width: 140, height: 140, borderRadius: 70, borderWidth: 1.5, borderStyle: 'dashed', borderColor: 'rgba(205,242,92,0.35)' },
  tile: { flex: 1, borderRadius: 22, backgroundColor: '#fff', padding: 16, gap: 12 },
  tileText: { fontFamily: font.semi, fontSize: 15, color: color.ink },
  contact: { flexDirection: 'row', alignItems: 'center', gap: 12, backgroundColor: '#fff', borderRadius: 20, paddingVertical: 10, paddingHorizontal: 12 },
});

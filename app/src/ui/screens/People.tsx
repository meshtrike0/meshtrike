import React, { useEffect, useState } from 'react';
import { Pressable, StyleSheet, Text, TextInput, View } from 'react-native';
import { GroupMember, Hex } from '../../core/types';
import { Icon } from '../icons';
import { Appear, Avatar, Button, ErrorText, Eyebrow, Field, HoldButton, IconTile, Screen, Tag, TrustBadge } from '../kit';
import { errorText, useApp, useCoreEvents } from '../state';
import { CARRIER, color, font, LINK_CARRIER, type } from '../theme';

function Hero({ seed, kind, name, children }: { seed: string; kind?: 'person' | 'group' | 'channel'; name: string; children?: React.ReactNode }) {
  return (
    <Appear kind="up" style={{ alignItems: 'center', gap: 12, paddingTop: 4, paddingBottom: 6 }}>
      <Avatar seed={seed} size={96} kind={kind} />
      <Text style={{ fontFamily: font.display, fontSize: 28, lineHeight: 31, letterSpacing: -0.5, color: color.ink, textAlign: 'center' }} accessibilityRole="header">
        {name}
      </Text>
      {children}
    </Appear>
  );
}

export function ContactDetail({ contact }: { contact: Hex }) {
  const { contacts, call, pop, push, reset, notify, network } = useApp();
  const c = contacts.find((x) => x.id === contact);
  const [safety, setSafety] = useState<string | null>(null);
  const [safetyError, setSafetyError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setSafetyError(null);
    call<string>('safety_number', { contact })
      .then(setSafety)
      .catch((e) => {
        // Never verify against placeholders: show why, and keep "Mark verified" off.
        setSafety(null);
        setSafetyError(errorText(e));
      });
  }, [call, contact, c?.trust]);
  if (!c) {
    return (
      <Screen onBack={pop}>
        <Text style={type.body}>This contact was removed.</Text>
      </Screen>
    );
  }
  const groups = safety ? safety.split(' ').filter(Boolean) : [];
  return (
    <Screen onBack={pop}>
      <Hero seed={c.identity_key} name={c.nickname}>
        <TrustBadge trust={c.trust} large />
        <Text style={[type.small, { textAlign: 'center' }]}>
          {c.link ? `Connected now · ${CARRIER[LINK_CARRIER[c.link]].label}` : c.internet ? 'Far away · reachable over the internet route' : c.nearby ? 'In range now' : 'Not in range right now'}
        </Text>
      </Hero>
      {c.trust === 'KeyChanged' ? <KeyChangedNotice /> : null}
      <View style={st.card}>
        <View style={{ flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center' }}>
          <Text style={{ fontFamily: font.semi, fontSize: 15, color: color.ink }}>Safety number</Text>
          <Text style={{ fontFamily: font.medium, fontSize: 12, color: color.textMuted }}>Read it aloud together</Text>
        </View>
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }} accessibilityLabel={safety ? `Safety number ${safety}` : safetyError ? 'Safety number unavailable' : 'Loading safety number'}>
          {(groups.length ? groups : ['·····', '·····', '·····', '·····', '·····', '·····']).map((g, i) => (
            <View key={i} style={st.snChip}>
              <Text style={{ fontFamily: font.monoSemi, fontSize: 18, letterSpacing: 1, color: color.ink }}>{g}</Text>
            </View>
          ))}
        </View>
        <Text style={[type.small, { fontSize: 12.5 }]}>If both phones show the same 30 digits, nobody is sitting between you.</Text>
        {safetyError ? <ErrorText error={`Safety number unavailable: ${safetyError}`} /> : null}
        {c.trust === 'Unverified' ? (
          <Button
            title="Numbers match · Mark verified"
            label="Numbers match: mark verified"
            icon="shieldOk"
            size="md"
            style={st.btn}
            disabled={!safety}
            onPress={async () => {
              try {
                await call('mark_verified', { contact });
                notify(`${c.nickname} marked verified`);
              } catch (e) {
                setError(errorText(e));
              }
            }}
          />
        ) : null}
        {c.trust === 'KeyChanged' ? <Button title="Re-pair in person" size="md" style={st.btn} onPress={() => push({ name: 'Pair' })} /> : null}
      </View>
      {c.conversation ? <Button title="Open chat" variant="secondary" size="md" icon="chat" style={st.btn} onPress={() => push({ name: 'Chat', conv: c.conversation! })} /> : null}
      {network?.bridges ? (
        <Button
          title="Share my bridges"
          variant="secondary"
          size="md"
          icon="shieldCheck"
          style={st.btn}
          onPress={async () => {
            try {
              const n = await call<number>('share_bridges', { contact: c.id });
              // The recipient decides: bridges are only taken from contacts they verified in person.
              notify(`${n} bridge${n === 1 ? '' : 's'} sent · used if ${c.nickname} verified you in person`);
            } catch (e) {
              setError(errorText(e));
            }
          }}
        />
      ) : null}
      <ErrorText error={error} />
      <View style={{ gap: 8 }}>
        <HoldButton
          title="Hold to remove contact"
          ms={2000}
          tone="outline"
          onDone={async () => {
            try {
              await call('remove_contact', { contact });
              reset({ name: 'Chats' });
              notify('Contact and chat removed');
            } catch (e) {
              setError(errorText(e));
            }
          }}
        />
        {/* Second sentence is real: remove_contact rotates our presence key. */}
        <Text style={st.caption}>Removes the contact and deletes your chat. They can no longer tell when you’re nearby. This can’t be undone.</Text>
      </View>
    </Screen>
  );
}

/** Key-changed alert as in the design: 13.5 text, icon top-aligned, gap 10. */
function KeyChangedNotice() {
  return (
    <Appear kind="up">
      <View accessibilityRole="alert" style={st.keyChanged}>
        <View style={{ paddingTop: 1 }}>
          <Icon name="alert" size={18} tint={color.dangerDeep} />
        </View>
        <Text style={{ flex: 1, fontFamily: font.medium, fontSize: 13.5, lineHeight: 19, color: color.dangerDeep }}>
          The safety number below is new. Don’t trust it until you’ve met and paired again.
        </Text>
      </View>
    </Appear>
  );
}

export function NewGroup() {
  const { contacts, call, pop, reset, push } = useApp();
  const [name, setName] = useState('');
  const [picked, setPicked] = useState<Hex[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const toggle = (id: Hex) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : p.length >= 49 ? p : [...p, id]));
  const title = picked.length === 0 ? 'Pick at least one person' : `Create group · ${picked.length + 1} people`;
  return (
    <Screen
      title="New group"
      onBack={pop}
      footer={
        <View style={{ paddingHorizontal: 16, paddingTop: 10, paddingBottom: 30 }}>
          <Button
            title={title}
            label="Create group"
            busy={busy}
            disabled={picked.length === 0}
            onPress={async () => {
              setBusy(true);
              try {
                const conv = await call<Hex>('create_group', { name: name.trim() || 'New group', members: picked });
                reset({ name: 'Chats' });
                push({ name: 'Chat', conv });
              } catch (e) {
                setError(errorText(e));
              } finally {
                setBusy(false);
              }
            }}
          />
        </View>
      }
    >
      <Field label="Group name" hideLabel placeholder="Group name" value={name} onChangeText={setName} maxLength={64} style={{ height: 56, borderRadius: 18, fontFamily: font.semi, fontSize: 17 }} />
      <Eyebrow>Pick members · up to 50</Eyebrow>
      {contacts.length === 0 ? <Text style={[st.caption, { textAlign: 'left' }]}>No contacts yet. Only contacts can be added.</Text> : null}
      {contacts.map((c, i) => {
        const on = picked.includes(c.id);
        return (
          <Appear key={c.id} kind="up" delay={Math.min(i, 8) * 35}>
            <Pressable onPress={() => toggle(c.id)} accessibilityRole="button" accessibilityLabel={`${c.nickname}, ${on ? 'selected' : 'not selected'}`} style={st.pickRow}>
              <Avatar seed={c.identity_key} size={44} />
              <View style={{ flex: 1, gap: 5, minWidth: 0 }}>
                <Text style={st.name} numberOfLines={1}>
                  {c.nickname}
                </Text>
                <TrustBadge trust={c.trust} compact />
              </View>
              <View style={[st.box, on ? { backgroundColor: color.ink, borderColor: color.ink } : null]}>{on ? <Icon name="check" size={16} tint={color.lime} /> : null}</View>
            </Pressable>
          </Appear>
        );
      })}
      <ErrorText error={error} />
    </Screen>
  );
}

export function GroupDetail({ conv }: { conv: Hex }) {
  const { call, pop, reset, convs, contacts, notify } = useApp();
  const summary = convs.find((c) => c.id === conv);
  const [members, setMembers] = useState<GroupMember[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const load = () => call<GroupMember[]>('group_members', { conversation: conv }).then(setMembers).catch((e) => setError(errorText(e)));
  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conv]);
  useCoreEvents((e) => e.event === 'ConversationsChanged' && load(), [conv]);
  const admin = !!summary?.is_admin;
  const memberKeys = new Set(members.map((m) => m.identity_key));
  const candidates = contacts.filter((c) => !memberKeys.has(c.identity_key));
  return (
    <Screen onBack={pop}>
      <Hero seed={conv} kind="group" name={summary?.title ?? 'Group'}>
        <Text style={{ fontFamily: font.medium, fontSize: 13.5, color: color.textMuted }}>
          {members.length} {members.length === 1 ? 'member' : 'members'}
          {admin ? ' · you are admin' : ''}
        </Text>
      </Hero>
      <View style={[st.card, { paddingVertical: 8, paddingHorizontal: 14, gap: 0 }]}>
        {admin ? (
          <Pressable onPress={() => setAdding((a) => !a)} accessibilityRole="button" accessibilityLabel="Add members" style={st.memberRow}>
            <IconTile icon={adding ? 'x' : 'userPlus'} bg={color.lime} size={40} />
            <Text style={st.name}>{adding ? 'Done adding' : 'Add members'}</Text>
          </Pressable>
        ) : null}
        {adding
          ? candidates.map((c) => (
              <Appear key={c.id} kind="up">
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={`Add ${c.nickname}`}
                  style={[st.memberRow, st.line]}
                  onPress={async () => {
                    try {
                      await call('add_members', { conversation: conv, members: [c.id] });
                      notify(`${c.nickname} added`);
                      load();
                    } catch (e) {
                      setError(errorText(e));
                    }
                  }}
                >
                  <Avatar seed={c.identity_key} size={40} />
                  <Text style={[st.name, { flex: 1 }]}>{c.nickname}</Text>
                  <Icon name="plus" size={20} />
                </Pressable>
              </Appear>
            ))
          : null}
        {adding && candidates.length === 0 ? <Text style={[st.caption, st.line, { paddingVertical: 12 }]}>All your contacts are already in this group.</Text> : null}
        {members.map((m, i) => (
          <View key={m.identity_key} style={[st.memberRow, admin || i > 0 ? st.line : null]}>
            <Avatar seed={m.identity_key} size={40} />
            <View style={{ flex: 1, minWidth: 0, gap: 4 }}>
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
                <Text style={st.name} numberOfLines={1}>
                  {m.is_me ? 'You' : m.nickname}
                </Text>
                {m.is_admin ? <Tag text="ADMIN" /> : null}
              </View>
              {!m.is_me ? <TrustBadge trust={m.trust} compact /> : null}
              {!m.is_me && !m.contact ? (
                <View style={st.note}>
                  <Text style={{ fontFamily: font.medium, fontSize: 11, color: color.grey }}>Not in your contacts</Text>
                </View>
              ) : null}
            </View>
            {admin && !m.is_me && m.member ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`Remove ${m.nickname}`}
                hitSlop={8}
                style={{ width: 36, height: 36, borderRadius: 12, backgroundColor: color.chip, alignItems: 'center', justifyContent: 'center' }}
                onPress={async () => {
                  try {
                    await call('remove_member', { conversation: conv, member: m.member });
                    notify(`${m.nickname} removed from group`);
                    load();
                  } catch (e) {
                    setError(errorText(e));
                  }
                }}
              >
                <Icon name="x" tint={color.danger} size={17} />
              </Pressable>
            ) : null}
          </View>
        ))}
      </View>
      <ErrorText error={error} />
      {/* Hold to confirm: a single stray tap must not leave the group. */}
      <HoldButton
        title="Hold to leave group"
        ms={2000}
        tone="outline"
        icon="logout"
        onDone={async () => {
          try {
            await call('leave_group', { conversation: conv });
            reset({ name: 'Chats' });
            notify('You left the group');
          } catch (e) {
            setError(errorText(e));
          }
        }}
      />
    </Screen>
  );
}

export function Channels() {
  const { convs, call, pop, push, notify } = useApp();
  const [name, setName] = useState('');
  const [focus, setFocus] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Long-press reveals Leave on that row only, so a stray press never leaves.
  const [leaving, setLeaving] = useState<Hex | null>(null);
  const channels = convs.filter((c) => c.kind === 'Channel');
  const join = async () => {
    if (!name.trim()) return;
    try {
      const conv = await call<Hex>('join_channel', { name });
      setName('');
      push({ name: 'Chat', conv });
    } catch (e) {
      setError(errorText(e));
    }
  };
  const leave = (id: Hex, title: string) => {
    setLeaving(null);
    call('leave_channel', { conversation: id }).then(
      () => notify(`Left #${title}`),
      (e) => notify(errorText(e)),
    );
  };
  return (
    <Screen title="Public channels" onBack={pop}>
      <View style={[st.joinRow, focus ? { borderColor: color.ink } : null]}>
        <Text style={{ fontFamily: font.monoSemi, fontSize: 20, color: color.textFaint }}>#</Text>
        <TextInput
          accessibilityLabel="Channel name"
          placeholder="plaza"
          placeholderTextColor={color.textFaint}
          value={name}
          onChangeText={setName}
          maxLength={64}
          autoCapitalize="none"
          autoCorrect={false}
          importantForAutofill="no"
          onFocus={() => setFocus(true)}
          onBlur={() => setFocus(false)}
          onSubmitEditing={join}
          returnKeyType="go"
          style={{ flex: 1, minWidth: 0, height: 44, fontFamily: font.semi, fontSize: 17, color: color.ink }}
        />
        <Pressable
          onPress={join}
          disabled={!name.trim()}
          accessibilityRole="button"
          accessibilityLabel="Join"
          accessibilityState={{ disabled: !name.trim() }}
          style={({ pressed }) => [st.joinBtn, !name.trim() ? { opacity: 0.45 } : null, pressed ? { transform: [{ scale: 0.98 }] } : null]}
        >
          <Text style={{ fontFamily: font.semi, fontSize: 14, color: '#fff' }}>Join</Text>
        </Pressable>
      </View>
      <View accessibilityRole="alert" style={st.warn}>
        <Icon name="eye" size={17} tint={color.warnInk} />
        <Text style={{ flex: 1, fontFamily: font.medium, fontSize: 13, lineHeight: 18, color: color.warnInk }}>
          Anyone who knows a channel’s name can read it. You get a different random name in each channel.
        </Text>
      </View>
      <ErrorText error={error} />
      <Eyebrow>Joined</Eyebrow>
      {channels.length === 0 ? <Text style={[st.caption, { textAlign: 'left' }]}>None yet. Try a place name everyone around you would guess.</Text> : null}
      {channels.map((c, i) => {
        const open = leaving === c.id;
        return (
          <Appear key={c.id} kind="up" delay={Math.min(i, 8) * 35}>
            <Pressable
              onPress={() => (leaving ? setLeaving(null) : push({ name: 'Chat', conv: c.id }))}
              onLongPress={() => setLeaving(open ? null : c.id)}
              accessibilityRole="button"
              accessibilityLabel={`#${c.title}`}
              accessibilityHint="Long-press to leave"
              accessibilityActions={[{ name: 'leave', label: `Leave #${c.title}` }]}
              onAccessibilityAction={(e) => e.nativeEvent.actionName === 'leave' && leave(c.id, c.title)}
              style={st.chanRow}
            >
              <Avatar seed={c.id} size={40} kind="channel" />
              <Text style={[st.name, { flex: 1 }]} numberOfLines={1}>
                #{c.title}
              </Text>
              {open ? (
                <Pressable accessibilityRole="button" accessibilityLabel={`Leave #${c.title}`} onPress={() => leave(c.id, c.title)} hitSlop={6} style={st.leave}>
                  <Text style={{ fontFamily: font.semi, fontSize: 12, color: color.danger }}>Leave</Text>
                </Pressable>
              ) : (
                <Icon name="right" size={18} tint={color.textFaint} />
              )}
            </Pressable>
          </Appear>
        );
      })}
      {channels.length ? <Text style={st.caption}>Long-press a channel to leave it.</Text> : null}
    </Screen>
  );
}

const st = StyleSheet.create({
  card: { backgroundColor: '#fff', borderRadius: 24, padding: 18, gap: 14 },
  snChip: { width: '31%', flexGrow: 1, height: 46, borderRadius: 14, backgroundColor: color.chip, alignItems: 'center', justifyContent: 'center' },
  caption: { fontFamily: font.regular, fontSize: 12, lineHeight: 17, color: color.textFaint, textAlign: 'center', paddingHorizontal: 6 },
  name: { fontFamily: font.semi, fontSize: 15, color: color.ink, flexShrink: 1 },
  pickRow: { flexDirection: 'row', alignItems: 'center', gap: 12, backgroundColor: '#fff', borderRadius: 20, paddingVertical: 10, paddingLeft: 12, paddingRight: 14 },
  box: { width: 28, height: 28, borderRadius: 9, borderWidth: 2, borderColor: color.dash, backgroundColor: '#fff', alignItems: 'center', justifyContent: 'center' },
  memberRow: { flexDirection: 'row', alignItems: 'center', gap: 12, paddingVertical: 10 },
  line: { borderTopWidth: 1, borderTopColor: color.lineSoft },
  note: { alignSelf: 'flex-start', paddingHorizontal: 7, paddingVertical: 2, borderRadius: 99, borderWidth: 1.5, borderStyle: 'dashed', borderColor: color.disabled },
  joinRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
    backgroundColor: '#fff',
    borderRadius: 22,
    borderWidth: 1.5,
    borderColor: color.line,
    paddingVertical: 6,
    paddingRight: 6,
    paddingLeft: 16,
  },
  chanRow: { flexDirection: 'row', alignItems: 'center', gap: 12, backgroundColor: '#fff', borderRadius: 20, paddingVertical: 10, paddingHorizontal: 12 },
  joinBtn: { height: 44, paddingHorizontal: 18, borderRadius: 99, backgroundColor: color.ink, alignItems: 'center', justifyContent: 'center' },
  warn: { flexDirection: 'row', alignItems: 'flex-start', gap: 10, backgroundColor: color.warnBg, borderRadius: 18, paddingVertical: 12, paddingHorizontal: 14 },
  keyChanged: { flexDirection: 'row', alignItems: 'flex-start', gap: 10, backgroundColor: color.dangerBg, borderRadius: 20, padding: 14 },
  btn: { height: 50 },
  leave: { paddingHorizontal: 10, paddingVertical: 8, borderRadius: 99, backgroundColor: color.dangerBg },
});

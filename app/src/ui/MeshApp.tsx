import React, { useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Animated, Platform, Pressable, StyleSheet, Text, View } from 'react-native';
import { CoreClient } from '../core/client';
import { Icon, IconName } from './icons';
import { Button, EASE, IconTile, native } from './kit';
import { CallScreen } from './CallScreen';
import { ReachSheet } from './Reach';
import { Chat } from './screens/Chat';
import { Chats, NewChat } from './screens/Chats';
import { Permissions, Unlock, Welcome } from './screens/Onboarding';
import { Pair } from './screens/Pair';
import { Channels, ContactDetail, GroupDetail, NewGroup } from './screens/People';
import { About, Communities, Handoff, HighRisk, InternetRoute, Network, Settings } from './screens/System';
import { AppProvider, errorText, useApp } from './state';
import { color, font } from './theme';

export function MeshApp({ client }: { client: CoreClient }) {
  return (
    <AppProvider client={client}>
      <Shell />
    </AppProvider>
  );
}

const TABS: { name: 'Chats' | 'Network' | 'Settings'; icon: IconName }[] = [
  { name: 'Chats', icon: 'chat' },
  { name: 'Network', icon: 'mesh' },
  { name: 'Settings', icon: 'sliders' },
];

function Shell() {
  const { phase, route, client } = useApp();
  let body: React.ReactNode;
  if (phase === 'loading') {
    body = (
      <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center', backgroundColor: color.bg }}>
        <ActivityIndicator color={color.ink} />
      </View>
    );
  } else if (phase === 'welcome') body = <Welcome />;
  else if (phase === 'locked') body = <Unlock />;
  else {
    switch (route.name) {
      case 'Chat':
        body = <Chat key={route.conv} conv={route.conv} />;
        break;
      case 'NewChat':
        body = <NewChat />;
        break;
      case 'Pair':
        body = <Pair tab={route.tab} />;
        break;
      case 'ContactDetail':
        body = <ContactDetail contact={route.contact} />;
        break;
      case 'NewGroup':
        body = <NewGroup />;
        break;
      case 'GroupDetail':
        body = <GroupDetail conv={route.conv} />;
        break;
      case 'Channels':
        body = <Channels />;
        break;
      case 'Network':
        body = <Network key="network" />;
        break;
      case 'Handoff':
        body = <Handoff />;
        break;
      case 'Settings':
        body = <Settings key="settings" />;
        break;
      case 'HighRisk':
        body = <HighRisk />;
        break;
      case 'Communities':
        body = <Communities />;
        break;
      case 'Internet':
        body = <InternetRoute />;
        break;
      case 'About':
        body = <About />;
        break;
      case 'Permissions':
        body = <Permissions />;
        break;
      default:
        body = <Chats key="chats" />;
    }
  }
  const isTab = phase === 'ready' && (route.name === 'Chats' || route.name === 'Network' || route.name === 'Settings');
  return (
    <View style={{ flex: 1, backgroundColor: color.bg, overflow: 'hidden' }}>
      {body}
      {isTab ? <TabBar /> : null}
      <MessageBanner />
      <Toast />
      {phase === 'ready' ? <PairingRequestDialog /> : null}
      {phase === 'ready' ? <ReachSheet /> : null}
      {phase === 'ready' && client.hooks.call ? <CallScreen /> : null}
    </View>
  );
}

function TabBar() {
  const { route, tab, convs } = useApp();
  const unread = convs.reduce((a, c) => a + (c.unread || 0), 0);
  // Design spec: 18 px side padding per tab. Narrow screens shrink it so the bar
  // always keeps a margin from the screen edges instead of clipping.
  const [width, setWidth] = useState(362);
  const pad = Math.max(10, Math.min(18, Math.floor((width - 12 - 8 - 3 * 28 - 151) / 6)));
  return (
    <View style={s.tabWrap} pointerEvents="box-none" onLayout={(e) => setWidth(e.nativeEvent.layout.width)}>
      <View style={s.tabBar} accessibilityRole="tablist">
        {TABS.map((t) => {
          const on = route.name === t.name;
          return (
            <Pressable
              key={t.name}
              onPress={() => tab(t.name)}
              accessibilityRole="tab"
              accessibilityState={{ selected: on }}
              accessibilityLabel={t.name}
              style={[s.tab, { paddingHorizontal: pad }, on ? { backgroundColor: color.ink } : null]}
            >
              <Icon name={t.icon} size={20} tint={on ? color.lime : color.grey} />
              <Text style={{ fontFamily: font.semi, fontSize: 14, color: on ? '#fff' : color.grey }}>{t.name}</Text>
              {t.name === 'Chats' && !on && unread > 0 ? (
                <View style={s.tabBadge}>
                  <Text style={{ fontFamily: font.bold, fontSize: 10, color: color.ink }}>{unread}</Text>
                </View>
              ) : null}
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}


function MessageBanner() {
  const { banner, dismissBanner, push, phase } = useApp();
  const v = useRef(new Animated.Value(0)).current;
  // Kept while it slides out, then removed, so its buttons never linger
  // invisible for screen readers.
  const [shown, setShown] = useState(banner);
  useEffect(() => {
    if (banner) setShown(banner);
    Animated.timing(v, { toValue: banner ? 1 : 0, duration: 450, easing: EASE, useNativeDriver: native }).start(({ finished }) => {
      if (finished && !banner) setShown(null);
    });
  }, [banner, v]);
  const b = banner ?? shown;
  if (!b) return null;
  const open = b.conv && phase === 'ready' ? () => (dismissBanner(), push({ name: 'Chat', conv: b.conv! })) : undefined;
  return (
    <Animated.View
      pointerEvents={banner ? 'auto' : 'none'}
      style={[s.banner, { opacity: v, transform: [{ translateY: v.interpolate({ inputRange: [0, 1], outputRange: [-160, 0] }) }] }]}
      accessibilityLiveRegion="polite"
    >
      <IconTile icon="chat" bg={color.lime} size={38} />
      <View style={{ flex: 1, gap: 2 }}>
        <Text style={{ fontFamily: font.semi, fontSize: 14.5, color: '#fff' }}>{b.title}</Text>
        <Text style={{ fontFamily: font.regular, fontSize: 12.5, color: color.onInkMuted }} numberOfLines={1}>
          {b.sub}
        </Text>
      </View>
      {open ? (
        <Pressable onPress={open} accessibilityRole="button" accessibilityLabel="Open" style={s.bannerOpen}>
          <Text style={{ fontFamily: font.semi, fontSize: 13, color: color.ink }}>Open</Text>
        </Pressable>
      ) : null}
      <Pressable onPress={dismissBanner} accessibilityRole="button" accessibilityLabel="Dismiss" style={{ width: 30, height: 30, alignItems: 'center', justifyContent: 'center' }}>
        <Icon name="x" size={16} tint="rgba(255,255,255,0.7)" />
      </Pressable>
    </Animated.View>
  );
}

function Toast() {
  const { notice } = useApp();
  const v = useRef(new Animated.Value(0)).current;
  // The last text stays rendered while it fades out, then goes, so screen
  // readers never find a stale, invisible toast.
  const [shown, setShown] = useState(notice);
  useEffect(() => {
    if (notice) setShown(notice);
    Animated.timing(v, { toValue: notice ? 1 : 0, duration: 300, easing: EASE, useNativeDriver: native }).start(({ finished }) => {
      if (finished && !notice) setShown(null);
    });
  }, [notice, v]);
  const text = notice ?? shown;
  if (!text) return null;
  return (
    <View style={s.toastWrap} pointerEvents="none">
      <Animated.View
        accessibilityLiveRegion="polite"
        accessibilityLabel={text}
        style={[s.toast, { opacity: v, transform: [{ translateY: v.interpolate({ inputRange: [0, 1], outputRange: [24, 0] }) }] }]}
      >
        <Text style={{ fontFamily: font.medium, fontSize: 13.5, lineHeight: 18, color: '#fff', textAlign: 'center' }}>{text}</Text>
      </Animated.View>
    </View>
  );
}

function PairingRequestDialog() {
  const { requests, dismissRequest, call, notify } = useApp();
  const r = requests[0];
  const v = useRef(new Animated.Value(0)).current;
  useEffect(() => {
    if (r) Animated.spring(v, { toValue: 1, friction: 7, tension: 120, useNativeDriver: native }).start();
    else v.setValue(0);
  }, [r, v]);
  if (!r) return null;
  const decline = () => {
    dismissRequest(r.pairing);
    call('decline_pairing', { pairing: r.pairing }).catch(() => {});
  };
  return (
    <View style={[StyleSheet.absoluteFill, { zIndex: 80, alignItems: 'center', justifyContent: 'center', padding: 24 }]}>
      <Pressable style={[StyleSheet.absoluteFill, { backgroundColor: color.scrim }]} onPress={decline} accessibilityLabel="Decline" />
      <Animated.View
        accessibilityViewIsModal
        accessibilityRole="alert"
        style={[s.dialog, { opacity: v, transform: [{ scale: v.interpolate({ inputRange: [0, 1], outputRange: [0.92, 1] }) }] }]}
      >
        <IconTile icon="hash" bg={color.ink} fg={color.lime} size={52} />
        <Text style={{ fontFamily: font.display, fontSize: 22, lineHeight: 26, color: color.ink }}>Someone in #{r.channel} wants to chat</Text>
        <Text style={{ fontFamily: font.regular, fontSize: 14, lineHeight: 20, color: color.textMuted }}>
          If you accept, you exchange keys. They’ll be added as Not verified until you meet in person and compare safety numbers.
        </Text>
        <View style={{ flexDirection: 'row', gap: 8, marginTop: 4 }}>
          <Button title="Decline" variant="secondary" size="md" style={s.dialogBtn} onPress={decline} />
          <Button
            title="Accept"
            size="md"
            style={s.dialogBtn}
            onPress={async () => {
              dismissRequest(r.pairing);
              try {
                await call('accept_pairing', { pairing: r.pairing });
                notify('Accepted · waiting for their reply');
              } catch (e) {
                notify(errorText(e));
              }
            }}
          />
        </View>
      </Animated.View>
    </View>
  );
}

const s = StyleSheet.create({
  tabWrap: { position: 'absolute', left: 14, right: 14, bottom: 24, alignItems: 'center', zIndex: 20 },
  tabBar: {
    flexDirection: 'row',
    gap: 4,
    padding: 6,
    backgroundColor: 'rgba(255,255,255,0.92)',
    borderRadius: 99,
    // Design: 0 12px 30px -12px rgba(30,28,20,.35), 0 0 0 1px rgba(0,0,0,.04)
    shadowColor: '#1E1C14',
    shadowOpacity: 0.35,
    shadowRadius: 15,
    shadowOffset: { width: 0, height: 12 },
    elevation: 10,
    borderWidth: 1,
    borderColor: 'rgba(0,0,0,0.04)',
    ...(Platform.OS === 'web' ? ({ backdropFilter: 'blur(12px)', boxShadow: '0 12px 30px -12px rgba(30,28,20,.35)' } as object) : null),
  },
  tab: { height: 50, borderRadius: 99, flexDirection: 'row', alignItems: 'center', gap: 8 },
  tabBadge: { position: 'absolute', top: 8, left: 30, minWidth: 16, height: 16, borderRadius: 99, backgroundColor: color.lime, alignItems: 'center', justifyContent: 'center', paddingHorizontal: 4 },
  banner: {
    position: 'absolute',
    left: 10,
    right: 10,
    // Shell sits inside the root SafeAreaView, so 0 is already below the status bar (design: 4 px under it).
    top: 4,
    zIndex: 60,
    backgroundColor: color.ink,
    borderRadius: 22,
    paddingVertical: 12,
    paddingLeft: 14,
    paddingRight: 12,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
    shadowColor: '#000',
    shadowOpacity: 0.35,
    shadowRadius: 24,
    shadowOffset: { width: 0, height: 12 },
    elevation: 14,
  },
  bannerOpen: { height: 36, borderRadius: 99, backgroundColor: '#fff', paddingHorizontal: 14, alignItems: 'center', justifyContent: 'center' },
  // Above the pairing dialog (80), as in the design.
  toastWrap: { position: 'absolute', left: 16, right: 16, bottom: 110, alignItems: 'center', zIndex: 95, elevation: 95 },
  toast: { backgroundColor: color.ink, borderRadius: 18, paddingVertical: 11, paddingHorizontal: 16, maxWidth: 330, boxShadow: '0 10px 24px -10px rgba(0,0,0,0.5)' },
  dialog: { backgroundColor: '#fff', borderRadius: 32, padding: 24, gap: 14, width: '100%', maxWidth: 380 },
  dialogBtn: { flex: 1, height: 50 },
});

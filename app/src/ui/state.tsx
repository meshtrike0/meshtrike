import React, { createContext, ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { CoreClient } from '../core/client';
import { Contact, ConversationSummary, CoreError, CoreEvent, Hex, IdentityInfo, NetworkStatus, Settings, Warning } from '../core/types';

export type Route =
  | { name: 'Welcome' }
  | { name: 'CreateIdentity' }
  | { name: 'Unlock' }
  | { name: 'Chats' }
  | { name: 'Chat'; conv: Hex }
  | { name: 'NewChat' }
  | { name: 'Pair'; tab?: 'show' | 'scan' }
  | { name: 'ContactDetail'; contact: Hex }
  | { name: 'NewGroup' }
  | { name: 'GroupDetail'; conv: Hex }
  | { name: 'Channels' }
  | { name: 'Network' }
  | { name: 'Handoff' }
  | { name: 'Settings' }
  | { name: 'HighRisk' }
  | { name: 'Communities' }
  | { name: 'Internet' }
  | { name: 'About' }
  | { name: 'Permissions' };

export type Phase = 'loading' | 'welcome' | 'locked' | 'ready';

export interface InAppBanner {
  title: string;
  sub: string;
  conv: Hex | null;
  id: number;
}

export interface PairingRequest {
  pairing: Hex;
  channel: string;
}

interface Ctx {
  client: CoreClient;
  phase: Phase;
  route: Route;
  push: (r: Route) => void;
  pop: () => void;
  reset: (r: Route) => void;
  /** Bottom tab bar: switch to a top-level screen. */
  tab: (name: 'Chats' | 'Network' | 'Settings') => void;
  identity: IdentityInfo | null;
  convs: ConversationSummary[];
  contacts: Contact[];
  network: NetworkStatus | null;
  settings: Settings | null;
  interference: boolean;
  warnings: Warning[];
  dismissWarning: (w: Warning) => void;
  requests: PairingRequest[];
  dismissRequest: (p: Hex) => void;
  /** Bottom toast for confirmations ("Copied", "Deleted for you"). */
  notice: string | null;
  notify: (text: string) => void;
  /** Top banner for incoming messages while the app is open. */
  banner: InAppBanner | null;
  dismissBanner: () => void;
  refresh: () => Promise<void>;
  call: <T = unknown>(method: string, args?: Record<string, unknown>) => Promise<T>;
  onEvent: (fn: (e: CoreEvent) => void) => () => void;
  afterUnlock: () => Promise<void>;
}

const AppContext = createContext<Ctx | null>(null);

export function useApp(): Ctx {
  const c = useContext(AppContext);
  if (!c) throw new Error('useApp outside provider');
  return c;
}

export function errorText(e: unknown): string {
  if (e instanceof CoreError) return e.message;
  return e instanceof Error ? e.message : String(e);
}

export function AppProvider({ client, children }: { client: CoreClient; children: ReactNode }) {
  const [phase, setPhase] = useState<Phase>('loading');
  const [stack, setStack] = useState<Route[]>([{ name: 'Chats' }]);
  const [identity, setIdentity] = useState<IdentityInfo | null>(null);
  const [convs, setConvs] = useState<ConversationSummary[]>([]);
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [network, setNetwork] = useState<NetworkStatus | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [interference, setInterference] = useState(false);
  const [warnings, setWarnings] = useState<Warning[]>([]);
  const [requests, setRequests] = useState<PairingRequest[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const [banner, setBanner] = useState<InAppBanner | null>(null);
  const nearbyRef = useRef(-1);
  const convsRef = useRef<ConversationSummary[]>([]);
  convsRef.current = convs;
  const routeRef = useRef<Route>(stack[stack.length - 1]);
  routeRef.current = stack[stack.length - 1];

  const call = useCallback(<T,>(m: string, a?: Record<string, unknown>) => client.call<T>(m, a), [client]);

  const refresh = useCallback(async () => {
    try {
      const unlocked = await client.call<boolean>('is_unlocked');
      setNetwork(await client.call<NetworkStatus>('network_status'));
      setSettings(await client.call<Settings>('settings'));
      if (!unlocked) return;
      setConvs(await client.call<ConversationSummary[]>('conversations'));
      setContacts(await client.call<Contact[]>('contacts'));
      setIdentity(await client.call<IdentityInfo>('identity'));
    } catch {
      /* locked between calls */
    }
  }, [client]);

  const notify = useCallback((text: string) => {
    setNotice(text);
    setTimeout(() => setNotice((n) => (n === text ? null : n)), 2400);
  }, []);

  const showBanner = useCallback((title: string, sub: string, conv: Hex | null) => {
    const id = Date.now();
    setBanner({ title, sub, conv, id });
    setTimeout(() => setBanner((b) => (b?.id === id ? null : b)), 5000);
  }, []);

  const afterUnlock = useCallback(async () => {
    await refresh();
    // Device integrity at startup (Android attestation, section 12.3).
    if (client.hooks.attestationChain) {
      client.hooks
        .attestationChain()
        .then((chain) => (chain.length ? client.call('integrity_report', { chain }) : null))
        .catch(() => {});
    }
    setStack([{ name: 'Chats' }]);
    setPhase('ready');
  }, [refresh, client]);

  // Initial phase.
  useEffect(() => {
    (async () => {
      const has = await client.call<boolean>('has_identity');
      if (!has) return setPhase('welcome');
      const unlocked = await client.call<boolean>('is_unlocked');
      if (unlocked) await afterUnlock();
      else setPhase('locked');
    })();
  }, [client, afterUnlock]);

  // Core events.
  const listeners = useRef(new Set<(e: CoreEvent) => void>());
  useEffect(
    () =>
      client.subscribe((e) => {
        switch (e.event) {
          case 'ConversationsChanged':
          case 'ContactChanged':
          case 'MessageDeleted':
          case 'MessageState':
            // Previews, unread counts and ticks on the Chats list.
            refresh();
            break;
          case 'MessageAdded': {
            const r = routeRef.current;
            if (!e.message.outgoing && !(r.name === 'Chat' && r.conv === e.conversation)) {
              // Never the message text; the app is open and unlocked, so the
              // conversation name is shown, as in the design.
              // Read the fresh unread count from the core, not a stale list.
              client
                .call<ConversationSummary[]>('conversations')
                .then((list) => {
                  setConvs(list);
                  const c = list.find((x) => x.id === e.conversation);
                  const name = c ? (c.kind === 'Channel' ? `#${c.title}` : c.title) : 'A conversation';
                  showBanner('New message', c && c.unread > 0 ? `${name} · ${c.unread} new` : name, e.conversation);
                })
                .catch(() => {});
            }
            refresh();
            break;
          }
          case 'NetworkChanged':
            // A contact came into or left range: contact.nearby/link changed,
            // which the core reports through the contacts list.
            if (e.status.contacts_nearby !== nearbyRef.current) {
              nearbyRef.current = e.status.contacts_nearby;
              refresh();
            }
            setNetwork(e.status);
            break;
          case 'Interference':
            setInterference(e.suspected);
            break;
          case 'Warning':
            setWarnings((w) => [...w.filter((x) => JSON.stringify(x) !== JSON.stringify(e.warning)), e.warning]);
            break;
          case 'PairingResult':
            if (e.outcome.outcome === 'Request') {
              const channel = e.outcome.detail.channel;
              setRequests((r) => [...r, { pairing: e.pairing, channel }]);
            } else if (e.outcome.outcome === 'Paired') {
              // Channel requests finish in the background (either side), so
              // say so wherever the user is.
              const d = e.outcome.detail;
              notify(`Paired with ${d.nickname}${d.verified ? '' : ' · Not verified'}`);
            }
            refresh();
            break;
          case 'Locked':
            // Nothing readable stays in memory or on screen while locked.
            setConvs([]);
            setContacts([]);
            setIdentity(null);
            setBanner(null);
            setPhase('locked');
            break;
          case 'IncomingMessage':
            showBanner('New message', 'Unlock to read it', null);
            break;
          case 'Wiped':
            setIdentity(null);
            setConvs([]);
            setContacts([]);
            setPhase('welcome');
            break;
        }
        for (const l of listeners.current) l(e);
      }),
    [client, refresh, showBanner, notify],
  );

  const value = useMemo<Ctx>(
    () => ({
      client,
      phase,
      route: stack[stack.length - 1],
      push: (r) => setStack((s) => [...s, r]),
      pop: () => setStack((s) => (s.length > 1 ? s.slice(0, -1) : s)),
      reset: (r) => setStack([r]),
      tab: (name) => setStack([{ name }]),
      identity,
      convs,
      contacts,
      network,
      settings,
      // The event updates it live; the network status covers unlock and restart.
      interference: interference || !!network?.interference,
      warnings,
      dismissWarning: (w) => setWarnings((ws) => ws.filter((x) => JSON.stringify(x) !== JSON.stringify(w))),
      requests,
      dismissRequest: (p) => setRequests((r) => r.filter((x) => x.pairing !== p)),
      notice,
      notify,
      banner,
      dismissBanner: () => setBanner(null),
      refresh,
      call,
      onEvent: (fn) => {
        listeners.current.add(fn);
        return () => {
          listeners.current.delete(fn);
        };
      },
      afterUnlock,
    }),
    [client, phase, stack, identity, convs, contacts, network, settings, interference, warnings, requests, notice, notify, banner, refresh, call, afterUnlock],
  );
  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useCoreEvents(fn: (e: CoreEvent) => void, deps: unknown[]) {
  const { onEvent } = useApp();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => onEvent(fn), [onEvent, ...deps]);
}

/** Clock time today; weekday and date for any other day (past or future). */
export function timeAgo(ms: number | null | undefined): string {
  if (!ms) return '';
  const d = new Date(ms);
  const hm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  if (d.toDateString() === new Date().toDateString()) return hm;
  const day = d.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' });
  return `${day} ${hm}`;
}

export const DISAPPEAR_OPTIONS = [
  { value: 300, label: '5 minutes', short: '5m' },
  { value: 3600, label: '1 hour', short: '1h' },
  { value: 86400, label: '24 hours', short: '24h' },
  { value: 604800, label: '1 week', short: '1w' },
  { value: 2419200, label: '4 weeks', short: '4w' },
];

export function disappearShort(secs: number) {
  // 0 = no timer (public channels start that way).
  if (secs <= 0) return 'Off';
  return DISAPPEAR_OPTIONS.find((o) => o.value === secs)?.short ?? `${Math.round(secs / 3600)}h`;
}

export function disappearLabel(secs: number) {
  if (secs <= 0) return 'off';
  return DISAPPEAR_OPTIONS.find((o) => o.value === secs)?.label ?? `${Math.round(secs / 3600)} h`;
}

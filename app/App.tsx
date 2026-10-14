import React from 'react';
import { Platform, View } from 'react-native';
import { BricolageGrotesque_600SemiBold, BricolageGrotesque_700Bold } from '@expo-google-fonts/bricolage-grotesque';
import { Geist_400Regular, Geist_500Medium, Geist_600SemiBold, Geist_700Bold } from '@expo-google-fonts/geist';
import { GeistMono_500Medium, GeistMono_600SemiBold } from '@expo-google-fonts/geist-mono';
import { useFonts } from 'expo-font';
import { Harness } from './src/harness/Harness';
import { MacNode } from './src/mac/MacNode';
import { NativeRoot } from './src/native/NativeRoot';

// Web (harness, Mac node): fields show focus with their own border, not the browser ring.
if (Platform.OS === 'web' && typeof document !== 'undefined') {
  const st = document.createElement('style');
  st.textContent = 'input:focus,textarea:focus{outline:none}';
  document.head.appendChild(st);
}

export default function App() {
  // Font files ship inside the app bundle; nothing is fetched from the network.
  const [fontsReady] = useFonts({
    BricolageGrotesque_600SemiBold,
    BricolageGrotesque_700Bold,
    Geist_400Regular,
    Geist_500Medium,
    Geist_600SemiBold,
    Geist_700Bold,
    GeistMono_500Medium,
    GeistMono_600SemiBold,
  });
  if (!fontsReady) return <View style={{ flex: 1, backgroundColor: '#F6F5F1' }} />;
  if (Platform.OS !== 'web') return <NativeRoot />;
  // /?mode=mac: this Mac as a real node over Web Bluetooth; otherwise the simulator harness.
  const mac = typeof location !== 'undefined' && new URLSearchParams(location.search).get('mode') === 'mac';
  return mac ? <MacNode /> : <Harness />;
}

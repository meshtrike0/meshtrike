import React, { useEffect, useState } from 'react';
import { StatusBar, Text, View } from 'react-native';
import { SafeAreaProvider, SafeAreaView } from 'react-native-safe-area-context';
import { CoreClient } from '../core/client';
import { MeshApp } from '../ui/MeshApp';
import { color } from '../ui/theme';
import { createNativeClient } from './nativeClient';

export function NativeRoot() {
  const [client, setClient] = useState<CoreClient | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    createNativeClient().then(setClient, (e) => setError(String(e)));
  }, []);
  return (
    <SafeAreaProvider>
      <StatusBar barStyle="dark-content" backgroundColor={color.bg} />
      <SafeAreaView style={{ flex: 1, backgroundColor: color.bg }}>
        {client ? (
          <MeshApp client={client} />
        ) : (
          <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center' }}>
            <Text style={{ color: color.text }}>{error ?? 'Starting…'}</Text>
          </View>
        )}
      </SafeAreaView>
    </SafeAreaProvider>
  );
}

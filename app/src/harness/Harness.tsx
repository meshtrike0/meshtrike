import React from 'react';
import { Text, View } from 'react-native';

/** The multi-phone harness exists only on web. */
export function Harness() {
  return (
    <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center' }}>
      <Text>The browser harness runs on web only.</Text>
    </View>
  );
}

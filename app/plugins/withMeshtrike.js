// Android configuration for section 18 / D32: no backups, and release
// builds carry no INTERNET permission (debug keeps it for the dev server).
const { withAndroidManifest, withAppBuildGradle, withDangerousMod, withGradleProperties } = require('expo/config-plugins');
const fs = require('fs');
const path = require('path');

const RELEASE_MANIFEST = `<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
  xmlns:tools="http://schemas.android.com/tools">
  <!-- D7/D32: the app must not be able to reach any network. CI fails if a merged release manifest contains INTERNET. -->
  <uses-permission android:name="android.permission.INTERNET" tools:node="remove" />
  <uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" tools:node="remove" />
  <uses-permission android:name="android.permission.ACCESS_WIFI_STATE" tools:node="remove" />
  <uses-permission android:name="android.permission.SYSTEM_ALERT_WINDOW" tools:node="remove" />
</manifest>
`;

const EXTRACTION_RULES = `<?xml version="1.0" encoding="utf-8"?>
<data-extraction-rules>
  <cloud-backup>
    <exclude domain="root" path="." />
    <exclude domain="file" path="." />
    <exclude domain="database" path="." />
    <exclude domain="sharedpref" path="." />
    <exclude domain="external" path="." />
  </cloud-backup>
  <device-transfer>
    <exclude domain="root" path="." />
    <exclude domain="file" path="." />
    <exclude domain="database" path="." />
    <exclude domain="sharedpref" path="." />
    <exclude domain="external" path="." />
  </device-transfer>
</data-extraction-rules>
`;

function withNoBackup(config) {
  return withAndroidManifest(config, (c) => {
    const app = c.modResults.manifest.application[0];
    app.$['android:allowBackup'] = 'false';
    app.$['android:fullBackupContent'] = 'false';
    app.$['android:dataExtractionRules'] = '@xml/data_extraction_rules';
    c.modResults.manifest.$['xmlns:tools'] = 'http://schemas.android.com/tools';
    return c;
  });
}

function withReleaseFiles(config) {
  return withDangerousMod(config, [
    'android',
    async (c) => {
      const root = c.modRequest.platformProjectRoot;
      const rel = path.join(root, 'app/src/release');
      fs.mkdirSync(rel, { recursive: true });
      fs.writeFileSync(path.join(rel, 'AndroidManifest.xml'), RELEASE_MANIFEST);
      const xml = path.join(root, 'app/src/main/res/xml');
      fs.mkdirSync(xml, { recursive: true });
      fs.writeFileSync(path.join(xml, 'data_extraction_rules.xml'), EXTRACTION_RULES);
      return c;
    },
  ]);
}

// The Rust core is built for these ABIs only (cargo ndk -t arm64-v8a -t x86_64).
function withAbis(config) {
  return withGradleProperties(config, (c) => {
    c.modResults = c.modResults.filter((p) => !(p.type === 'property' && p.key === 'reactNativeArchitectures'));
    c.modResults.push({ type: 'property', key: 'reactNativeArchitectures', value: 'arm64-v8a,x86_64' });
    return c;
  });
}

// Release signing: when ci/build-android.sh runs with MESHTRIKE_RELEASE=1 it
// passes the private release key (from ~/MeshtrikeKeys and the macOS Keychain)
// through the environment; nothing secret is ever written into the project.
// Without it, release builds keep the local debug key for testing only.
function withReleaseSigning(config) {
  return withAppBuildGradle(config, (c) => {
    let g = c.modResults.contents;
    if (g.includes('MESHTRIKE_KEYSTORE')) return c;
    g = g.replace(
      /signingConfigs \{\n(\s*)debug \{/,
      (m, ind) =>
        `signingConfigs {\n${ind}meshtrikeRelease {\n` +
        `${ind}    storeFile System.getenv('MESHTRIKE_KEYSTORE') ? file(System.getenv('MESHTRIKE_KEYSTORE')) : null\n` +
        `${ind}    storePassword System.getenv('MESHTRIKE_STORE_PASSWORD')\n` +
        `${ind}    keyAlias 'meshtrike'\n` +
        `${ind}    keyPassword System.getenv('MESHTRIKE_STORE_PASSWORD')\n` +
        `${ind}}\n${ind}debug {`,
    );
    g = g.replace(
      /(release \{[\s\S]*?)signingConfig signingConfigs\.debug/,
      `$1signingConfig System.getenv('MESHTRIKE_KEYSTORE') ? signingConfigs.meshtrikeRelease : signingConfigs.debug`,
    );
    if (!g.includes('signingConfigs.meshtrikeRelease')) throw new Error('withMeshtrike: could not wire release signing');
    c.modResults.contents = g;
    return c;
  });
}

// The app's C++ libraries (React Native codegen) embed source paths in assert
// messages; map the build machine's home folder and the project folder to
// neutral names so no user name or disk name ends up in a published APK.
function withNeutralNativePaths(config) {
  return withAppBuildGradle(config, (c) => {
    let g = c.modResults.contents;
    if (g.includes('ffile-prefix-map')) return c;
    const maps =
      `"-ffile-prefix-map=\${System.getProperty('user.home')}=/home", ` +
      `"-ffile-prefix-map=\${rootDir.parentFile.parentFile}=/src"`;
    g = g.replace(
      /(\n(\s*)defaultConfig \{\n)/,
      (m, head, ind) =>
        `${head}${ind}    externalNativeBuild {\n${ind}        cmake {\n` +
        `${ind}            cFlags ${maps}\n${ind}            cppFlags ${maps}\n` +
        `${ind}        }\n${ind}    }\n`,
    );
    if (!g.includes('ffile-prefix-map')) throw new Error('withMeshtrike: could not add native path mapping');
    c.modResults.contents = g;
    return c;
  });
}

module.exports = (config) =>
  withNeutralNativePaths(withReleaseSigning(withAbis(withReleaseFiles(withNoBackup(config)))));

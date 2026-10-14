/*
 * Site configuration. Change download links and version labels here only.
 *
 * MESHTRIKE_APK_URL  - the main Meshtrike app APK (no internet permission).
 * REACH_APK_URL      - the optional MeshGRID add-on APK
 *                      (Wi-Fi Direct + the private internet route over the Nym mixnet).
 * RELEASES_URL       - every release, with signed checksums and verify.sh.
 *
 * The pages also carry these URLs in their HTML (for visitors without JavaScript);
 * js/main.js replaces them with the values below, so this file stays the one to edit.
 */
window.MESHTRIKE_CONFIG = {
  MESHTRIKE_APK_URL: "https://github.com/meshtrike0/meshtrike/releases/latest/download/meshtrike.apk",
  REACH_APK_URL: "https://github.com/meshtrike0/meshtrike/releases/latest/download/meshgrid.apk",
  RELEASES_URL: "https://github.com/meshtrike0/meshtrike/releases",

  // Shown in the hero badge, the download page and the compare table.
  MESHTRIKE_VERSION: "v1.0.0",
  REACH_VERSION: "v1.2.0",

  // File names shown on the download page and in the "Downloading ..." confirmation.
  MESHTRIKE_APK_NAME: "meshtrike.apk",
  REACH_APK_NAME: "meshgrid.apk"
};

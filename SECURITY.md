# Security policy

Meshtrike is meant for people in difficult places, so security reports matter more to us than anything else.

## Reporting a vulnerability

Please report it **privately**: open the [Security tab](https://github.com/meshtrike0/meshtrike/security) of this repository and choose **Report a vulnerability**. Only the Meshtrike team can see the report.

Please do not open a public issue for security problems. Include what you found, how to reproduce it, and which version (Settings → About & limits shows it).

We aim to reply within 7 days, fix serious issues as fast as we can, and credit you in the release notes unless you'd rather stay anonymous.

## Scope

In scope: the Rust core (`core/`), the Android app (`app/`), the MeshGRID add-on (`plugins/link/`), the release and signing process, and the website's verification page.

Especially interesting: anything that lets a relaying phone read, link or trace messages; breaks of the app's no-internet guarantee; screenshot or notification leaks; ways to get a fake APK accepted as real.

## Status

Meshtrike has not yet had an independent security audit. Treat it as beta software for high-risk use.

## Verifying releases

Every release is signed with the Meshtrike release key. Check any copy with `sh verify.sh <apk>`, or follow https://meshtrike0.github.io/verify.html.

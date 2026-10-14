/*
 * Check a Meshtrike / MeshGRID APK in the browser, the way Android does:
 *   1. find the APK Signature Scheme v2 (or v3) block,
 *   2. verify the signer's signature over the signed data with its public key,
 *   3. recompute the content digest of the whole file and compare it,
 *   4. check the public key belongs to the signing certificate, and that the
 *      certificate's SHA-256 is the official Meshtrike release fingerprint.
 * The file never leaves the device: it is read locally with the File API and
 * hashed with WebCrypto (the page's CSP also blocks every network request).
 */
(function () {
  'use strict';

  // Official release certificate (SHA-256 of the DER certificate), as printed by
  // `apksigner verify --print-certs` and published in the README and whitepaper.
  var OFFICIAL_CERT_SHA256 = 'AB:09:7B:F4:3C:9B:63:0F:06:67:02:E0:61:9A:60:43:05:13:6A:D2:D6:4B:AA:38:E7:62:2F:AF:17:09:AD:3E';

  var V2_ID = 0x7109871a, V3_ID = 0xf05368c0, V31_ID = 0x1b93ad61;
  var CHUNK = 1048576;
  // signature algorithm id -> how to verify it, and which content digest it covers
  var ALGS = {
    0x0101: { name: 'RSA-PSS', hash: 'SHA-256', salt: 32, digest: 1 },
    0x0102: { name: 'RSA-PSS', hash: 'SHA-512', salt: 64, digest: 2 },
    0x0103: { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256', digest: 1 },
    0x0104: { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-512', digest: 2 },
    0x0201: { name: 'ECDSA', hash: 'SHA-256', digest: 1 },
    0x0202: { name: 'ECDSA', hash: 'SHA-512', digest: 2 }
  };
  var DIGEST_HASH = { 1: 'SHA-256', 2: 'SHA-512' };

  function Fail(msg) { this.message = msg; }

  function u32(b, o) { return (b[o] | (b[o + 1] << 8) | (b[o + 2] << 16) | (b[o + 3] << 24)) >>> 0; }
  function u64(b, o) { return u32(b, o) + u32(b, o + 4) * 4294967296; }
  function hex(buf) {
    var b = new Uint8Array(buf), s = '';
    for (var i = 0; i < b.length; i++) s += (b[i] < 16 ? '0' : '') + b[i].toString(16);
    return s;
  }
  function same(a, b) {
    if (a.length !== b.length) return false;
    for (var i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }
  // A u32-length-prefixed field: returns [bytes, next offset].
  function lp(b, o) {
    if (o + 4 > b.length) throw new Fail('The signature data is cut short.');
    var n = u32(b, o);
    if (o + 4 + n > b.length) throw new Fail('The signature data is cut short.');
    return [b.subarray(o + 4, o + 4 + n), o + 4 + n];
  }
  function seq(b) {
    var out = [], o = 0;
    while (o < b.length) { var r = lp(b, o); out.push(r[0]); o = r[1]; }
    return out;
  }

  // Minimal DER walk: returns the SubjectPublicKeyInfo bytes of an X.509 certificate.
  function der(b, o) {
    var tag = b[o], len = b[o + 1], h = 2;
    if (len & 0x80) { var n = len & 0x7f; len = 0; for (var i = 0; i < n; i++) len = len * 256 + b[o + 2 + i]; h = 2 + n; }
    return { tag: tag, start: o, body: o + h, end: o + h + len };
  }
  function certSpki(cert) {
    var c = der(cert, 0), tbs = der(cert, c.body), o = tbs.body;
    var f = der(cert, o);
    if (f.tag === 0xa0) o = f.end;          // [0] version
    for (var i = 0; i < 5; i++) o = der(cert, o).end; // serial, sigAlg, issuer, validity, subject
    var spki = der(cert, o);
    return cert.subarray(spki.start, spki.end);
  }
  // ECDSA signatures in APKs are DER; WebCrypto wants raw r||s.
  function ecdsaRaw(sig, size) {
    var s = der(sig, 0), r = der(sig, s.body), t = der(sig, r.end);
    function fix(x) { var v = sig.subarray(x.body, x.end); while (v.length > size && v[0] === 0) v = v.subarray(1); var o = new Uint8Array(size); o.set(v, size - v.length); return o; }
    var out = new Uint8Array(size * 2); out.set(fix(r), 0); out.set(fix(t), size); return out;
  }

  async function readAt(file, start, len) {
    return new Uint8Array(await file.slice(start, start + len).arrayBuffer());
  }

  async function locate(file) {
    var tailLen = Math.min(file.size, 65557);
    var tail = await readAt(file, file.size - tailLen, tailLen);
    var e = -1;
    for (var i = tail.length - 22; i >= 0; i--) {
      if (tail[i] === 0x50 && tail[i + 1] === 0x4b && tail[i + 2] === 0x05 && tail[i + 3] === 0x06) { e = i; break; }
    }
    if (e < 0) throw new Fail('This is not an APK (no ZIP directory found).');
    var eocdOff = file.size - tailLen + e;
    var eocd = tail.subarray(e);
    var cdOff = u32(eocd, 16), cdSize = u32(eocd, 12);
    if (cdOff + cdSize !== eocdOff) throw new Fail('The APK structure is not valid.');
    var foot = await readAt(file, cdOff - 24, 24);
    if (String.fromCharCode.apply(null, foot.subarray(8)) !== 'APK Sig Block 42') {
      throw new Fail('This APK has no modern Android signature, so it is not a Meshtrike release.');
    }
    var size = u64(foot, 0), blockOff = cdOff - size - 8;
    var block = await readAt(file, blockOff + 8, size - 16);
    var pairs = {}, o = 0;
    while (o + 12 <= block.length) {
      var len = u64(block, o), id = u32(block, o + 8);
      pairs[id] = block.subarray(o + 12, o + 8 + len);
      o += 8 + len;
    }
    return { blockOff: blockOff, cdOff: cdOff, cdSize: cdSize, eocdOff: eocdOff, eocd: eocd, pairs: pairs };
  }

  function parseSigner(raw, v3) {
    var r = lp(raw, 0), signedData = r[0], o = r[1];
    if (v3) o += 8; // min/max SDK
    var s = lp(raw, o), k = lp(raw, s[1]);
    var sd = lp(signedData, 0), certs = lp(signedData, sd[1]);
    return {
      signedData: signedData,
      digests: seq(sd[0]).map(function (d) { return { alg: u32(d, 0), value: lp(d, 4)[0] }; }),
      certs: seq(certs[0]),
      sigs: seq(s[0]).map(function (x) { return { alg: u32(x, 0), value: lp(x, 4)[0] }; }),
      publicKey: k[0]
    };
  }

  async function contentDigest(file, loc, hashName, onProgress) {
    var eocd = new Uint8Array(loc.eocd);
    eocd[16] = loc.blockOff & 255; eocd[17] = (loc.blockOff >>> 8) & 255;
    eocd[18] = (loc.blockOff >>> 16) & 255; eocd[19] = (loc.blockOff >>> 24) & 255;
    var parts = [[0, loc.blockOff], [loc.cdOff, loc.cdSize], null];
    var total = Math.ceil(loc.blockOff / CHUNK) + Math.ceil(loc.cdSize / CHUNK) + 1;
    var digests = [], done = 0;
    for (var p = 0; p < 3; p++) {
      var start = p < 2 ? parts[p][0] : 0, len = p < 2 ? parts[p][1] : eocd.length;
      for (var off = 0; off < len; off += CHUNK) {
        var n = Math.min(CHUNK, len - off);
        var bytes = p < 2 ? await readAt(file, start + off, n) : eocd;
        var buf = new Uint8Array(5 + n);
        buf[0] = 0xa5; buf[1] = n & 255; buf[2] = (n >>> 8) & 255; buf[3] = (n >>> 16) & 255; buf[4] = (n >>> 24) & 255;
        buf.set(bytes, 5);
        digests.push(new Uint8Array(await crypto.subtle.digest(hashName, buf)));
        done++;
        if (onProgress) onProgress(done / total);
      }
    }
    var dl = digests[0].length, top = new Uint8Array(5 + dl * digests.length);
    var c = digests.length;
    top[0] = 0x5a; top[1] = c & 255; top[2] = (c >>> 8) & 255; top[3] = (c >>> 16) & 255; top[4] = (c >>> 24) & 255;
    for (var i = 0; i < c; i++) top.set(digests[i], 5 + i * dl);
    return new Uint8Array(await crypto.subtle.digest(hashName, top));
  }

  async function verifySignature(alg, spki, sig, data) {
    var key;
    if (alg.name === 'ECDSA') {
      var curve = spki.length > 120 ? 'P-384' : 'P-256';
      key = await crypto.subtle.importKey('spki', spki, { name: 'ECDSA', namedCurve: curve }, false, ['verify']);
      return crypto.subtle.verify({ name: 'ECDSA', hash: alg.hash }, key, ecdsaRaw(sig, curve === 'P-384' ? 48 : 32), data);
    }
    key = await crypto.subtle.importKey('spki', spki, { name: alg.name, hash: alg.hash }, false, ['verify']);
    return crypto.subtle.verify(alg.name === 'RSA-PSS' ? { name: 'RSA-PSS', saltLength: alg.salt } : { name: alg.name }, key, sig, data);
  }

  // Returns { real, certSha256, fileSha256, reason }.
  async function check(file, onProgress) {
    var loc = await locate(file);
    var v3 = !loc.pairs[V2_ID] && !!(loc.pairs[V3_ID] || loc.pairs[V31_ID]);
    var raw = loc.pairs[V2_ID] || loc.pairs[V3_ID] || loc.pairs[V31_ID];
    if (!raw) throw new Fail('This APK has no Android v2/v3 signature, so it is not a Meshtrike release.');
    var signers = seq(lp(raw, 0)[0]);
    if (signers.length !== 1) throw new Fail('This APK has ' + signers.length + ' signers; a Meshtrike release has exactly one.');
    var s = parseSigner(signers[0], v3);

    var best = null;
    s.sigs.forEach(function (x) {
      var a = ALGS[x.alg];
      if (a && (!best || (a.hash === 'SHA-512' && ALGS[best.alg].hash !== 'SHA-512'))) best = x;
    });
    if (!best) throw new Fail('This APK uses a signature type this page cannot check.');
    var alg = ALGS[best.alg];

    var ok = await verifySignature(alg, s.publicKey, best.value, s.signedData);
    if (!ok) return { real: false, reason: 'The signature is broken: the APK was changed after it was signed.' };

    var listed = s.digests.filter(function (d) { return d.alg === best.alg; })[0];
    if (!listed) return { real: false, reason: 'The signed file list is incomplete.' };
    var actual = await contentDigest(file, loc, DIGEST_HASH[alg.digest], onProgress);
    if (!same(actual, listed.value)) return { real: false, reason: 'The file contents do not match the signature: the APK was modified.' };

    var cert = s.certs[0];
    if (!cert || !same(certSpki(cert), s.publicKey)) return { real: false, reason: 'The signing key does not match its certificate.' };
    var certSha = hex(await crypto.subtle.digest('SHA-256', cert));
    var fileSha = hex(await crypto.subtle.digest('SHA-256', await file.arrayBuffer()));
    var want = OFFICIAL_CERT_SHA256.replace(/[^0-9a-f]/gi, '').toLowerCase();
    if (want.length !== 64) return { real: null, certSha256: certSha, fileSha256: fileSha, reason: 'The official fingerprint is published with the first signed release.' };
    if (certSha !== want) return { real: false, certSha256: certSha, fileSha256: fileSha, reason: 'The APK is validly signed, but by someone else: not the Meshtrike release key.' };
    return { real: true, certSha256: certSha, fileSha256: fileSha };
  }

  window.MeshtrikeVerify = { check: check, OFFICIAL_CERT_SHA256: OFFICIAL_CERT_SHA256 };

  // ---------- page UI ----------
  function fp(h) { return h.toUpperCase().match(/.{2}/g).join(':'); }
  function init() {
    var root = document.querySelector('[data-checker]');
    if (!root) return;
    var input = root.querySelector('input[type=file]');
    var drop = root.querySelector('[data-drop]');
    var out = root.querySelector('[data-result]');
    var bar = root.querySelector('[data-progress]');

    function show(kind, title, lines) {
      out.hidden = false;
      out.className = 'checker__result checker__result--' + kind;
      out.textContent = '';
      var t = document.createElement('p'); t.className = 'checker__title'; t.textContent = title; out.appendChild(t);
      lines.forEach(function (l) {
        var p = document.createElement('p');
        if (l.mono) { p.className = 'checker__mono'; }
        p.textContent = l.text || l; out.appendChild(p);
      });
    }
    async function run(file) {
      if (!file) return;
      if (!window.crypto || !crypto.subtle) { show('bad', 'This browser cannot check files', ['Use a recent Chrome, Firefox or Safari, or the computer method below.']); return; }
      show('busy', 'Checking ' + file.name + '…', ['Reading and verifying on this device. Nothing is uploaded.']);
      bar.hidden = false; bar.value = 0;
      try {
        var r = await check(file, function (f) { bar.value = f; });
        var details = [];
        if (r.certSha256) details.push({ text: 'Signing certificate SHA-256:' }, { text: fp(r.certSha256), mono: true });
        if (r.fileSha256) details.push({ text: 'File SHA-256:' }, { text: r.fileSha256, mono: true });
        if (r.real === true) show('good', 'REAL: signed by Meshtrike', ['The signature is valid, nothing in the file was changed, and it was signed with the official Meshtrike release key. Safe to install and to pass on.'].concat(details));
        else if (r.real === null) show('busy', 'Signature valid; official key not published yet', [r.reason].concat(details));
        else show('bad', 'NOT REAL: do not install it', [r.reason, 'Delete it, do not pass it on, and warn whoever gave it to you.'].concat(details));
      } catch (e) {
        show('bad', 'NOT REAL: do not install it', [e instanceof Fail ? e.message : 'The file could not be read as an APK.']);
      } finally { bar.hidden = true; }
    }
    input.addEventListener('change', function () { run(input.files[0]); });
    ['dragenter', 'dragover'].forEach(function (ev) { drop.addEventListener(ev, function (e) { e.preventDefault(); drop.classList.add('is-over'); }); });
    ['dragleave', 'drop'].forEach(function (ev) { drop.addEventListener(ev, function (e) { e.preventDefault(); drop.classList.remove('is-over'); }); });
    drop.addEventListener('drop', function (e) { run(e.dataTransfer.files[0]); });
  }
  if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init); else init();
  }
})();

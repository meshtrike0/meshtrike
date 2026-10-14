//! Android hardware key attestation (blueprint section 12.3).
//!
//! The Android shell generates a Keystore key with an attestation challenge at
//! first launch and every 7 days, then hands the certificate chain returned by
//! `KeyStore.getCertificateChain(alias)` (DER, leaf first) to [`verify_chain`].
//! Verification is fully offline:
//!
//! 1. every certificate's signature is checked with the next certificate's
//!    public key (ECDSA P-256 / P-384 with SHA-256/384/512, RSA PKCS#1 v1.5
//!    with SHA-256/384/512);
//! 2. the last certificate's SubjectPublicKeyInfo must be byte-identical to one
//!    of Google's hardware attestation roots bundled below;
//! 3. the Android key description extension (OID 1.3.6.1.4.1.11129.2.1.17) of
//!    the leaf is parsed and its hardware-enforced root of trust inspected.
//!
//! The result is advisory only: a `Warning` is shown as a persistent red banner
//! and never blocks use.
//!
//! Deliberately out of scope:
//! - Validity periods are not checked: the device clock is untrusted offline,
//!   and attestation leaves routinely carry `notBefore` of 1970.
//! - Revocation (<https://android.googleapis.com/attestation/status>) needs the
//!   network, so revoked-but-genuine keys are not detected here.
//! - The attestation challenge is not compared: the result is a local health
//!   indicator, not a remote attestation protocol, so replay is irrelevant.

use crate::types::IntegrityStatus;

/// Verify an Android key attestation certificate chain (DER, leaf first).
///
/// Built without the `integrity` feature (e.g. the WASM target) this always
/// returns [`IntegrityStatus::Unknown`].
#[cfg(not(feature = "integrity"))]
pub fn verify_chain(chain: &[Vec<u8>]) -> IntegrityStatus {
    let _ = chain;
    IntegrityStatus::Unknown
}

/// Verify an Android key attestation certificate chain (DER, leaf first).
///
/// Never panics, whatever the input bytes. See the module docs for the checks.
#[cfg(feature = "integrity")]
pub fn verify_chain(chain: &[Vec<u8>]) -> IntegrityStatus {
    imp::evaluate(chain)
}

#[cfg(feature = "integrity")]
mod imp {
    use super::IntegrityStatus;
    use base64::Engine as _;
    use p256::ecdsa::signature::hazmat::PrehashVerifier as _;
    use rsa::pkcs1::DecodeRsaPublicKey as _;
    use sha2::Digest as _;

    pub(super) const MSG_UNVERIFIED: &str =
        "Attestation could not be verified (unknown or software root)";
    pub(super) const MSG_SOFTWARE: &str = "Keys are not protected by secure hardware";
    pub(super) const MSG_UNLOCKED: &str = "Bootloader is unlocked";

    /// Real chains are 3-5 certificates; anything far longer is rejected
    /// before doing any signature work.
    const MAX_CHAIN_LEN: usize = 10;

    // ---------------------------------------------------------------------
    // Google hardware attestation roots.
    //
    // Fetched 2026-10-11 from
    // https://developer.android.com/privacy-and-security/security-key-attestation
    // ("Root certificates" and "Previously issued root certificates"), and
    // cross-checked against https://android.googleapis.com/attestation/root
    // (which lists ROOT_RSA_2022 and ROOT_EC_CA1_2025).
    //
    // All four RSA certificates (subject serialNumber=f92009e853b6b045) carry
    // the same RSA-4096 public key; they are kept verbatim so the bundle
    // matches Google's published list. ROOT_EC_CA1_2025 ("Key Attestation CA1",
    // ECDSA P-384) signs attestation chains from 2026-02-01 onwards.
    // ---------------------------------------------------------------------

    /// Current. Subject serialNumber=f92009e853b6b045, RSA-4096, 2022-03-20 .. 2042-03-15.
    const ROOT_RSA_2022: &str = "\
-----BEGIN CERTIFICATE-----
MIIFHDCCAwSgAwIBAgIJAPHBcqaZ6vUdMA0GCSqGSIb3DQEBCwUAMBsxGTAXBgNV
BAUTEGY5MjAwOWU4NTNiNmIwNDUwHhcNMjIwMzIwMTgwNzQ4WhcNNDIwMzE1MTgw
NzQ4WjAbMRkwFwYDVQQFExBmOTIwMDllODUzYjZiMDQ1MIICIjANBgkqhkiG9w0B
AQEFAAOCAg8AMIICCgKCAgEAr7bHgiuxpwHsK7Qui8xUFmOr75gvMsd/dTEDDJdS
Sxtf6An7xyqpRR90PL2abxM1dEqlXnf2tqw1Ne4Xwl5jlRfdnJLmN0pTy/4lj4/7
tv0Sk3iiKkypnEUtR6WfMgH0QZfKHM1+di+y9TFRtv6y//0rb+T+W8a9nsNL/ggj
nar86461qO0rOs2cXjp3kOG1FEJ5MVmFmBGtnrKpa73XpXyTqRxB/M0n1n/W9nGq
C4FSYa04T6N5RIZGBN2z2MT5IKGbFlbC8UrW0DxW7AYImQQcHtGl/m00QLVWutHQ
oVJYnFPlXTcHYvASLu+RhhsbDmxMgJJ0mcDpvsC4PjvB+TxywElgS70vE0XmLD+O
JtvsBslHZvPBKCOdT0MS+tgSOIfga+z1Z1g7+DVagf7quvmag8jfPioyKvxnK/Eg
sTUVi2ghzq8wm27ud/mIM7AY2qEORR8Go3TVB4HzWQgpZrt3i5MIlCaY504LzSRi
igHCzAPlHws+W0rB5N+er5/2pJKnfBSDiCiFAVtCLOZ7gLiMm0jhO2B6tUXHI/+M
RPjy02i59lINMRRev56GKtcd9qO/0kUJWdZTdA2XoS82ixPvZtXQpUpuL12ab+9E
aDK8Z4RHJYYfCT3Q5vNAXaiWQ+8PTWm2QgBR/bkwSWc+NpUFgNPN9PvQi8WEg5Um
AGMCAwEAAaNjMGEwHQYDVR0OBBYEFDZh4QB8iAUJUYtEbEf/GkzJ6k8SMB8GA1Ud
IwQYMBaAFDZh4QB8iAUJUYtEbEf/GkzJ6k8SMA8GA1UdEwEB/wQFMAMBAf8wDgYD
VR0PAQH/BAQDAgIEMA0GCSqGSIb3DQEBCwUAA4ICAQB8cMqTllHc8U+qCrOlg3H7
174lmaCsbo/bJ0C17JEgMLb4kvrqsXZs01U3mB/qABg/1t5Pd5AORHARs1hhqGIC
W/nKMav574f9rZN4PC2ZlufGXb7sIdJpGiO9ctRhiLuYuly10JccUZGEHpHSYM2G
tkgYbZba6lsCPYAAP83cyDV+1aOkTf1RCp/lM0PKvmxYN10RYsK631jrleGdcdkx
oSK//mSQbgcWnmAEZrzHoF1/0gso1HZgIn0YLzVhLSA/iXCX4QT2h3J5z3znluKG
1nv8NQdxei2DIIhASWfu804CA96cQKTTlaae2fweqXjdN1/v2nqOhngNyz1361mF
mr4XmaKH/ItTwOe72NI9ZcwS1lVaCvsIkTDCEXdm9rCNPAY10iTunIHFXRh+7KPz
lHGewCq/8TOohBRn0/NNfh7uRslOSZ/xKbN9tMBtw37Z8d2vvnXq/YWdsm1+JLVw
n6yYD/yacNJBlwpddla8eaVMjsF6nBnIgQOf9zKSe06nSTqvgwUHosgOECZJZ1Eu
zbH4yswbt02tKtKEFhx+v+OTge/06V+jGsqTWLsfrOCNLuA8H++z+pUENmpqnnHo
vaI47gC+TNpkgYGkkBT6B/m/U01BuOBBTzhIlMEZq9qkDWuM2cA5kW5V3FJUcfHn
w1IdYIg2Wxg7yHcQZemFQg==
-----END CERTIFICATE-----
";

    /// Current (from 2026-02-01). Subject CN=Key Attestation CA1, OU=Android,
    /// O=Google LLC, C=US, ECDSA P-384, 2025-07-17 .. 2035-07-15.
    const ROOT_EC_CA1_2025: &str = "\
-----BEGIN CERTIFICATE-----
MIICIjCCAaigAwIBAgIRAISp0Cl7DrWK5/8OgN52BgUwCgYIKoZIzj0EAwMwUjEc
MBoGA1UEAwwTS2V5IEF0dGVzdGF0aW9uIENBMTEQMA4GA1UECwwHQW5kcm9pZDET
MBEGA1UECgwKR29vZ2xlIExMQzELMAkGA1UEBhMCVVMwHhcNMjUwNzE3MjIzMjE4
WhcNMzUwNzE1MjIzMjE4WjBSMRwwGgYDVQQDDBNLZXkgQXR0ZXN0YXRpb24gQ0Ex
MRAwDgYDVQQLDAdBbmRyb2lkMRMwEQYDVQQKDApHb29nbGUgTExDMQswCQYDVQQG
EwJVUzB2MBAGByqGSM49AgEGBSuBBAAiA2IABCPaI3FO3z5bBQo8cuiEas4HjqCt
G/mLFfRT0MsIssPBEEU5Cfbt6sH5yOAxqEi5QagpU1yX4HwnGb7OtBYpDTB57uH5
Eczm34A5FNijV3s0/f0UPl7zbJcTx6xwqMIRq6NCMEAwDwYDVR0TAQH/BAUwAwEB
/zAOBgNVHQ8BAf8EBAMCAQYwHQYDVR0OBBYEFFIyuyz7RkOb3NaBqQ5lZuA0QepA
MAoGCCqGSM49BAMDA2gAMGUCMETfjPO/HwqReR2CS7p0ZWoD/LHs6hDi422opifH
EUaYLxwGlT9SLdjkVpz0UUOR5wIxAIoGyxGKRHVTpqpGRFiJtQEOOTp/+s1GcxeY
uR2zh/80lQyu9vAFCj6E4AXc+osmRg==
-----END CERTIFICATE-----
";

    /// Previously issued. Subject serialNumber=f92009e853b6b045, RSA-4096, 2016-05-26 .. 2026-05-24.
    const ROOT_RSA_2016: &str = "\
-----BEGIN CERTIFICATE-----
MIIFYDCCA0igAwIBAgIJAOj6GWMU0voYMA0GCSqGSIb3DQEBCwUAMBsxGTAXBgNV
BAUTEGY5MjAwOWU4NTNiNmIwNDUwHhcNMTYwNTI2MTYyODUyWhcNMjYwNTI0MTYy
ODUyWjAbMRkwFwYDVQQFExBmOTIwMDllODUzYjZiMDQ1MIICIjANBgkqhkiG9w0B
AQEFAAOCAg8AMIICCgKCAgEAr7bHgiuxpwHsK7Qui8xUFmOr75gvMsd/dTEDDJdS
Sxtf6An7xyqpRR90PL2abxM1dEqlXnf2tqw1Ne4Xwl5jlRfdnJLmN0pTy/4lj4/7
tv0Sk3iiKkypnEUtR6WfMgH0QZfKHM1+di+y9TFRtv6y//0rb+T+W8a9nsNL/ggj
nar86461qO0rOs2cXjp3kOG1FEJ5MVmFmBGtnrKpa73XpXyTqRxB/M0n1n/W9nGq
C4FSYa04T6N5RIZGBN2z2MT5IKGbFlbC8UrW0DxW7AYImQQcHtGl/m00QLVWutHQ
oVJYnFPlXTcHYvASLu+RhhsbDmxMgJJ0mcDpvsC4PjvB+TxywElgS70vE0XmLD+O
JtvsBslHZvPBKCOdT0MS+tgSOIfga+z1Z1g7+DVagf7quvmag8jfPioyKvxnK/Eg
sTUVi2ghzq8wm27ud/mIM7AY2qEORR8Go3TVB4HzWQgpZrt3i5MIlCaY504LzSRi
igHCzAPlHws+W0rB5N+er5/2pJKnfBSDiCiFAVtCLOZ7gLiMm0jhO2B6tUXHI/+M
RPjy02i59lINMRRev56GKtcd9qO/0kUJWdZTdA2XoS82ixPvZtXQpUpuL12ab+9E
aDK8Z4RHJYYfCT3Q5vNAXaiWQ+8PTWm2QgBR/bkwSWc+NpUFgNPN9PvQi8WEg5Um
AGMCAwEAAaOBpjCBozAdBgNVHQ4EFgQUNmHhAHyIBQlRi0RsR/8aTMnqTxIwHwYD
VR0jBBgwFoAUNmHhAHyIBQlRi0RsR/8aTMnqTxIwDwYDVR0TAQH/BAUwAwEB/zAO
BgNVHQ8BAf8EBAMCAYYwQAYDVR0fBDkwNzA1oDOgMYYvaHR0cHM6Ly9hbmRyb2lk
Lmdvb2dsZWFwaXMuY29tL2F0dGVzdGF0aW9uL2NybC8wDQYJKoZIhvcNAQELBQAD
ggIBACDIw41L3KlXG0aMiS//cqrG+EShHUGo8HNsw30W1kJtjn6UBwRM6jnmiwfB
Pb8VA91chb2vssAtX2zbTvqBJ9+LBPGCdw/E53Rbf86qhxKaiAHOjpvAy5Y3m00m
qC0w/Zwvju1twb4vhLaJ5NkUJYsUS7rmJKHHBnETLi8GFqiEsqTWpG/6ibYCv7rY
DBJDcR9W62BW9jfIoBQcxUCUJouMPH25lLNcDc1ssqvC2v7iUgI9LeoM1sNovqPm
QUiG9rHli1vXxzCyaMTjwftkJLkf6724DFhuKug2jITV0QkXvaJWF4nUaHOTNA4u
JU9WDvZLI1j83A+/xnAJUucIv/zGJ1AMH2boHqF8CY16LpsYgBt6tKxxWH00XcyD
CdW2KlBCeqbQPcsFmWyWugxdcekhYsAWyoSf818NUsZdBWBaR/OukXrNLfkQ79Iy
ZohZbvabO/X+MVT3rriAoKc8oE2Uws6DF+60PV7/WIPjNvXySdqspImSN78mflxD
qwLqRBYkA3I75qppLGG9rp7UCdRjxMl8ZDBld+7yvHVgt1cVzJx9xnyGCC23Uaic
MDSXYrB4I4WHXPGjxhZuCuPBLTdOLU8YRvMYdEvYebWHMpvwGCF6bAx3JBpIeOQ1
wDB5y0USicV3YgYGmi+NZfhA4URSh77Yd6uuJOJENRaNVTzk
-----END CERTIFICATE-----
";

    /// Previously issued. Subject serialNumber=f92009e853b6b045, RSA-4096, 2019-11-22 .. 2034-11-18.
    const ROOT_RSA_2019: &str = "\
-----BEGIN CERTIFICATE-----
MIIFHDCCAwSgAwIBAgIJANUP8luj8tazMA0GCSqGSIb3DQEBCwUAMBsxGTAXBgNV
BAUTEGY5MjAwOWU4NTNiNmIwNDUwHhcNMTkxMTIyMjAzNzU4WhcNMzQxMTE4MjAz
NzU4WjAbMRkwFwYDVQQFExBmOTIwMDllODUzYjZiMDQ1MIICIjANBgkqhkiG9w0B
AQEFAAOCAg8AMIICCgKCAgEAr7bHgiuxpwHsK7Qui8xUFmOr75gvMsd/dTEDDJdS
Sxtf6An7xyqpRR90PL2abxM1dEqlXnf2tqw1Ne4Xwl5jlRfdnJLmN0pTy/4lj4/7
tv0Sk3iiKkypnEUtR6WfMgH0QZfKHM1+di+y9TFRtv6y//0rb+T+W8a9nsNL/ggj
nar86461qO0rOs2cXjp3kOG1FEJ5MVmFmBGtnrKpa73XpXyTqRxB/M0n1n/W9nGq
C4FSYa04T6N5RIZGBN2z2MT5IKGbFlbC8UrW0DxW7AYImQQcHtGl/m00QLVWutHQ
oVJYnFPlXTcHYvASLu+RhhsbDmxMgJJ0mcDpvsC4PjvB+TxywElgS70vE0XmLD+O
JtvsBslHZvPBKCOdT0MS+tgSOIfga+z1Z1g7+DVagf7quvmag8jfPioyKvxnK/Eg
sTUVi2ghzq8wm27ud/mIM7AY2qEORR8Go3TVB4HzWQgpZrt3i5MIlCaY504LzSRi
igHCzAPlHws+W0rB5N+er5/2pJKnfBSDiCiFAVtCLOZ7gLiMm0jhO2B6tUXHI/+M
RPjy02i59lINMRRev56GKtcd9qO/0kUJWdZTdA2XoS82ixPvZtXQpUpuL12ab+9E
aDK8Z4RHJYYfCT3Q5vNAXaiWQ+8PTWm2QgBR/bkwSWc+NpUFgNPN9PvQi8WEg5Um
AGMCAwEAAaNjMGEwHQYDVR0OBBYEFDZh4QB8iAUJUYtEbEf/GkzJ6k8SMB8GA1Ud
IwQYMBaAFDZh4QB8iAUJUYtEbEf/GkzJ6k8SMA8GA1UdEwEB/wQFMAMBAf8wDgYD
VR0PAQH/BAQDAgIEMA0GCSqGSIb3DQEBCwUAA4ICAQBOMaBc8oumXb2voc7XCWnu
XKhBBK3e2KMGz39t7lA3XXRe2ZLLAkLM5y3J7tURkf5a1SutfdOyXAmeE6SRo83U
h6WszodmMkxK5GM4JGrnt4pBisu5igXEydaW7qq2CdC6DOGjG+mEkN8/TA6p3cno
L/sPyz6evdjLlSeJ8rFBH6xWyIZCbrcpYEJzXaUOEaxxXxgYz5/cTiVKN2M1G2ok
QBUIYSY6bjEL4aUN5cfo7ogP3UvliEo3Eo0YgwuzR2v0KR6C1cZqZJSTnghIC/vA
D32KdNQ+c3N+vl2OTsUVMC1GiWkngNx1OO1+kXW+YTnnTUOtOIswUP/Vqd5SYgAI
mMAfY8U9/iIgkQj6T2W6FsScy94IN9fFhE1UtzmLoBIuUFsVXJMTz+Jucth+IqoW
Fua9v1R93/k98p41pjtFX+H8DslVgfP097vju4KDlqN64xV1grw3ZLl4CiOe/A91
oeLm2UHOq6wn3esB4r2EIQKb6jTVGu5sYCcdWpXr0AUVqcABPdgL+H7qJguBw09o
jm6xNIrw2OocrDKsudk/okr/AwqEyPKw9WnMlQgLIKw1rODG2NvU9oR3GVGdMkUB
ZutL8VuFkERQGt6vQ2OCw0sV47VMkuYbacK/xyZFiRcrPJPb41zgbQj9XAEyLKCH
ex0SdDrx+tWUDqG8At2JHA==
-----END CERTIFICATE-----
";

    /// Previously issued. Subject serialNumber=f92009e853b6b045, RSA-4096, 2021-11-17 .. 2036-11-13.
    const ROOT_RSA_2021: &str = "\
-----BEGIN CERTIFICATE-----
MIIFHDCCAwSgAwIBAgIJAMNrfES5rhgxMA0GCSqGSIb3DQEBCwUAMBsxGTAXBgNV
BAUTEGY5MjAwOWU4NTNiNmIwNDUwHhcNMjExMTE3MjMxMDQyWhcNMzYxMTEzMjMx
MDQyWjAbMRkwFwYDVQQFExBmOTIwMDllODUzYjZiMDQ1MIICIjANBgkqhkiG9w0B
AQEFAAOCAg8AMIICCgKCAgEAr7bHgiuxpwHsK7Qui8xUFmOr75gvMsd/dTEDDJdS
Sxtf6An7xyqpRR90PL2abxM1dEqlXnf2tqw1Ne4Xwl5jlRfdnJLmN0pTy/4lj4/7
tv0Sk3iiKkypnEUtR6WfMgH0QZfKHM1+di+y9TFRtv6y//0rb+T+W8a9nsNL/ggj
nar86461qO0rOs2cXjp3kOG1FEJ5MVmFmBGtnrKpa73XpXyTqRxB/M0n1n/W9nGq
C4FSYa04T6N5RIZGBN2z2MT5IKGbFlbC8UrW0DxW7AYImQQcHtGl/m00QLVWutHQ
oVJYnFPlXTcHYvASLu+RhhsbDmxMgJJ0mcDpvsC4PjvB+TxywElgS70vE0XmLD+O
JtvsBslHZvPBKCOdT0MS+tgSOIfga+z1Z1g7+DVagf7quvmag8jfPioyKvxnK/Eg
sTUVi2ghzq8wm27ud/mIM7AY2qEORR8Go3TVB4HzWQgpZrt3i5MIlCaY504LzSRi
igHCzAPlHws+W0rB5N+er5/2pJKnfBSDiCiFAVtCLOZ7gLiMm0jhO2B6tUXHI/+M
RPjy02i59lINMRRev56GKtcd9qO/0kUJWdZTdA2XoS82ixPvZtXQpUpuL12ab+9E
aDK8Z4RHJYYfCT3Q5vNAXaiWQ+8PTWm2QgBR/bkwSWc+NpUFgNPN9PvQi8WEg5Um
AGMCAwEAAaNjMGEwHQYDVR0OBBYEFDZh4QB8iAUJUYtEbEf/GkzJ6k8SMB8GA1Ud
IwQYMBaAFDZh4QB8iAUJUYtEbEf/GkzJ6k8SMA8GA1UdEwEB/wQFMAMBAf8wDgYD
VR0PAQH/BAQDAgIEMA0GCSqGSIb3DQEBCwUAA4ICAQBTNNZe5cuf8oiq+jV0itTG
zWVhSTjOBEk2FQvh11J3o3lna0o7rd8RFHnN00q4hi6TapFhh4qaw/iG6Xg+xOan
63niLWIC5GOPFgPeYXM9+nBb3zZzC8ABypYuCusWCmt6Tn3+Pjbz3MTVhRGXuT/T
QH4KGFY4PhvzAyXwdjTOCXID+aHud4RLcSySr0Fq/L+R8TWalvM1wJJPhyRjqRCJ
erGtfBagiALzvhnmY7U1qFcS0NCnKjoO7oFedKdWlZz0YAfu3aGCJd4KHT0MsGiL
Zez9WP81xYSrKMNEsDK+zK5fVzw6jA7cxmpXcARTnmAuGUeI7VVDhDzKeVOctf3a
0qQLwC+d0+xrETZ4r2fRGNw2YEs2W8Qj6oDcfPvq9JySe7pJ6wcHnl5EZ0lwc4xH
7Y4Dx9RA1JlfooLMw3tOdJZH0enxPXaydfAD3YifeZpFaUzicHeLzVJLt9dvGB0b
HQLE4+EqKFgOZv2EoP686DQqbVS1u+9k0p2xbMA105TBIk7npraa8VM0fnrRKi7w
lZKwdH+aNAyhbXRW9xsnODJ+g8eF452zvbiKKngEKirK5LGieoXBX7tZ9D1GNBH2
Ob3bKOwwIWdEFle/YF/h6zWgdeoaNGDqVBrLr2+0DtWoiB1aDEjLWl9FmyIUyUm7
mD/vFDkzF+wm7cyWpQpCVQ==
-----END CERTIFICATE-----
";

    pub(super) const GOOGLE_ROOTS_PEM: [&str; 5] = [
        ROOT_RSA_2022,
        ROOT_EC_CA1_2025,
        ROOT_RSA_2016,
        ROOT_RSA_2019,
        ROOT_RSA_2021,
    ];

    pub(super) fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
        let body: String = pem
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("-----"))
            .collect();
        base64::engine::general_purpose::STANDARD.decode(body).ok()
    }

    /// SubjectPublicKeyInfo DER of every bundled root.
    pub(super) fn google_root_spkis() -> Vec<Vec<u8>> {
        GOOGLE_ROOTS_PEM
            .iter()
            .filter_map(|pem| pem_to_der(pem))
            .filter_map(|der| parse_cert(&der).map(|c| c.spki.to_vec()))
            .collect()
    }

    // ---------------------------------------------------------------------
    // Entry point and result mapping.
    // ---------------------------------------------------------------------

    pub(super) fn evaluate(chain: &[Vec<u8>]) -> IntegrityStatus {
        let Some(leaf_der) = chain.first() else {
            return IntegrityStatus::Unknown;
        };
        let Some(leaf) = parse_cert(leaf_der) else {
            return IntegrityStatus::Unknown;
        };
        if !chain_is_trusted(chain) {
            return IntegrityStatus::Warning(MSG_UNVERIFIED.into());
        }
        // A Google-signed leaf without a well-formed key description (or
        // without a hardware-enforced root of trust) is a leaf parse failure.
        match leaf.key_description.and_then(parse_key_description) {
            Some(kd) => classify(&kd),
            None => IntegrityStatus::Unknown,
        }
    }

    pub(super) fn classify(kd: &KeyDescription) -> IntegrityStatus {
        if kd.attestation_security_level == SecurityLevel::Software {
            return IntegrityStatus::Warning(MSG_SOFTWARE.into());
        }
        let Some(rot) = &kd.root_of_trust else {
            return IntegrityStatus::Unknown;
        };
        if rot.verified_boot_state != VerifiedBootState::Verified {
            return IntegrityStatus::Warning(format!(
                "Verified boot state is {}",
                rot.verified_boot_state.name()
            ));
        }
        if !rot.device_locked {
            return IntegrityStatus::Warning(MSG_UNLOCKED.into());
        }
        IntegrityStatus::Ok
    }

    /// Every link verifies and the chain ends in a bundled Google root key.
    pub(super) fn chain_is_trusted(chain: &[Vec<u8>]) -> bool {
        if chain.is_empty() || chain.len() > MAX_CHAIN_LEN {
            return false;
        }
        let mut certs = Vec::with_capacity(chain.len());
        for der in chain {
            match parse_cert(der) {
                Some(c) => certs.push(c),
                None => return false,
            }
        }
        let Some(root) = certs.last() else {
            return false;
        };
        if !google_root_spkis()
            .iter()
            .any(|k| k.as_slice() == root.spki)
        {
            return false;
        }
        certs
            .windows(2)
            .all(|pair| verify_signature(&pair[0], pair[1].spki))
    }

    // ---------------------------------------------------------------------
    // Key description (attestation extension) model.
    // ---------------------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum SecurityLevel {
        Software,
        TrustedEnvironment,
        StrongBox,
        Other(i64),
    }

    impl SecurityLevel {
        fn from_i64(v: i64) -> Self {
            match v {
                0 => Self::Software,
                1 => Self::TrustedEnvironment,
                2 => Self::StrongBox,
                other => Self::Other(other),
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum VerifiedBootState {
        Verified,
        SelfSigned,
        Unverified,
        Failed,
        Other(i64),
    }

    impl VerifiedBootState {
        fn from_i64(v: i64) -> Self {
            match v {
                0 => Self::Verified,
                1 => Self::SelfSigned,
                2 => Self::Unverified,
                3 => Self::Failed,
                other => Self::Other(other),
            }
        }

        fn name(self) -> &'static str {
            match self {
                Self::Verified => "Verified",
                Self::SelfSigned => "SelfSigned",
                Self::Unverified => "Unverified",
                // Unknown values are treated as the worst case.
                Self::Failed | Self::Other(_) => "Failed",
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct RootOfTrust {
        pub verified_boot_key: Vec<u8>,
        pub device_locked: bool,
        pub verified_boot_state: VerifiedBootState,
        pub verified_boot_hash: Option<Vec<u8>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct KeyDescription {
        pub attestation_version: i64,
        pub attestation_security_level: SecurityLevel,
        pub keymint_version: i64,
        pub keymint_security_level: SecurityLevel,
        pub attestation_challenge: Vec<u8>,
        /// rootOfTrust from the hardware-enforced AuthorizationList only.
        pub root_of_trust: Option<RootOfTrust>,
    }

    /// Context-specific tag of `rootOfTrust` in an AuthorizationList.
    const TAG_ROOT_OF_TRUST: u32 = 704;

    /// KeyDescription ::= SEQUENCE {
    ///   attestationVersion INTEGER, attestationSecurityLevel SecurityLevel,
    ///   keyMintVersion INTEGER, keyMintSecurityLevel SecurityLevel,
    ///   attestationChallenge OCTET STRING, uniqueId OCTET STRING,
    ///   softwareEnforced AuthorizationList, hardwareEnforced AuthorizationList }
    pub(super) fn parse_key_description(ext_value: &[u8]) -> Option<KeyDescription> {
        let (seq, rest) = der::expect(ext_value, der::SEQUENCE)?;
        if !rest.is_empty() {
            return None;
        }
        let r = seq.value;
        let (v, r) = der::expect(r, der::INTEGER)?;
        let attestation_version = der::small_int(v.value)?;
        let (v, r) = der::expect(r, der::ENUMERATED)?;
        let attestation_security_level = SecurityLevel::from_i64(der::small_int(v.value)?);
        let (v, r) = der::expect(r, der::INTEGER)?;
        let keymint_version = der::small_int(v.value)?;
        let (v, r) = der::expect(r, der::ENUMERATED)?;
        let keymint_security_level = SecurityLevel::from_i64(der::small_int(v.value)?);
        let (challenge, r) = der::expect(r, der::OCTET_STRING)?;
        let (_unique_id, r) = der::expect(r, der::OCTET_STRING)?;
        let (_software, r) = der::expect(r, der::SEQUENCE)?;
        let (hardware, _future_fields) = der::expect(r, der::SEQUENCE)?;

        let mut root_of_trust = None;
        let mut items = hardware.value;
        while !items.is_empty() {
            let (item, next) = der::read(items)?;
            items = next;
            if item.class == der::CLASS_CONTEXT && item.tag == TAG_ROOT_OF_TRUST {
                if !item.constructed || root_of_trust.is_some() {
                    return None;
                }
                root_of_trust = Some(parse_root_of_trust(item.value)?);
            }
        }

        Some(KeyDescription {
            attestation_version,
            attestation_security_level,
            keymint_version,
            keymint_security_level,
            attestation_challenge: challenge.value.to_vec(),
            root_of_trust,
        })
    }

    /// RootOfTrust ::= SEQUENCE { verifiedBootKey OCTET STRING,
    ///   deviceLocked BOOLEAN, verifiedBootState VerifiedBootState,
    ///   verifiedBootHash OCTET STRING (attestation v3+) }
    fn parse_root_of_trust(explicit: &[u8]) -> Option<RootOfTrust> {
        let (seq, rest) = der::expect(explicit, der::SEQUENCE)?;
        if !rest.is_empty() {
            return None;
        }
        let (key, r) = der::expect(seq.value, der::OCTET_STRING)?;
        let (locked, r) = der::expect(r, der::BOOLEAN)?;
        let device_locked = match locked.value {
            [b] => *b != 0,
            _ => return None,
        };
        let (state, r) = der::expect(r, der::ENUMERATED)?;
        let verified_boot_state = VerifiedBootState::from_i64(der::small_int(state.value)?);
        let verified_boot_hash = if r.is_empty() {
            None
        } else {
            Some(der::expect(r, der::OCTET_STRING)?.0.value.to_vec())
        };
        Some(RootOfTrust {
            verified_boot_key: key.value.to_vec(),
            device_locked,
            verified_boot_state,
            verified_boot_hash,
        })
    }

    // ---------------------------------------------------------------------
    // X.509 certificate (structure only, RFC 5280 section 4.1).
    // ---------------------------------------------------------------------

    pub(super) struct Cert<'a> {
        /// Full TLV of tbsCertificate: the signed bytes.
        pub tbs: &'a [u8],
        /// Contents of the outer signatureAlgorithm AlgorithmIdentifier.
        pub sig_alg: &'a [u8],
        pub signature: &'a [u8],
        /// Full TLV of subjectPublicKeyInfo.
        pub spki: &'a [u8],
        /// Contents of the key description extension's OCTET STRING.
        pub key_description: Option<&'a [u8]>,
    }

    /// DER encoding (contents) of 1.3.6.1.4.1.11129.2.1.17.
    const OID_KEY_DESCRIPTION: &[u8] =
        &[0x2b, 0x06, 0x01, 0x04, 0x01, 0xd6, 0x79, 0x02, 0x01, 0x11];

    pub(super) fn parse_cert(input: &[u8]) -> Option<Cert<'_>> {
        let (cert, rest) = der::expect(input, der::SEQUENCE)?;
        if !rest.is_empty() {
            return None;
        }
        let (tbs, r) = der::expect(cert.value, der::SEQUENCE)?;
        let (sig_alg, r) = der::expect(r, der::SEQUENCE)?;
        let (sig, r) = der::expect(r, der::BIT_STRING)?;
        if !r.is_empty() {
            return None;
        }
        let signature = der::bit_string_bytes(sig.value)?;

        // tbsCertificate
        let mut r = tbs.value;
        if let Some((version, next)) = der::expect(r, der::context(0, true)) {
            der::expect(version.value, der::INTEGER)?;
            r = next;
        }
        let (_serial, r) = der::expect(r, der::INTEGER)?;
        let (inner_alg, r) = der::expect(r, der::SEQUENCE)?;
        if inner_alg.value != sig_alg.value {
            return None;
        }
        let (_issuer, r) = der::expect(r, der::SEQUENCE)?;
        let (_validity, r) = der::expect(r, der::SEQUENCE)?;
        let (_subject, r) = der::expect(r, der::SEQUENCE)?;
        // The leaf's own key type is irrelevant (it never verifies anything);
        // issuer keys are decoded in `verify_signature`.
        let (spki, mut r) = der::expect(r, der::SEQUENCE)?;

        let mut key_description = None;
        while !r.is_empty() {
            let (field, next) = der::read(r)?;
            r = next;
            if field.class != der::CLASS_CONTEXT {
                return None;
            }
            match field.tag {
                // issuerUniqueID / subjectUniqueID: ignored.
                1 | 2 => {}
                3 if field.constructed => {
                    key_description = find_key_description(field.value)?;
                    if !r.is_empty() {
                        return None;
                    }
                }
                _ => return None,
            }
        }

        Some(Cert {
            tbs: tbs.raw,
            sig_alg: sig_alg.value,
            signature,
            spki: spki.raw,
            key_description,
        })
    }

    /// `[3] EXPLICIT Extensions`: returns `Some(None)` when the attestation
    /// extension is absent, `None` on malformed or duplicated extensions.
    fn find_key_description(explicit: &[u8]) -> Option<Option<&[u8]>> {
        let (exts, rest) = der::expect(explicit, der::SEQUENCE)?;
        if !rest.is_empty() {
            return None;
        }
        let mut found = None;
        let mut r = exts.value;
        while !r.is_empty() {
            let (ext, next) = der::expect(r, der::SEQUENCE)?;
            r = next;
            let (oid, e) = der::expect(ext.value, der::OID)?;
            let e = match der::expect(e, der::BOOLEAN) {
                Some((_critical, after)) => after,
                None => e,
            };
            let (value, e) = der::expect(e, der::OCTET_STRING)?;
            if !e.is_empty() {
                return None;
            }
            if oid.value == OID_KEY_DESCRIPTION {
                if found.is_some() {
                    return None;
                }
                found = Some(value.value);
            }
        }
        Some(found)
    }

    // ---------------------------------------------------------------------
    // Public keys and signatures.
    // ---------------------------------------------------------------------

    const OID_EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
    const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
    const OID_P384: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x22];
    const OID_RSA_ENCRYPTION: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
    const OID_ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
    const OID_ECDSA_SHA384: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x03];
    const OID_ECDSA_SHA512: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x04];
    const OID_RSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
    const OID_RSA_SHA384: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0c];
    const OID_RSA_SHA512: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0d];

    enum PublicKey<'a> {
        P256(&'a [u8]),
        P384(&'a [u8]),
        /// PKCS#1 RSAPublicKey DER.
        Rsa(&'a [u8]),
    }

    #[derive(Clone, Copy)]
    enum Hash {
        Sha256,
        Sha384,
        Sha512,
    }

    impl Hash {
        fn digest(self, msg: &[u8]) -> Vec<u8> {
            match self {
                Hash::Sha256 => sha2::Sha256::digest(msg).to_vec(),
                Hash::Sha384 => sha2::Sha384::digest(msg).to_vec(),
                Hash::Sha512 => sha2::Sha512::digest(msg).to_vec(),
            }
        }
    }

    enum SigAlg {
        Ecdsa(Hash),
        RsaPkcs1v15(Hash),
    }

    /// SubjectPublicKeyInfo ::= SEQUENCE { AlgorithmIdentifier, BIT STRING }
    fn parse_spki(spki: &[u8]) -> Option<PublicKey<'_>> {
        let (seq, rest) = der::expect(spki, der::SEQUENCE)?;
        if !rest.is_empty() {
            return None;
        }
        let (alg, r) = der::expect(seq.value, der::SEQUENCE)?;
        let (bits, r) = der::expect(r, der::BIT_STRING)?;
        if !r.is_empty() {
            return None;
        }
        let key = der::bit_string_bytes(bits.value)?;
        let (oid, params) = der::expect(alg.value, der::OID)?;
        if oid.value == OID_EC_PUBLIC_KEY {
            let (curve, r) = der::expect(params, der::OID)?;
            if !r.is_empty() {
                return None;
            }
            match curve.value {
                OID_P256 => Some(PublicKey::P256(key)),
                OID_P384 => Some(PublicKey::P384(key)),
                _ => None,
            }
        } else if oid.value == OID_RSA_ENCRYPTION {
            if !params.is_empty() && params != [0x05, 0x00] {
                return None;
            }
            Some(PublicKey::Rsa(key))
        } else {
            None
        }
    }

    fn parse_sig_alg(alg: &[u8]) -> Option<SigAlg> {
        let (oid, params) = der::expect(alg, der::OID)?;
        let rsa_params_ok = params.is_empty() || params == [0x05, 0x00];
        match oid.value {
            OID_ECDSA_SHA256 if params.is_empty() => Some(SigAlg::Ecdsa(Hash::Sha256)),
            OID_ECDSA_SHA384 if params.is_empty() => Some(SigAlg::Ecdsa(Hash::Sha384)),
            OID_ECDSA_SHA512 if params.is_empty() => Some(SigAlg::Ecdsa(Hash::Sha512)),
            OID_RSA_SHA256 if rsa_params_ok => Some(SigAlg::RsaPkcs1v15(Hash::Sha256)),
            OID_RSA_SHA384 if rsa_params_ok => Some(SigAlg::RsaPkcs1v15(Hash::Sha384)),
            OID_RSA_SHA512 if rsa_params_ok => Some(SigAlg::RsaPkcs1v15(Hash::Sha512)),
            _ => None,
        }
    }

    /// Checks `cert`'s signature with the issuer's SubjectPublicKeyInfo.
    pub(super) fn verify_signature(cert: &Cert<'_>, issuer_spki: &[u8]) -> bool {
        let (Some(alg), Some(key)) = (parse_sig_alg(cert.sig_alg), parse_spki(issuer_spki)) else {
            return false;
        };
        match (alg, key) {
            (SigAlg::Ecdsa(hash), PublicKey::P256(point)) => {
                let digest = hash.digest(cert.tbs);
                let (Ok(vk), Ok(sig)) = (
                    p256::ecdsa::VerifyingKey::from_sec1_bytes(point),
                    p256::ecdsa::Signature::from_der(cert.signature),
                ) else {
                    return false;
                };
                vk.verify_prehash(&digest, &sig).is_ok()
            }
            (SigAlg::Ecdsa(hash), PublicKey::P384(point)) => {
                let digest = hash.digest(cert.tbs);
                let (Ok(vk), Ok(sig)) = (
                    p384::ecdsa::VerifyingKey::from_sec1_bytes(point),
                    p384::ecdsa::Signature::from_der(cert.signature),
                ) else {
                    return false;
                };
                vk.verify_prehash(&digest, &sig).is_ok()
            }
            (SigAlg::RsaPkcs1v15(hash), PublicKey::Rsa(pkcs1)) => {
                let Ok(pk) = rsa::RsaPublicKey::from_pkcs1_der(pkcs1) else {
                    return false;
                };
                let digest = hash.digest(cert.tbs);
                let scheme = match hash {
                    Hash::Sha256 => rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
                    Hash::Sha384 => rsa::Pkcs1v15Sign::new::<sha2::Sha384>(),
                    Hash::Sha512 => rsa::Pkcs1v15Sign::new::<sha2::Sha512>(),
                };
                pk.verify(scheme, &digest, cert.signature).is_ok()
            }
            _ => false,
        }
    }

    // ---------------------------------------------------------------------
    // Minimal defensive DER reader: bounds-checked slicing only, no recursion,
    // no indefinite lengths, no panics on any input.
    // ---------------------------------------------------------------------

    mod der {
        pub const CLASS_UNIVERSAL: u8 = 0;
        pub const CLASS_CONTEXT: u8 = 2;

        /// (class, constructed, tag number)
        pub type Tag = (u8, bool, u32);
        pub const BOOLEAN: Tag = (CLASS_UNIVERSAL, false, 1);
        pub const INTEGER: Tag = (CLASS_UNIVERSAL, false, 2);
        pub const BIT_STRING: Tag = (CLASS_UNIVERSAL, false, 3);
        pub const OCTET_STRING: Tag = (CLASS_UNIVERSAL, false, 4);
        pub const OID: Tag = (CLASS_UNIVERSAL, false, 6);
        pub const ENUMERATED: Tag = (CLASS_UNIVERSAL, false, 10);
        pub const SEQUENCE: Tag = (CLASS_UNIVERSAL, true, 16);

        pub const fn context(tag: u32, constructed: bool) -> Tag {
            (CLASS_CONTEXT, constructed, tag)
        }

        pub struct Tlv<'a> {
            pub class: u8,
            pub constructed: bool,
            pub tag: u32,
            pub value: &'a [u8],
            /// Header plus value.
            pub raw: &'a [u8],
        }

        /// Reads one TLV, returning it and the remaining input.
        pub fn read(input: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
            let (&first, mut rest) = input.split_first()?;
            let class = first >> 6;
            let constructed = first & 0x20 != 0;
            let mut tag = u32::from(first & 0x1f);
            if tag == 0x1f {
                // High tag number form, base-128; cap at 4 bytes (28 bits).
                tag = 0;
                let mut count = 0;
                loop {
                    let (&b, r) = rest.split_first()?;
                    rest = r;
                    count += 1;
                    if count > 4 {
                        return None;
                    }
                    tag = (tag << 7) | u32::from(b & 0x7f);
                    if b & 0x80 == 0 {
                        break;
                    }
                }
            }
            let (&len0, r) = rest.split_first()?;
            rest = r;
            let len = if len0 < 0x80 {
                usize::from(len0)
            } else {
                let n = usize::from(len0 & 0x7f);
                // 0x80 is indefinite length (BER only); > 4 bytes is absurd.
                if n == 0 || n > 4 {
                    return None;
                }
                let bytes = rest.get(..n)?;
                rest = rest.get(n..)?;
                let mut len: usize = 0;
                for &b in bytes {
                    len = len.checked_mul(256)?.checked_add(usize::from(b))?;
                }
                len
            };
            let value = rest.get(..len)?;
            let after = rest.get(len..)?;
            let header = input.len().checked_sub(rest.len())?;
            let raw = input.get(..header.checked_add(len)?)?;
            Some((
                Tlv {
                    class,
                    constructed,
                    tag,
                    value,
                    raw,
                },
                after,
            ))
        }

        /// Reads one TLV and checks its tag.
        pub fn expect(input: &[u8], (class, constructed, tag): Tag) -> Option<(Tlv<'_>, &[u8])> {
            let (tlv, rest) = read(input)?;
            (tlv.class == class && tlv.constructed == constructed && tlv.tag == tag)
                .then_some((tlv, rest))
        }

        /// BIT STRING contents with zero unused bits.
        pub fn bit_string_bytes(value: &[u8]) -> Option<&[u8]> {
            match value.split_first()? {
                (0, bytes) => Some(bytes),
                _ => None,
            }
        }

        /// Two's-complement INTEGER / ENUMERATED of at most 8 bytes.
        pub fn small_int(value: &[u8]) -> Option<i64> {
            if value.is_empty() || value.len() > 8 {
                return None;
            }
            let negative = value[0] & 0x80 != 0;
            let mut v: i64 = if negative { -1 } else { 0 };
            for &b in value {
                v = (v << 8) | i64::from(b);
            }
            Some(v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chain_is_unknown() {
        assert_eq!(verify_chain(&[]), IntegrityStatus::Unknown);
    }

    #[test]
    fn garbage_is_never_ok() {
        let status = verify_chain(&[vec![0x30, 0x03, 0x02, 0x01, 0x00]]);
        assert_ne!(status, IntegrityStatus::Ok);
    }
}

/// Test vectors.
///
/// Real chains from Google's `android-key-attestation` repository
/// (Apache-2.0), fetched 2026-10-11 from
/// <https://github.com/google/android-key-attestation/tree/master/src/test/resources/pem>
/// (`algorithm_{EC,RSA}_SecurityLevel_{TEE,StrongBox}/cert{0..3}.pem`, leaf is
/// cert0). They were generated with TestDPC on an unlocked development device:
/// attestation v3, KeyMaster 4, deviceLocked = false, verifiedBootState =
/// Unverified (confirmed with `openssl asn1parse`). The TEE chains end in the
/// 2016 Google RSA root; the StrongBox chains end in a non-Google root
/// (serialNumber=e35d38c6897d47e8), and the EC StrongBox leaf is not signed
/// by its cert1 (`openssl verify` reports a signature failure).
///
/// `FORGED_SELF_SIGNED` was generated locally with `openssl req -x509` (P-256,
/// subject copied from the Google root) and carries a hand-built key
/// description claiming TEE, Verified and locked.
#[cfg(all(test, feature = "integrity"))]
mod vector_tests {
    use super::imp::*;
    use super::*;
    use proptest::prelude::*;

    fn der(pem: &str) -> Vec<u8> {
        pem_to_der(pem).expect("test PEM decodes")
    }

    fn chain(pems: &[&str]) -> Vec<Vec<u8>> {
        pems.iter().map(|p| der(p)).collect()
    }

    fn ec_tee() -> Vec<Vec<u8>> {
        chain(&[EC_TEE_0, EC_TEE_1, EC_TEE_2, EC_TEE_3])
    }

    fn rsa_tee() -> Vec<Vec<u8>> {
        chain(&[RSA_TEE_0, RSA_TEE_1, RSA_TEE_2, RSA_TEE_3])
    }

    fn rsa_strongbox() -> Vec<Vec<u8>> {
        chain(&[
            RSA_STRONGBOX_0,
            RSA_STRONGBOX_1,
            RSA_STRONGBOX_2,
            RSA_STRONGBOX_3,
        ])
    }

    fn ec_strongbox() -> Vec<Vec<u8>> {
        chain(&[
            EC_STRONGBOX_0,
            EC_STRONGBOX_1,
            EC_STRONGBOX_2,
            EC_STRONGBOX_3,
        ])
    }

    fn unverified() -> IntegrityStatus {
        IntegrityStatus::Warning(MSG_UNVERIFIED.into())
    }

    #[test]
    fn bundled_roots_parse_and_self_verify() {
        for pem in GOOGLE_ROOTS_PEM {
            let der = der(pem);
            let cert = parse_cert(&der).expect("root parses");
            assert!(verify_signature(&cert, cert.spki), "root self-signature");
        }
        // Four RSA certificates share one key, plus the P-384 CA1 key.
        let mut spkis = google_root_spkis();
        assert_eq!(spkis.len(), 5);
        spkis.sort();
        spkis.dedup();
        assert_eq!(spkis.len(), 2);
    }

    #[test]
    fn genuine_ec_chain_verifies_to_google_root() {
        assert!(chain_is_trusted(&ec_tee()));
    }

    #[test]
    fn genuine_rsa_chain_verifies_to_google_root() {
        assert!(chain_is_trusted(&rsa_tee()));
    }

    #[test]
    fn genuine_chain_parsed_values() {
        for c in [ec_tee(), rsa_tee()] {
            let leaf = parse_cert(&c[0]).expect("leaf parses");
            let kd = parse_key_description(leaf.key_description.expect("has extension"))
                .expect("key description parses");
            assert_eq!(kd.attestation_version, 3);
            assert_eq!(
                kd.attestation_security_level,
                SecurityLevel::TrustedEnvironment
            );
            assert_eq!(kd.keymint_version, 4);
            assert_eq!(kd.keymint_security_level, SecurityLevel::TrustedEnvironment);
            assert_eq!(kd.attestation_challenge, b"abc");
            let rot = kd.root_of_trust.expect("hardware root of trust");
            assert_eq!(rot.verified_boot_key, vec![0u8; 32]);
            assert!(!rot.device_locked);
            assert_eq!(rot.verified_boot_state, VerifiedBootState::Unverified);
            assert_eq!(rot.verified_boot_hash.map(|h| h.len()), Some(32));
        }
        let sb = rsa_strongbox();
        let leaf = parse_cert(&sb[0]).expect("leaf parses");
        let kd = parse_key_description(leaf.key_description.expect("has extension"))
            .expect("key description parses");
        assert_eq!(kd.attestation_security_level, SecurityLevel::StrongBox);
    }

    #[test]
    fn genuine_dev_device_chain_reports_boot_state() {
        let expected = IntegrityStatus::Warning("Verified boot state is Unverified".into());
        assert_eq!(verify_chain(&ec_tee()), expected);
        assert_eq!(verify_chain(&rsa_tee()), expected);
    }

    #[test]
    fn non_google_root_is_unverified() {
        let c = rsa_strongbox();
        // The links themselves are valid; only the anchor is unknown.
        let certs: Vec<_> = c.iter().map(|d| parse_cert(d).expect("parses")).collect();
        assert!(certs.windows(2).all(|p| verify_signature(&p[0], p[1].spki)));
        assert_eq!(verify_chain(&c), unverified());
    }

    #[test]
    fn broken_link_is_unverified() {
        assert_eq!(verify_chain(&ec_strongbox()), unverified());
        // Leaf of one chain spliced onto another device's intermediates.
        let mut spliced = rsa_tee();
        spliced[0] = ec_tee()[0].clone();
        assert_eq!(verify_chain(&spliced), unverified());
    }

    #[test]
    fn truncated_chain_is_unverified() {
        let c = ec_tee();
        for len in 1..c.len() {
            assert_eq!(verify_chain(&c[..len]), unverified(), "len {len}");
        }
        // Dropping an intermediate breaks the signature chain.
        let skipped = vec![c[0].clone(), c[2].clone(), c[3].clone()];
        assert_eq!(verify_chain(&skipped), unverified());
    }

    #[test]
    fn tampered_leaf_never_ok() {
        let genuine = ec_tee();
        for i in 0..genuine[0].len() {
            let mut c = genuine.clone();
            c[0][i] ^= 0x01;
            let status = verify_chain(&c);
            assert!(
                status == unverified() || status == IntegrityStatus::Unknown,
                "byte {i}: {status:?}"
            );
        }
    }

    #[test]
    fn forged_self_signed_is_unverified() {
        let forged = der(FORGED_SELF_SIGNED);
        let cert = parse_cert(&forged).expect("forged cert parses");
        // Its own P-256 signature is fine and the forged record claims a
        // perfect device ...
        assert!(verify_signature(&cert, cert.spki));
        let kd = parse_key_description(cert.key_description.expect("extension"))
            .expect("forged key description parses");
        assert_eq!(classify(&kd), IntegrityStatus::Ok);
        // ... but it does not chain to Google.
        assert_eq!(verify_chain(std::slice::from_ref(&forged)), unverified());
        let with_google_root = vec![forged, der(GOOGLE_ROOTS_PEM[0])];
        assert_eq!(verify_chain(&with_google_root), unverified());
    }

    #[test]
    fn classify_mapping() {
        let base = KeyDescription {
            attestation_version: 300,
            attestation_security_level: SecurityLevel::TrustedEnvironment,
            keymint_version: 300,
            keymint_security_level: SecurityLevel::TrustedEnvironment,
            attestation_challenge: vec![],
            root_of_trust: Some(RootOfTrust {
                verified_boot_key: vec![1; 32],
                device_locked: true,
                verified_boot_state: VerifiedBootState::Verified,
                verified_boot_hash: None,
            }),
        };
        assert_eq!(classify(&base), IntegrityStatus::Ok);

        let mut sw = base.clone();
        sw.attestation_security_level = SecurityLevel::Software;
        assert_eq!(classify(&sw), IntegrityStatus::Warning(MSG_SOFTWARE.into()));

        for (state, name) in [
            (VerifiedBootState::SelfSigned, "SelfSigned"),
            (VerifiedBootState::Unverified, "Unverified"),
            (VerifiedBootState::Failed, "Failed"),
            (VerifiedBootState::Other(9), "Failed"),
        ] {
            let mut kd = base.clone();
            if let Some(rot) = kd.root_of_trust.as_mut() {
                rot.verified_boot_state = state;
            }
            assert_eq!(
                classify(&kd),
                IntegrityStatus::Warning(format!("Verified boot state is {name}"))
            );
        }

        let mut unlocked = base.clone();
        if let Some(rot) = unlocked.root_of_trust.as_mut() {
            rot.device_locked = false;
        }
        assert_eq!(
            classify(&unlocked),
            IntegrityStatus::Warning(MSG_UNLOCKED.into())
        );

        let mut missing = base;
        missing.root_of_trust = None;
        assert_eq!(classify(&missing), IntegrityStatus::Unknown);
    }

    #[test]
    fn oversized_chain_is_unverified() {
        let c = ec_tee();
        let long: Vec<Vec<u8>> = (0..20).map(|_| c[0].clone()).collect();
        assert_eq!(verify_chain(&long), unverified());
    }

    proptest! {
        #[test]
        fn never_panics_on_random_bytes(
            chain in proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 0..600), 0..5)
        ) {
            let _ = verify_chain(&chain);
        }

        #[test]
        fn never_panics_on_random_der_prefix(
            tail in proptest::collection::vec(any::<u8>(), 0..400),
            cut in 0usize..2000,
        ) {
            // Valid-looking prefix of a real leaf followed by noise.
            let leaf = ec_tee()[0].clone();
            let cut = cut.min(leaf.len());
            let mut bytes = leaf[..cut].to_vec();
            bytes.extend_from_slice(&tail);
            let _ = verify_chain(&[bytes]);
        }

        #[test]
        fn mutated_genuine_chain_never_ok(
            which in 0usize..4,
            pos in any::<prop::sample::Index>(),
            val in any::<u8>(),
        ) {
            let mut c = rsa_tee();
            let i = pos.index(c[which].len());
            prop_assume!(c[which][i] != val);
            c[which][i] = val;
            prop_assert_ne!(verify_chain(&c), IntegrityStatus::Ok);
        }
    }

    // Vectors (see module docs for provenance).

    const EC_TEE_0: &str = "\
    -----BEGIN CERTIFICATE-----
MIID7jCCA5SgAwIBAgIBATAKBggqhkjOPQQDAjApMRkwFwYDVQQFExAyZGM1OGIy
ZDFhMjQxMzI2MQwwCgYDVQQMDANURUUwIBcNNzAwMTAxMDAwMDAwWhgPMjEwNjAy
MDcwNjI4MTVaMB8xHTAbBgNVBAMMFEFuZHJvaWQgS2V5c3RvcmUgS2V5MFkwEwYH
KoZIzj0CAQYIKoZIzj0DAQcDQgAEHkyl3epGPODlaNT50JG1QK/DTFIz5vkasDfs
OMQiKlcrbKwmCTfFJqJcz6z/CKt6x5edTL66YxaQ430d0Is3JKOCArMwggKvMA4G
A1UdDwEB/wQEAwIHgDCCApsGCisGAQQB1nkCAREEggKLMIIChwIBAwoBAQIBBAoB
AQQDYWJjBAAwggHNv4U9CAIGAWTmEWf/v4VFggG7BIIBtzCCAbMxggGLMAwEB2Fu
ZHJvaWQCAR0wGQQUY29tLmFuZHJvaWQua2V5Y2hhaW4CAR0wGQQUY29tLmFuZHJv
aWQuc2V0dGluZ3MCAR0wGQQUY29tLnF0aS5kaWFnc2VydmljZXMCAR0wGgQVY29t
LmFuZHJvaWQuZHluc3lzdGVtAgEdMB0EGGNvbS5hbmRyb2lkLmlucHV0ZGV2aWNl
cwIBHTAfBBpjb20uYW5kcm9pZC5sb2NhbHRyYW5zcG9ydAIBHTAfBBpjb20uYW5k
cm9pZC5sb2NhdGlvbi5mdXNlZAIBHTAfBBpjb20uYW5kcm9pZC5zZXJ2ZXIudGVs
ZWNvbQIBHTAgBBtjb20uYW5kcm9pZC53YWxscGFwZXJiYWNrdXACAR0wIQQcY29t
Lmdvb2dsZS5TU1Jlc3RhcnREZXRlY3RvcgIBHTAiBB1jb20uZ29vZ2xlLmFuZHJv
aWQuaGlkZGVubWVudQIBATAjBB5jb20uYW5kcm9pZC5wcm92aWRlcnMuc2V0dGlu
Z3MCAR0xIgQgMBqjywgRNFAcRfFCKrxmwkIk/V3tX9yPF+aXF2/YZqowgaChCDEG
AgECAgEDogMCAQOjBAICAQClBTEDAgEEqgMCAQG/g3cCBQC/hT4DAgEAv4VATDBK
BCAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEBAAoBAgQgco2xJ08f
HPFXHeQ4CwSKVUrEo4Dnb1NVCDUpCEqTeAG/hUEDAgEAv4VCBQIDAxSzv4VOBQID
AxSzv4VPBQIDAxSzMAoGCCqGSM49BAMCA0gAMEUCIDsINbPXvgn8qN2V74vvO9Rc
uXe17dswxNkm1vyx1BqCAiEAkicGYwvPJp4jAIZbD9++D+kQJQSJZyE3kT9ukSeS
fHs=
-----END CERTIFICATE-----
";

    const EC_TEE_1: &str = "\
    -----BEGIN CERTIFICATE-----
MIICJjCCAaugAwIBAgIKEyBjEXiWOIIJETAKBggqhkjOPQQDAjApMRkwFwYDVQQF
ExAyOTYwY2I5YWViZDUwNWY0MQwwCgYDVQQMDANURUUwHhcNMTgwMzIxMjA1ODU4
WhcNMjgwMzE4MjA1ODU4WjApMRkwFwYDVQQFExAyZGM1OGIyZDFhMjQxMzI2MQww
CgYDVQQMDANURUUwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAATvMEBAHYIMvvup
iQYIRlRceTcA6ZARblK+YPHYI01TXd3qVfLImEdHSp2LiG+c8Xk4ORhOE9MfT/yu
kMlqDUSAo4G6MIG3MB0GA1UdDgQWBBT3sqQpymXuuJ9rHrXvzF6qRvAoJzAfBgNV
HSMEGDAWgBTZCjmyzP+wdzrAqIzS/zURX5DNxjAPBgNVHRMBAf8EBTADAQH/MA4G
A1UdDwEB/wQEAwICBDBUBgNVHR8ETTBLMEmgR6BFhkNodHRwczovL2FuZHJvaWQu
Z29vZ2xlYXBpcy5jb20vYXR0ZXN0YXRpb24vY3JsLzEzMjA2MzExNzg5NjM4ODIw
OTExMAoGCCqGSM49BAMCA2kAMGYCMQDWh8lKjA7mypbTxqgg9kGPSnVuSKPrW1aO
zO+ml3r1D1BJaYsTn+v0gPWaAyG09tQCMQCqP4ulQISn1qhfvcSKYtgATw90q811
e8LUvdfid9h6Ew15S9Q5Fs9x+lnnLKtGHoo=
-----END CERTIFICATE-----
";

    const EC_TEE_2: &str = "\
    -----BEGIN CERTIFICATE-----
MIID0TCCAbmgAwIBAgIKA4gmZ2BliZaFfTANBgkqhkiG9w0BAQsFADAbMRkwFwYD
VQQFExBmOTIwMDllODUzYjZiMDQ1MB4XDTE4MDMyMTIwNTM1M1oXDTI4MDMxODIw
NTM1M1owKTEZMBcGA1UEBRMQMjk2MGNiOWFlYmQ1MDVmNDEMMAoGA1UEDAwDVEVF
MHYwEAYHKoZIzj0CAQYFK4EEACIDYgAEjXELw7zFszCPjAS+Qn6zEFajdp6hD+53
zSpN7egUzWfkFEh+wcLa4k67Fv2tKuekmLIhr4xidPllElgiwAyeB33abMJ5BckS
X1ayj4lnpMVvYRovl/5ZHIKNIJMN/hC6o4G2MIGzMB0GA1UdDgQWBBTZCjmyzP+w
dzrAqIzS/zURX5DNxjAfBgNVHSMEGDAWgBQ2YeEAfIgFCVGLRGxH/xpMyepPEjAP
BgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwICBDBQBgNVHR8ESTBHMEWgQ6BB
hj9odHRwczovL2FuZHJvaWQuZ29vZ2xlYXBpcy5jb20vYXR0ZXN0YXRpb24vY3Js
L0U4RkExOTYzMTREMkZBMTgwDQYJKoZIhvcNAQELBQADggIBAJKrsCoJam9qc1Za
HeQWf2TAjHDDVylLuUvczTSuu8VhZJiU0+yKGbTNnA13n/dvn7rmMj8W6ObWL582
Se9MBBdKtAVeCG7qYFq+m6kYFIfL8sOuutKXG5eW+8VY4he0hu+5eZiS2e3mAZGY
F9JwQTAv0H//6+9eLm/+j+L5tr2X4b7H6k2h4YW5HTIzIbVMvp/ic/zmv+RvrmYy
Xliaz1xglRqrJ6EiFI9/KsYYsSSVgorlrXPyKOBeXGc2SFIVQmLwfW4gc9BC8V4q
BDcpFK1kxOfnkyXloVZeat4yuYFSIdtJq9FErMFHfp+pDjIKRGeeWWrPzkbELqtH
USeSktbqN229c+86YRfScxpx5iJ0Jp2YsHkiYDFs+0LOex2RNdOKErGKZ+8QbMtV
xUIQARTlhZkeLTLAWufFcclEbZkpoe6jo131VjAlUrkQfK6dVVMx+gLhOffLjH9E
mUOD8Y1an9DQCsZl1tpBN4WgInbRMlzBsKF17Z9BEW1f2A+Gn0mnznZoq6t0Oxsv
RJTEHEgswQh4Vkr77UYSdcwQ0flrpD9Oe6ObnB0VSSCVo1yhPC39R35Z5KDhiNS8
BkuPaJoUK8RBbkl5ay4O86LFfY9tdAoHT0A/2hmq/NZhz48K1C9AJGtS+0+zHwSa
tjWJ7rM/KXAhJ0Nln/nMaeteQy2f
-----END CERTIFICATE-----
";

    const EC_TEE_3: &str = "\
    -----BEGIN CERTIFICATE-----
MIIFYDCCA0igAwIBAgIJAOj6GWMU0voYMA0GCSqGSIb3DQEBCwUAMBsxGTAXBgNV
BAUTEGY5MjAwOWU4NTNiNmIwNDUwHhcNMTYwNTI2MTYyODUyWhcNMjYwNTI0MTYy
ODUyWjAbMRkwFwYDVQQFExBmOTIwMDllODUzYjZiMDQ1MIICIjANBgkqhkiG9w0B
AQEFAAOCAg8AMIICCgKCAgEAr7bHgiuxpwHsK7Qui8xUFmOr75gvMsd/dTEDDJdS
Sxtf6An7xyqpRR90PL2abxM1dEqlXnf2tqw1Ne4Xwl5jlRfdnJLmN0pTy/4lj4/7
tv0Sk3iiKkypnEUtR6WfMgH0QZfKHM1+di+y9TFRtv6y//0rb+T+W8a9nsNL/ggj
nar86461qO0rOs2cXjp3kOG1FEJ5MVmFmBGtnrKpa73XpXyTqRxB/M0n1n/W9nGq
C4FSYa04T6N5RIZGBN2z2MT5IKGbFlbC8UrW0DxW7AYImQQcHtGl/m00QLVWutHQ
oVJYnFPlXTcHYvASLu+RhhsbDmxMgJJ0mcDpvsC4PjvB+TxywElgS70vE0XmLD+O
JtvsBslHZvPBKCOdT0MS+tgSOIfga+z1Z1g7+DVagf7quvmag8jfPioyKvxnK/Eg
sTUVi2ghzq8wm27ud/mIM7AY2qEORR8Go3TVB4HzWQgpZrt3i5MIlCaY504LzSRi
igHCzAPlHws+W0rB5N+er5/2pJKnfBSDiCiFAVtCLOZ7gLiMm0jhO2B6tUXHI/+M
RPjy02i59lINMRRev56GKtcd9qO/0kUJWdZTdA2XoS82ixPvZtXQpUpuL12ab+9E
aDK8Z4RHJYYfCT3Q5vNAXaiWQ+8PTWm2QgBR/bkwSWc+NpUFgNPN9PvQi8WEg5Um
AGMCAwEAAaOBpjCBozAdBgNVHQ4EFgQUNmHhAHyIBQlRi0RsR/8aTMnqTxIwHwYD
VR0jBBgwFoAUNmHhAHyIBQlRi0RsR/8aTMnqTxIwDwYDVR0TAQH/BAUwAwEB/zAO
BgNVHQ8BAf8EBAMCAYYwQAYDVR0fBDkwNzA1oDOgMYYvaHR0cHM6Ly9hbmRyb2lk
Lmdvb2dsZWFwaXMuY29tL2F0dGVzdGF0aW9uL2NybC8wDQYJKoZIhvcNAQELBQAD
ggIBACDIw41L3KlXG0aMiS//cqrG+EShHUGo8HNsw30W1kJtjn6UBwRM6jnmiwfB
Pb8VA91chb2vssAtX2zbTvqBJ9+LBPGCdw/E53Rbf86qhxKaiAHOjpvAy5Y3m00m
qC0w/Zwvju1twb4vhLaJ5NkUJYsUS7rmJKHHBnETLi8GFqiEsqTWpG/6ibYCv7rY
DBJDcR9W62BW9jfIoBQcxUCUJouMPH25lLNcDc1ssqvC2v7iUgI9LeoM1sNovqPm
QUiG9rHli1vXxzCyaMTjwftkJLkf6724DFhuKug2jITV0QkXvaJWF4nUaHOTNA4u
JU9WDvZLI1j83A+/xnAJUucIv/zGJ1AMH2boHqF8CY16LpsYgBt6tKxxWH00XcyD
CdW2KlBCeqbQPcsFmWyWugxdcekhYsAWyoSf818NUsZdBWBaR/OukXrNLfkQ79Iy
ZohZbvabO/X+MVT3rriAoKc8oE2Uws6DF+60PV7/WIPjNvXySdqspImSN78mflxD
qwLqRBYkA3I75qppLGG9rp7UCdRjxMl8ZDBld+7yvHVgt1cVzJx9xnyGCC23Uaic
MDSXYrB4I4WHXPGjxhZuCuPBLTdOLU8YRvMYdEvYebWHMpvwGCF6bAx3JBpIeOQ1
wDB5y0USicV3YgYGmi+NZfhA4URSh77Yd6uuJOJENRaNVTzk
-----END CERTIFICATE-----
";

    const RSA_TEE_0: &str = "\
    -----BEGIN CERTIFICATE-----
MIIGCDCCBHCgAwIBAgIBATANBgkqhkiG9w0BAQsFADApMRkwFwYDVQQFExAyZGM1
OGIyZDFhMjQxMzI2MQwwCgYDVQQMDANURUUwIBcNNzAwMTAxMDAwMDAwWhgPMjEw
NjAyMDcwNjI4MTVaMB8xHTAbBgNVBAMMFEFuZHJvaWQgS2V5c3RvcmUgS2V5MIIB
IjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEApNVcnyN40MANMbbo2nMGNq2N
NysDSjfLm0W3i6wPKf0ffCYkhWM4dCmQKKf50uAZTBeTit4cNwXeZn3qellMlOsI
N3Qc384rfN/8cikrRvUAgibz0Jy7STykjwa7x6tKwqITxbO8HqAhKo8/BQXUxzrO
dIg5ciy+UM7Vgh7a7ogen0KL2iGgrsalb1ti7Vlzb6vIJ4WzIC3TGD2sCkoPahgh
wqFDZZCo/FzaLoNY0jAUX2mL+kf8aUaoxz7xA9FTvgara+1pLBR1s4c8xPS2HdZi
pcVXWfey0wujv1VAKs4+tXjKlHkYBHBBceEjxUtEmrapSQEdpHPv7Xh9Uanq4QID
AQABo4ICwTCCAr0wDgYDVR0PAQH/BAQDAgeAMIICqQYKKwYBBAHWeQIBEQSCApkw
ggKVAgEDCgEBAgEECgEBBANhYmMEADCCAc2/hT0IAgYBZOYGEYe/hUWCAbsEggG3
MIIBszGCAYswDAQHYW5kcm9pZAIBHTAZBBRjb20uYW5kcm9pZC5rZXljaGFpbgIB
HTAZBBRjb20uYW5kcm9pZC5zZXR0aW5ncwIBHTAZBBRjb20ucXRpLmRpYWdzZXJ2
aWNlcwIBHTAaBBVjb20uYW5kcm9pZC5keW5zeXN0ZW0CAR0wHQQYY29tLmFuZHJv
aWQuaW5wdXRkZXZpY2VzAgEdMB8EGmNvbS5hbmRyb2lkLmxvY2FsdHJhbnNwb3J0
AgEdMB8EGmNvbS5hbmRyb2lkLmxvY2F0aW9uLmZ1c2VkAgEdMB8EGmNvbS5hbmRy
b2lkLnNlcnZlci50ZWxlY29tAgEdMCAEG2NvbS5hbmRyb2lkLndhbGxwYXBlcmJh
Y2t1cAIBHTAhBBxjb20uZ29vZ2xlLlNTUmVzdGFydERldGVjdG9yAgEdMCIEHWNv
bS5nb29nbGUuYW5kcm9pZC5oaWRkZW5tZW51AgEBMCMEHmNvbS5hbmRyb2lkLnBy
b3ZpZGVycy5zZXR0aW5ncwIBHTEiBCAwGqPLCBE0UBxF8UIqvGbCQiT9Xe1f3I8X
5pcXb9hmqjCBrqEIMQYCAQICAQOiAwIBAaMEAgIIAKUFMQMCAQSmCDEGAgEDAgEF
v4FIBQIDAQABv4N3AgUAv4U+AwIBAL+FQEwwSgQgAAAAAAAAAAAAAAAAAAAAAAAA
AAAAAAAAAAAAAAAAAAABAQAKAQIEIHKNsSdPHxzxVx3kOAsEilVKxKOA529TVQg1
KQhKk3gBv4VBAwIBAL+FQgUCAwMUs7+FTgUCAwMUs7+FTwUCAwMUszANBgkqhkiG
9w0BAQsFAAOCAYEAJMIuzdNUdfrE6sIdmsnMn/scSG2odbphj8FkX9JGdF2SOT59
9HuDY9qhvkru2Dza4sLKK3f4ViBhuR9lpfeprKvstxbtBO7jkLYfVn0ZRzHRHVEy
iW5IVKh+qOXVJ9S1lMShOTlsaYJytLKIlcrRAZBEXZiNbzTuVh1CH6X9Ni1dog14
snm+lcOeORdL9fht2CHau/caRnpWiZbjoAoJp0O89uBrRkXPpln51+3jPY6AFny3
0grNAvKguauDcPPhNV1yR+ylSsQi2gm3Rs4pgtlxFLMfZLgT0cbkl+9zk/QUqlpB
P8ftUBsOI0ARr8xhFN3cvq9kXGLtJ9hEP9PRaflAFREkDK3IBIbVcAFZBFoAQOdE
9zy0+F5bQrznPGaZg4Dzhcx33qMDUTgHtWoy+k3ePGQMEtmoTTLgQywWOIkXEoFq
qGi9GKJXUT1KYi5NsigaYqu7FoN4Qsvs61pMUEfZSPP2AFwkA8uNFbmb9uxcxaGH
CA8i3i9VM6yOLIrP
-----END CERTIFICATE-----
";

    const RSA_TEE_1: &str = "\
    -----BEGIN CERTIFICATE-----
MIIFDjCCAvagAwIBAgIJFIcgYhN4mUUVMA0GCSqGSIb3DQEBCwUAMCkxGTAXBgNV
BAUTEDI5NjBjYjlhZWJkNTA1ZjQxDDAKBgNVBAwMA1RFRTAeFw0xODAzMjEyMDU4
NDhaFw0yODAzMTgyMDU4NDhaMCkxGTAXBgNVBAUTEDJkYzU4YjJkMWEyNDEzMjYx
DDAKBgNVBAwMA1RFRTCCAaIwDQYJKoZIhvcNAQEBBQADggGPADCCAYoCggGBALTJ
jMPjzdG6i/oPf+OuQpE0Pbamcs+t1F1jAeNJXcCSN+hL5kTzPBto7WUX64+D74/S
bwbHzSNsRptOwus2Fj4MXxdCcNjA+nYT19H++30ecY932WYtP6geamSZlZrNrh4D
RFGuSfOtQ2juw+alEEzuAfS5DigRP8RUjURyLXAQMIVGuIZoT1voB+H6OEv8ICal
lFkf7xI8ycKVYhOCNpNuf3tKELhaNqpZh7mAuaXMrxhsc7WiFsm6cBGY/UpZk/3k
oHUrHgCJxo3CJEYQUFEE+du4yJ4KRSJabC/3GzE415gKPPvcFRm7/9yQWt4VY8X9
F6EDkfsEYawDN+ZFN7sQSRuaFVA1+6U1UA0TlFjFCvAbMvlvb0RywIvaylpVQxCa
FHtwMnOeQoMfic9baiinUHqeDdV85YtK5ZAXBkv0Cty/43Xlq4zJ4O/TZa/dqqrt
YkdmzGjUu5ubtiT2AknFJjS/TwtDI7obVOgQYnStWQjZWBafVrm6l62BeFuEqQID
AQABo4G4MIG1MB0GA1UdDgQWBBRopWEF5KUGUKoHHaveKWTVGPzLDDAfBgNVHSME
GDAWgBT8ZOhxpzVk8i/b9lrISMYu2Rl7JDAPBgNVHRMBAf8EBTADAQH/MA4GA1Ud
DwEB/wQEAwICBDBSBgNVHR8ESzBJMEegRaBDhkFodHRwczovL2FuZHJvaWQuZ29v
Z2xlYXBpcy5jb20vYXR0ZXN0YXRpb24vY3JsLzE0ODcyMDYyMTM3ODk5NDUxNTAN
BgkqhkiG9w0BAQsFAAOCAgEAfdEIlzih1i06VvWE+ZutfqoV5mvcFQb1ey8lzrvP
hEPjm4SeKi6wLR1f1O12uOhYbAQzzOTmEuYsFqje3N6WGZVnXSXIvxlUT0D3kVqg
M48+dt+a27xxTMy3QEJNYzFdgOrrRIAzen5sXYqEhynP4pD40VEz1p0VFgQtGgtV
Bk/+jFg/aAZGmL2YRGeF4mIygSVsW0QCvoodNbPZ6qyZUIxDK1fTFXPy31PXvrP2
e8IxMDMkD3aRh3sKdzc498k/NrBbQG8lWwL/Fldx/BSORNNpJzXpHUYgPThaicZh
sOesMBQoa+Mhl6czaCDQ8rQo/lGs+wN7iyQ9o07/6uN7x4j1Olhh0bEH/edZenZE
x3Jnmmfea42vck83L6SvHgprkIlGiq1vKoTk703RsIS23RivRq9g3bI2DnVEDW3Z
lbY/i3LxTj1d2DRuiOY6LTtqiIL+OPXAeeB4pIQyyTqtl8kKxrOTSxZb+BCIKosr
xFTaUlIjFeCexvQso9DMSOkt2LCx/ckJYxLZ+UW/azbzhikbRwSkTTD6GQ1XWpLG
m/DH+PZT6BRQTJt3H9b/IfCNI0j/Z+p7s4tug3x5yp2lFT/Yfz7lC7+nuX6EsAV3
EwPSotY5eRNOwzMcuoQfW+Ngz5juQd4pixo/f6DLJTcZwO+uHSVSWRKtaODAtMw8
sOQ=
-----END CERTIFICATE-----
";

    const RSA_TEE_2: &str = "\
    -----BEGIN CERTIFICATE-----
MIIFfzCCA2egAwIBAgIKA4gmZ2BliZaFfDANBgkqhkiG9w0BAQsFADAbMRkwFwYD
VQQFExBmOTIwMDllODUzYjZiMDQ1MB4XDTE4MDMyMTIwNTMyMVoXDTI4MDMxODIw
NTMyMVowKTEZMBcGA1UEBRMQMjk2MGNiOWFlYmQ1MDVmNDEMMAoGA1UEDAwDVEVF
MIICIjANBgkqhkiG9w0BAQEFAAOCAg8AMIICCgKCAgEArsj0RmYrjTICBSQVVcaX
db0lM/mrDf4hbQGxgzmBshG5hoIGmR3rWQ0iL6NNXELhsTjif/15kPsSkZKtiCz9
S1/1ZJeYP2BhenkNE7CesaYqDU52+IU2tXv91ee+AfELlp4jmyb4BbB7S8mg4kB1
VRyYhbelUsQZ/AB6SnPzqrhnIW5NR8fGV+1hkCQ/KlSc6JRUkcVAWtB/jvF/k8ji
573tJG8q56Qu5q4HX+zOeo8X6XcCqalO9uea8MTSs68gAJtWGCmSfqTpkoayTCz7
/P02Ya5TbxncwMieOmVw4iWI3VFRqS9x3GZx89ySECU+O1UkF/tOzYjCNWMCdbJR
N5BgXIottTg/EdMQjheaRrWE9lD9DLzcBH0rWDB3tzoNmdBAK3S5C6M/XtH+qu/w
hheEFkWmSkfK4N/rURfvILnahaaO1luWwBI4T/7jPykcUYNyBcIKITT1U1LkTjsO
EqMZXmwL+yOUsb6QoaFQdyj8QaFU39S6nIWPhgedzocLrXGJ1wSkt5sid1Kd+PL5
i2/foWdEkaXFd5SwLX72JmJbJ8kxbxJpUtfzXjH3awd6R8DyQCDHUCblOpaZOJOj
bHmzhQmdyqXpYdbjfVlb0y/Xlg2j4Z20RzXbv8wylAnvxuLeoqlGljRxaEOFZDBz
WSJrrsLJOLqc23eWlB7KAnMCAwEAAaOBtjCBszAdBgNVHQ4EFgQU/GTocac1ZPIv
2/ZayEjGLtkZeyQwHwYDVR0jBBgwFoAUNmHhAHyIBQlRi0RsR/8aTMnqTxIwDwYD
VR0TAQH/BAUwAwEB/zAOBgNVHQ8BAf8EBAMCAgQwUAYDVR0fBEkwRzBFoEOgQYY/
aHR0cHM6Ly9hbmRyb2lkLmdvb2dsZWFwaXMuY29tL2F0dGVzdGF0aW9uL2NybC9F
OEZBMTk2MzE0RDJGQTE4MA0GCSqGSIb3DQEBCwUAA4ICAQCqHtl3I1gfODJVBMZy
IsHouloSr2KlpLhyvC0gPdgvN5jkNtJPYzcfjtHyNDEBv1QRu1ezdb95kk4sU7o9
M4YuwA9gOb2QguJ/ihaTSOW0hszqHjIX8HuYls/7D/mtdO1OQ5JD8a38hR9uhlNg
n+mSiBvwr9gySVGWWSAu6O9eZYOaCfTgc2tx0iqNltXNbL0OJZ8wYoBOo2fT72/E
jOsQDoDPILrEfl/B2ek1cEFcB7iSEWrf3tyqhq1zpaRs5k2fWYqRuXGDg+B0QUh6
cpC9bwbK5sBcOSOqtEZHBjEhIGZ1Sn6dRTCSk15yrMnl0U3EBXFxWrecqr6o4VsM
CvXFazpmDVutU1t9TUV8NaIFL2Yh7D5BnctIe8Q/zRTn/XBLy1IOu2b971elMs1B
Cd6T4KD5H9pT1RG8q8aN+tyu2lj3i0dx6ktwQKGPHeJJYW+1NZqFzBoLijnMNhV4
lQhBHmdmuy8YPDGZIwsiojNsJ+3lXM1mgypFrDj5d7bFerN4eB/8XsiCIf+j4dIe
YMIrAIee4P/KH9N2eXaA4O/EyojDlrlIKv0NHd50SZnAkxYJ8livW2wTyJd0SFu3
go5kDpjFxFuI+cBPUZSMtFhjwCvZHv1nmNrJGk4diUD3Taxpdax8Y49Ik7iLI2Gy
7vKtXwIzR9iyBKCOtvbTlhJWlQ==
-----END CERTIFICATE-----
";

    const RSA_TEE_3: &str = "\
    -----BEGIN CERTIFICATE-----
MIIFYDCCA0igAwIBAgIJAOj6GWMU0voYMA0GCSqGSIb3DQEBCwUAMBsxGTAXBgNV
BAUTEGY5MjAwOWU4NTNiNmIwNDUwHhcNMTYwNTI2MTYyODUyWhcNMjYwNTI0MTYy
ODUyWjAbMRkwFwYDVQQFExBmOTIwMDllODUzYjZiMDQ1MIICIjANBgkqhkiG9w0B
AQEFAAOCAg8AMIICCgKCAgEAr7bHgiuxpwHsK7Qui8xUFmOr75gvMsd/dTEDDJdS
Sxtf6An7xyqpRR90PL2abxM1dEqlXnf2tqw1Ne4Xwl5jlRfdnJLmN0pTy/4lj4/7
tv0Sk3iiKkypnEUtR6WfMgH0QZfKHM1+di+y9TFRtv6y//0rb+T+W8a9nsNL/ggj
nar86461qO0rOs2cXjp3kOG1FEJ5MVmFmBGtnrKpa73XpXyTqRxB/M0n1n/W9nGq
C4FSYa04T6N5RIZGBN2z2MT5IKGbFlbC8UrW0DxW7AYImQQcHtGl/m00QLVWutHQ
oVJYnFPlXTcHYvASLu+RhhsbDmxMgJJ0mcDpvsC4PjvB+TxywElgS70vE0XmLD+O
JtvsBslHZvPBKCOdT0MS+tgSOIfga+z1Z1g7+DVagf7quvmag8jfPioyKvxnK/Eg
sTUVi2ghzq8wm27ud/mIM7AY2qEORR8Go3TVB4HzWQgpZrt3i5MIlCaY504LzSRi
igHCzAPlHws+W0rB5N+er5/2pJKnfBSDiCiFAVtCLOZ7gLiMm0jhO2B6tUXHI/+M
RPjy02i59lINMRRev56GKtcd9qO/0kUJWdZTdA2XoS82ixPvZtXQpUpuL12ab+9E
aDK8Z4RHJYYfCT3Q5vNAXaiWQ+8PTWm2QgBR/bkwSWc+NpUFgNPN9PvQi8WEg5Um
AGMCAwEAAaOBpjCBozAdBgNVHQ4EFgQUNmHhAHyIBQlRi0RsR/8aTMnqTxIwHwYD
VR0jBBgwFoAUNmHhAHyIBQlRi0RsR/8aTMnqTxIwDwYDVR0TAQH/BAUwAwEB/zAO
BgNVHQ8BAf8EBAMCAYYwQAYDVR0fBDkwNzA1oDOgMYYvaHR0cHM6Ly9hbmRyb2lk
Lmdvb2dsZWFwaXMuY29tL2F0dGVzdGF0aW9uL2NybC8wDQYJKoZIhvcNAQELBQAD
ggIBACDIw41L3KlXG0aMiS//cqrG+EShHUGo8HNsw30W1kJtjn6UBwRM6jnmiwfB
Pb8VA91chb2vssAtX2zbTvqBJ9+LBPGCdw/E53Rbf86qhxKaiAHOjpvAy5Y3m00m
qC0w/Zwvju1twb4vhLaJ5NkUJYsUS7rmJKHHBnETLi8GFqiEsqTWpG/6ibYCv7rY
DBJDcR9W62BW9jfIoBQcxUCUJouMPH25lLNcDc1ssqvC2v7iUgI9LeoM1sNovqPm
QUiG9rHli1vXxzCyaMTjwftkJLkf6724DFhuKug2jITV0QkXvaJWF4nUaHOTNA4u
JU9WDvZLI1j83A+/xnAJUucIv/zGJ1AMH2boHqF8CY16LpsYgBt6tKxxWH00XcyD
CdW2KlBCeqbQPcsFmWyWugxdcekhYsAWyoSf818NUsZdBWBaR/OukXrNLfkQ79Iy
ZohZbvabO/X+MVT3rriAoKc8oE2Uws6DF+60PV7/WIPjNvXySdqspImSN78mflxD
qwLqRBYkA3I75qppLGG9rp7UCdRjxMl8ZDBld+7yvHVgt1cVzJx9xnyGCC23Uaic
MDSXYrB4I4WHXPGjxhZuCuPBLTdOLU8YRvMYdEvYebWHMpvwGCF6bAx3JBpIeOQ1
wDB5y0USicV3YgYGmi+NZfhA4URSh77Yd6uuJOJENRaNVTzk
-----END CERTIFICATE-----
";

    const RSA_STRONGBOX_0: &str = "\
    -----BEGIN CERTIFICATE-----
MIIFjjCCBHagAwIBAgIBATANBgkqhkiG9w0BAQsFADAvMRkwFwYDVQQFExBjY2Nl
MjYzZmQwOGRhYzNhMRIwEAYDVQQMDAlTdHJvbmdCb3gwHhcNNzAwMTAxMDAwMDAw
WhcNMjgwNTIzMjM1OTU5WjAfMR0wGwYDVQQDDBRBbmRyb2lkIEtleXN0b3JlIEtl
eTCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAN/ps3b6b+nFF1AdwOAp
Hr3zG8icT4AIkuMztfzTusXP88wDbzgh1Nsf/T4M9xaTy/jHz4wukPjxAX88/u0m
DinKb9SrgzDLfXyhV7PPus0wEqUfGaN62Bwx2ei3UmZCvyzLruCUUauABZsyH7Gu
dj7F4yfwvveLlHEpRx4Up9UzrnkKV3Kgd8mQf67CVBE66AGRPXJzyBu7+4WDAA0l
hAvE0XOyKac6Hl9CRxSBDX2QzAooVsjOaWjg6eKt+ed1lNYNdxNGNK4UX2e9LEWA
JlI50p5jLx3g5Uksu1S9WS1u9OU5L52ccGa/OSkPON/qxVf/3Yx6DuuuupPs01RL
/J8CAwEAAaOCAsMwggK/MA4GA1UdDwEB/wQEAwIHgDCCAqsGCisGAQQB1nkCAREE
ggKbMIIClwIBAwoBAgIBBAoBAgQDYWJjBAAwggHNv4U9CAIGAWv2IqDMv4VFggG7
BIIBtzCCAbMxggGLMAwEB2FuZHJvaWQCAR0wGQQUY29tLmFuZHJvaWQua2V5Y2hh
aW4CAR0wGQQUY29tLmFuZHJvaWQuc2V0dGluZ3MCAR0wGQQUY29tLnF0aS5kaWFn
c2VydmljZXMCAR0wGgQVY29tLmFuZHJvaWQuZHluc3lzdGVtAgEdMB0EGGNvbS5h
bmRyb2lkLmlucHV0ZGV2aWNlcwIBHTAfBBpjb20uYW5kcm9pZC5sb2NhbHRyYW5z
cG9ydAIBHTAfBBpjb20uYW5kcm9pZC5sb2NhdGlvbi5mdXNlZAIBHTAfBBpjb20u
YW5kcm9pZC5zZXJ2ZXIudGVsZWNvbQIBHTAgBBtjb20uYW5kcm9pZC53YWxscGFw
ZXJiYWNrdXACAR0wIQQcY29tLmdvb2dsZS5TU1Jlc3RhcnREZXRlY3RvcgIBHTAi
BB1jb20uZ29vZ2xlLmFuZHJvaWQuaGlkZGVubWVudQIBATAjBB5jb20uYW5kcm9p
ZC5wcm92aWRlcnMuc2V0dGluZ3MCAR0xIgQgMBqjywgRNFAcRfFCKrxmwkIk/V3t
X9yPF+aXF2/YZqowgbChCDEGAgECAgEDogMCAQGjBAICCAClBTEDAgEEpggxBgIB
AwIBBb+BSAUCAwEAAb+DdwIFAL+FPgMCAQC/hUBMMEoEIAAAAAAAAAAAAAAAAAAA
AAAAAAAAAAAAAAAAAAAAAAAAAQEACgECBCByjbEnTx8c8Vcd5DgLBIpVSsSjgOdv
U1UINSkISpN4Ab+FQQMCAQC/hUIFAgMDFLO/hU4GAgQBNBXxv4VPBgIEATQV7DAN
BgkqhkiG9w0BAQsFAAOCAQEAmu4L5Rs3q0EjeSJYIJVzGi4R/e9uNpS1IR5p6sjw
sWXIuS+KGNjE/bD9hiDDNW3EglUGU2Xp9cW/PhVTAp6hnappa1dZb5W4EUAwtedU
sLsgBva1YKJD5/EB+6vaktJLaMqFRK1x8V11SAVi45hBU8wgALmGTIN9n0w+mxs4
isab+unysXpnzcd7PRN/SdbyQ1AY2P52FDemV/VvMqUgfQTHLkf0Sjk3ydWDtpQ8
xAppxhssHofWeh84FXSIJCSQ59FUCgb+aPdyvKNZPDoUfaX098cAs7JXTFRhfW64
jOmoaNdcPfzSGFF7pMcsOWTo2vmCvKO+BRxsWDhPfB/k1Q==
-----END CERTIFICATE-----
";

    const RSA_STRONGBOX_1: &str = "\
    -----BEGIN CERTIFICATE-----
MIIEnTCCAoWgAwIBAgIKFYRggggVNRl4dzANBgkqhkiG9w0BAQsFADAvMRkwFwYD
VQQFExA2OTdiYzY0YjZjZDRjMDFlMRIwEAYDVQQMDAlTdHJvbmdCb3gwHhcNMTgw
MzIxMDQwOTE4WhcNMjgwMzE4MDQwOTE4WjAvMRkwFwYDVQQFExBjY2NlMjYzZmQw
OGRhYzNhMRIwEAYDVQQMDAlTdHJvbmdCb3gwggEiMA0GCSqGSIb3DQEBAQUAA4IB
DwAwggEKAoIBAQC44jdUqUTiyjmNPPY3KlN088t8B7qsmChFfIbZuNBHgr8mYY1Z
hjs170QO9OqXUtjhmWN1dAl9NtEe0dL+RgaOYD+DcgjObj4VuCvxo6iMLd5AyPUQ
T/js61g3mwC2XqZEEJPhen/KIX1cfDYYng4m3n8/IU/uVC1E1MaTMJDBZj+eaa9N
OStB/Srj/oQHl+3kzU72BtLL4vrmfbYhACv1RHRhA1CXIZeW6G8rlPeMCO1tjyXO
QttG2vr/Dj/dnRFOFIQAXCiIw3v63qah7eSWeS59/WncAS7VG8iEPqb/NwBdnGFi
JkagsADsFWJO8HJ1PIpwQ1xC/qnALLP9Ec65AgMBAAGjgbowgbcwHQYDVR0OBBYE
FAd28hRYKlUaWnBgG+hRJkH2Axy2MB8GA1UdIwQYMBaAFCwYZ+XKkRf4peQMHg55
sQUm/p/vMA8GA1UdEwEB/wQFMAMBAf8wDgYDVR0PAQH/BAQDAgIEMFQGA1UdHwRN
MEswSaBHoEWGQ2h0dHBzOi8vYW5kcm9pZC5nb29nbGVhcGlzLmNvbS9hdHRlc3Rh
dGlvbi9jcmwvMTU4NDYwODIwODE1MzUxOTc4NzcwDQYJKoZIhvcNAQELBQADggIB
ACrYYBCoxXuVBmsHv8rCDAsw0RNow3nJgGZjVlzPGO/Gpu/P1qkKb+ugu7BIvUdc
4MScbxjy/9JXUZGwTwski4n3rTwmSaMt5RP+eLTdKoLVkW1nh+FD6el3nKigN/vF
qoAaHv3ayA/7ERGEN5MU8vIQKu4BajPjP2RALHV8vTBm0JZbv/fByrS55kqnNLlW
GEgmyEC0oK+eG3sa/+ux3RJWgd6VyHhr8TdvV8NwLmEEa19DMiOTSC/vEPytwRj3
3FHGpubLpGuoAV3KpCXXHBVNChL7nn1bjnvHY2XA2xpbWp9gdJvDk2f9ODNIpQPu
f0fwWUeYyG+xof3BynJkCcfxmT04oDEFT1ANO2+cHL6cE2hJ9vJ8nblqvNOfyRgN
mYghVYXC19aqlr6wJHun5Lw2Ldd7AGU4Rfvb8ckgf8tGs2mz4XbFLlOZVbDC9bRj
wZ7F7ufPgDm0gN9qJHgG5dFe60nnm6Veq2VGnRz2w8GtjGb1sRFae/tiMt7eRwCH
HVGPXC7fCCWvWNbHn/2NhL+GzbGnkcU0wNEvuCn2kFEmY/q0SZp10qBTC+ztAG4F
XUVxdmDomq3sJXm3hxtMXvI948rGC1+C/xrrumUHLLZxJuQ/FB1p5pnJKaFKNZjh
uNBraB4/sXn9BcJ+XiqHnpludX2/IVddtXiyKwNbsC1X
-----END CERTIFICATE-----
";

    const RSA_STRONGBOX_2: &str = "\
    -----BEGIN CERTIFICATE-----
MIIFhTCCA22gAwIBAgIKBpaXYEQ3RICBoTANBgkqhkiG9w0BAQsFADAbMRkwFwYD
VQQFExBlMzVkMzhjNjg5N2Q0N2U4MB4XDTE4MDMyMTA0MDkxOFoXDTI4MDMxODA0
MDkxOFowLzEZMBcGA1UEBRMQNjk3YmM2NGI2Y2Q0YzAxZTESMBAGA1UEDAwJU3Ry
b25nQm94MIICIjANBgkqhkiG9w0BAQEFAAOCAg8AMIICCgKCAgEAxVPgrTkUevIE
JF4VU1JUKet/hsNmQcpl0OwSKEpEFAj8lMusW5LV9U7ZZ6dXK1SrJKQsC2Z9bFWL
3oqDfy+oJnliwgtrSdttIOcGEE7m9QFh+UPhcQplIqCdyf4IGu8kxx2pWDD0ZLGG
19B/NMiyTUKG9zSEkrOOd6t19KOxk+uVbfgQKRPAX4B9nq+EDZ4aH0mACFQomNpD
3ndUMWzvxFRR24xn0TjnYBfnxLqCYwfLhWpHpwKol1GQi/trUoCwsAsyO7bNQXnV
omQa3D5yf8Z0a+xG8qXLunWa4ydl/yEfWO+ahxov30dMzos1dD+gjc59Z4FkGZPo
LZWh0CSKWbbVb0gL8aEKB9EMFQUlDqpOhVzpWQ7xQTkUPlGiuUU0uPtCFe8SU1ZC
pwD3nyArVgqTKoH20fciFSjlIIdjgXRZsVL0skO4ZmSmfFndn25imE+dBbRUooV8
sIdytDFnNxBritjM3gPD0LGqIqpykJkFeHfIe2jT1f+iEK50P/cgCOtZSpGbL6x9
Vg2/2lPQjygynVsJ6U00fW1Z/s+hW8WxnfcRXSWBK1/2AC/6wNNpSriZJqjV0tFL
O6WJvp8kOcUev3/lqgafgQHrwSJIRKGLTfaSlzfsghzNtCmawLDSs6ASv0PnYabx
k4KbbhvdiL2eHysU5G+Tk0HMeX24lPECAwEAAaOBtjCBszAdBgNVHQ4EFgQULBhn
5cqRF/il5AweDnmxBSb+n+8wHwYDVR0jBBgwFoAUUw5FGxMV061c6o7ovFu2FIrN
HegwDwYDVR0TAQH/BAUwAwEB/zAOBgNVHQ8BAf8EBAMCAgQwUAYDVR0fBEkwRzBF
oEOgQYY/aHR0cHM6Ly9hbmRyb2lkLmdvb2dsZWFwaXMuY29tL2F0dGVzdGF0aW9u
L2NybC8zNTA1QTBBMkZEQ0M1MUU0MA0GCSqGSIb3DQEBCwUAA4ICAQCDsknVUCZN
IsdCxDechWvMXb+1lfYrKzc9MMstTHM5DNWR20Z7ZkC4dHRgYyhbdAIJCwGLkRhZ
7+mIwKaz0MkxGB09mZd6bF0vzflDRbOdXb5Yo1rtL0caxUbkpSwNSvVVaIyCvNNf
vwZSbqvRWyT8QXyijs5jsJyl2UX7fFy01YXfW6TFMBwagDVS9R5iuvpGQqGRaoCA
K5I0jY3h84fJkZM+eCb/WiwT6fpBSIpyMtgoWadNKwsqiX0MWa40k/Gx0v0zK4es
fYBO9pumZyL8TnnSZEKjwJOR9hSlB+i2c2I/Scc3gLKXirgF6ZDM5wFYJJpEq3SV
OYYUNxXYrw+RzHo/ostjjgrbJUACLR4kbRh/nZQFYCB9+N6E9QzA60VHySRYV21U
JQN6iqHcRUVz50tfmtF7raE2E6GfDmnAxBlvhPRQVgD7NLs8JNTKuZ4gyJ/r4Dx+
V8MpJAyCDarlCm+Mcl5c2pfjXa64tRSEvQ6/yJDAeBSylBb19ptIqDQLC09Q3PjN
ziRO4uLggRPZf/FBXBqTtrqp+rF2fQorTCWdspcoPzCOnswSZvYeixBNdUUbeXnD
fRFgeH8uBsd0yOf3pAKLUPJfXfHwY0/9tzBe3iC8nFC+MkbZQHoklOovI67sfOmS
bEKDtUffMf+oAHSxXpNxjY8CBVC8cFkEdA==
-----END CERTIFICATE-----
";

    const RSA_STRONGBOX_3: &str = "\
    -----BEGIN CERTIFICATE-----
MIIFXzCCA0egAwIBAgIINQWgov3MUeQwDQYJKoZIhvcNAQELBQAwGzEZMBcGA1UE
BRMQZTM1ZDM4YzY4OTdkNDdlODAeFw0xODAzMjEwMzU1MDFaFw0yODAzMTgwMzU1
MDFaMBsxGTAXBgNVBAUTEGUzNWQzOGM2ODk3ZDQ3ZTgwggIiMA0GCSqGSIb3DQEB
AQUAA4ICDwAwggIKAoICAQCoQi070/6PH9BAuJiBcTp8j5R2/Fj6kXFaSxsvUjJK
Rdi/FCOwUFBJfyhHiWJhga2iguIjAJuhZx5XlMj0pSY7buJisPqFknZhKdvfoi4C
54j54D+XxCky1APVjD5uc203H+hrRlhh6x4/LTzSXWvb0YLjfOK07HvSddSRCyKn
PydI5bhyCb5QMtVKHzC4Axgx+BihwG1B4UQjOpZpXBHIFZ6EK/XBFeJX0tgrx0MC
czkc1X0OhNFKAYumCKcKyh4q2cGh7UwTRJIT4beIJVrOKDVwo3Fc50k3ICpOAc1z
RGzRwupIKKGtW455KW1OIyAtTJ4NqfIwrkQy5EJ9w/zDzjqFiDNdgTXaMtsz52jH
UndfO3lzfvVdBjt2FWLRkWrFmd1tjQ6LQQFPrSvUHo0XN5kIiOYdNlAyIwBZ51Zn
/clU9he78UYAmXHdjfh04DXWgf+GGIFFdmg9tbiHwdqi8yEcakKHL2TYNylmFJuK
OkvLIS5iqEMu29OQyv+BJ1NNQtyn8o1D5f3K2a5qyoUcrL0Je2w28ByL1JeFURO7
wbzkXx9FWwH0stCs/dYq4JKJ8GxKN8aBOl/51atA1cdZAyI55B9ueAYwPevteORG
DhEfcKRaGiWJpSzIFLDmbL9uSpCAw5uqx5h1IRAX1OLi56EsVaFUedt8IqUaCkUc
MQIDAQABo4GmMIGjMB0GA1UdDgQWBBRTDkUbExXTrVzqjui8W7YUis0d6DAfBgNV
HSMEGDAWgBRTDkUbExXTrVzqjui8W7YUis0d6DAPBgNVHRMBAf8EBTADAQH/MA4G
A1UdDwEB/wQEAwICBDBABgNVHR8EOTA3MDWgM6Axhi9odHRwczovL2FuZHJvaWQu
Z29vZ2xlYXBpcy5jb20vYXR0ZXN0YXRpb24vY3JsLzANBgkqhkiG9w0BAQsFAAOC
AgEACHPElcXZEY6jItn3SnVn6gdOCvuFIrhB4J4fhxa6OlDCbxsylHHAbpyoOSJD
0R5dIBTRvz49ElfCpI9yp4l5tUOjWj2K3tMfQjhKN277jApdxbZ6MNac2u+Z3dwS
+34YiiCJLVKhVzWanXk/+9rmVnBzm7qvgan90j2SmH1oYIA3GowJL+1OWSjj6cBH
1VrU7guVd7q+aCEQPhTesfbmUkdUbM/TjUMyhf5SZ7WB8A28MnyYQcFttmGDsE1H
Zx4fTz8iXxjaph33alasmFUWGtG7KyzvbYUwwUOmpPd32eT8ocx3B0Z3g+Vo2Cv2
LzRRNp15FOf/Ag5HWloyIv3xrsBA2fa4xzOm3/t1Aq0ZViXLLerKKmU/EJKo7t5g
E0wtNebicNhUJaX2C+vShjfleBBwhs33a8b7dBFX/JMwU5+09N83WHRPZjCrYuK3
Uu4qANfJ+f1k+weqh6MpRxAZ6TT0OQt1XQz7rDAlq7AopfXRXI74OCNJf8d+2CPN
VW8i5IHNFjvyEz8ms1ETdw5n3uRbYIgYuCV2Pud3MCwaMTv2E0dLAqg8uJo0JYlh
H7wbqidMm2ODRL5FaqWbb2pfW0vbpCyGfyw0RQGTJAE7xVdvzfdun9HATDm6TbJF
cejBdJImfwpZmDOBtDPIbsg5+QVE3lLF6Licq13i5tIg5yQ=
-----END CERTIFICATE-----
";

    const EC_STRONGBOX_0: &str = "\
    -----BEGIN CERTIFICATE-----
MIID8zCCA5egAwIBAgIBATAMBggqhkjOPQQDAgUAMC8xGTAXBgNVBAUTEDY5N2Jj
NjRiNmNkNGMwMWUxEjAQBgNVBAwMCVN0cm9uZ0JveDAeFw03MDAxMDEwMDAwMDBa
Fw0yODA1MjMyMzU5NTlaMB8xHTAbBgNVBAMMFEFuZHJvaWQgS2V5c3RvcmUgS2V5
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEM8o810z1VgBTtio2H1Gh5vA3ySYQ
0/RIfn/uPQRCiHGZ1K7tvhQobsfa04rM5PAPuaZDmDnD86C5T9SL+msTVqOCArAw
ggKsMA4GA1UdDwEB/wQEAwIHgDCCApgGCisGAQQB1nkCAREEggKIMIIChAIBAwoB
AgIBBAoBAgQDYWJjBAAwggHNv4U9CAIGAWvSW/8Tv4VFggG7BIIBtzCCAbMxggGL
MAwEB2FuZHJvaWQCAR0wGQQUY29tLmFuZHJvaWQua2V5Y2hhaW4CAR0wGQQUY29t
LmFuZHJvaWQuc2V0dGluZ3MCAR0wGQQUY29tLnF0aS5kaWFnc2VydmljZXMCAR0w
GgQVY29tLmFuZHJvaWQuZHluc3lzdGVtAgEdMB0EGGNvbS5hbmRyb2lkLmlucHV0
ZGV2aWNlcwIBHTAfBBpjb20uYW5kcm9pZC5sb2NhbHRyYW5zcG9ydAIBHTAfBBpj
b20uYW5kcm9pZC5sb2NhdGlvbi5mdXNlZAIBHTAfBBpjb20uYW5kcm9pZC5zZXJ2
ZXIudGVsZWNvbQIBHTAgBBtjb20uYW5kcm9pZC53YWxscGFwZXJiYWNrdXACAR0w
IQQcY29tLmdvb2dsZS5TU1Jlc3RhcnREZXRlY3RvcgIBHTAiBB1jb20uZ29vZ2xl
LmFuZHJvaWQuaGlkZGVubWVudQIBATAjBB5jb20uYW5kcm9pZC5wcm92aWRlcnMu
c2V0dGluZ3MCAR0xIgQgMBqjywgRNFAcRfFCKrxmwkIk/V3tX9yPF+aXF2/YZqow
gZ2hCDEGAgECAgEDogMCAQOjBAICAQClBTEDAgEEv4N3AgUAv4U+AwIBAL+FQEww
SgQgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAQAKAQIEIHKNsSdP
HxzxVx3kOAsEilVKxKOA529TVQg1KQhKk3gBv4VBAwIBAL+FQgUCAwMUs7+FTgYC
BAE0FfG/hU8GAgQBNBXsMAwGCCqGSM49BAMCBQADSAAwRQIhAN82bz9RzrMXznZK
gu61ktdu397wVvW2Fj/ZKOkcy8p/AiAFhziu1TGVBklOdPH4usrPM/FxAvlOSUDQ
wj4HP/9PSg==
-----END CERTIFICATE-----
";

    const EC_STRONGBOX_1: &str = "\
    -----BEGIN CERTIFICATE-----
MIICMjCCAbegAwIBAgIKFClzmQlEZCZgeDAKBggqhkjOPQQDAjAvMRkwFwYDVQQF
ExA2OTdiYzY0YjZjZDRjMDFlMRIwEAYDVQQMDAlTdHJvbmdCb3gwHhcNMTgwMzIx
MDQwOTE5WhcNMjgwMzE4MDQwOTE5WjAvMRkwFwYDVQQFExBjY2NlMjYzZmQwOGRh
YzNhMRIwEAYDVQQMDAlTdHJvbmdCb3gwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNC
AARpw1ZoXy8BqjHbt0eMA+8QRMoK5aqokaKxWSMAqhkUisyxjx3eqq5drIA6kYCB
cbMeEaK1wskkdE4GnCXoLCOso4G6MIG3MB0GA1UdDgQWBBRR/V0W3sbZqtUtA0+V
yFs/c1wixTAfBgNVHSMEGDAWgBTdlrDyGscOhnpN5p1KzdLVm6b0+DAPBgNVHRMB
Af8EBTADAQH/MA4GA1UdDwEB/wQEAwICBDBUBgNVHR8ETTBLMEmgR6BFhkNodHRw
czovL2FuZHJvaWQuZ29vZ2xlYXBpcy5jb20vYXR0ZXN0YXRpb24vY3JsLzE0Mjk3
Mzk5MDk0NDY0MjY2MDc4MAoGCCqGSM49BAMCA2kAMGYCMQDID/CfPKBG4jksFjWd
2uRWxcJcp3/pR1WorB2OmikDznwKNpyTUUnxw442IcwyPkICMQCGvhARszM/jpxz
5WdgbZNsKRVW6fLXlU1bXurrp4Cr36SkunBOrWXaX1Q9/hdjilk=
-----END CERTIFICATE-----
";

    const EC_STRONGBOX_2: &str = "\
    -----BEGIN CERTIFICATE-----
MIID1zCCAb+gAwIBAgIKBpaXYEQ3RICBojANBgkqhkiG9w0BAQsFADAbMRkwFwYD
VQQFExBlMzVkMzhjNjg5N2Q0N2U4MB4XDTE4MDMyMTA0MDkxOFoXDTI4MDMxODA0
MDkxOFowLzEZMBcGA1UEBRMQNjk3YmM2NGI2Y2Q0YzAxZTESMBAGA1UEDAwJU3Ry
b25nQm94MHYwEAYHKoZIzj0CAQYFK4EEACIDYgAEh8FMICv77jLds6f1v7+i24HW
vfgCwr7KtZFjLfsNhgfVXbvH3uOJIHjrgIemkhtEGXVTs+PMdf4M9b+czx0q7Uia
vVrDy+EAV0EzBqAlL3HMjHoTZ/svm5ZqS5rzDravo4G2MIGzMB0GA1UdDgQWBBTd
lrDyGscOhnpN5p1KzdLVm6b0+DAfBgNVHSMEGDAWgBRTDkUbExXTrVzqjui8W7YU
is0d6DAPBgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwICBDBQBgNVHR8ESTBH
MEWgQ6BBhj9odHRwczovL2FuZHJvaWQuZ29vZ2xlYXBpcy5jb20vYXR0ZXN0YXRp
b24vY3JsLzM1MDVBMEEyRkRDQzUxRTQwDQYJKoZIhvcNAQELBQADggIBAA71Fgy7
j41XkCE0CtKcM7wVM5E5s1ErWEbsRu8oWoLOy3Ty6GTUVcNj08BXGaP1Rx0E7sub
/U7cbsALeVG7mdzBWS9hW9GFkiiXYbDRN8eAk8EJcmWYe0j2P4EMgPM9rTm1lJRP
NjCieahD/9QipC/RFpb7nmp5Ftx2Cb/On4qYJFuGTT/0uofZ87omU0d6bpaDw/5F
o6MI24XoaxVfP02Zvni5DkvGFFLsYRsYyCTij1x3sWBYlkIjiahDOzj271Spkxqw
qF4AYSVvsIExa677vonOxgPSAeS+48qx3MVW0eI3RsTAOPlcYgI98feD5HN7x3ZW
D+uBYaLUul43mLamjJXK0HdH8rGm8jGBCNKKTPNWVzQEMfVxo4wzPTbjMMuUbN7g
8RBDf4wxhG+PSwEO3jwiHAvDfD5F9HUsffeX1cHZpMFiojvT6XLh2YsDHqtjRfSI
Y7AfVRl8TlnsF4nRDR4RrmYBmGeq/QWgZf0KOCWfjroBJbGN3GJx9OPxig4VrEyz
W3yWWjPuy5+F90eL5XXtp0DReY3zoNBeG7Ie9egkPWl2K4+AoU3eEuoZFE8FBkkX
9D5lhlWCfSdYRrVo46Psx8/cdTcHHJNi/cuZLszcDSXTGtZ8lWz1qgggWoT4F8gp
lodevGYAtntt19XvsVkCRbdaELNUoxuli/LP
-----END CERTIFICATE-----
";

    const EC_STRONGBOX_3: &str = "\
    -----BEGIN CERTIFICATE-----
MIIFXzCCA0egAwIBAgIINQWgov3MUeQwDQYJKoZIhvcNAQELBQAwGzEZMBcGA1UE
BRMQZTM1ZDM4YzY4OTdkNDdlODAeFw0xODAzMjEwMzU1MDFaFw0yODAzMTgwMzU1
MDFaMBsxGTAXBgNVBAUTEGUzNWQzOGM2ODk3ZDQ3ZTgwggIiMA0GCSqGSIb3DQEB
AQUAA4ICDwAwggIKAoICAQCoQi070/6PH9BAuJiBcTp8j5R2/Fj6kXFaSxsvUjJK
Rdi/FCOwUFBJfyhHiWJhga2iguIjAJuhZx5XlMj0pSY7buJisPqFknZhKdvfoi4C
54j54D+XxCky1APVjD5uc203H+hrRlhh6x4/LTzSXWvb0YLjfOK07HvSddSRCyKn
PydI5bhyCb5QMtVKHzC4Axgx+BihwG1B4UQjOpZpXBHIFZ6EK/XBFeJX0tgrx0MC
czkc1X0OhNFKAYumCKcKyh4q2cGh7UwTRJIT4beIJVrOKDVwo3Fc50k3ICpOAc1z
RGzRwupIKKGtW455KW1OIyAtTJ4NqfIwrkQy5EJ9w/zDzjqFiDNdgTXaMtsz52jH
UndfO3lzfvVdBjt2FWLRkWrFmd1tjQ6LQQFPrSvUHo0XN5kIiOYdNlAyIwBZ51Zn
/clU9he78UYAmXHdjfh04DXWgf+GGIFFdmg9tbiHwdqi8yEcakKHL2TYNylmFJuK
OkvLIS5iqEMu29OQyv+BJ1NNQtyn8o1D5f3K2a5qyoUcrL0Je2w28ByL1JeFURO7
wbzkXx9FWwH0stCs/dYq4JKJ8GxKN8aBOl/51atA1cdZAyI55B9ueAYwPevteORG
DhEfcKRaGiWJpSzIFLDmbL9uSpCAw5uqx5h1IRAX1OLi56EsVaFUedt8IqUaCkUc
MQIDAQABo4GmMIGjMB0GA1UdDgQWBBRTDkUbExXTrVzqjui8W7YUis0d6DAfBgNV
HSMEGDAWgBRTDkUbExXTrVzqjui8W7YUis0d6DAPBgNVHRMBAf8EBTADAQH/MA4G
A1UdDwEB/wQEAwICBDBABgNVHR8EOTA3MDWgM6Axhi9odHRwczovL2FuZHJvaWQu
Z29vZ2xlYXBpcy5jb20vYXR0ZXN0YXRpb24vY3JsLzANBgkqhkiG9w0BAQsFAAOC
AgEACHPElcXZEY6jItn3SnVn6gdOCvuFIrhB4J4fhxa6OlDCbxsylHHAbpyoOSJD
0R5dIBTRvz49ElfCpI9yp4l5tUOjWj2K3tMfQjhKN277jApdxbZ6MNac2u+Z3dwS
+34YiiCJLVKhVzWanXk/+9rmVnBzm7qvgan90j2SmH1oYIA3GowJL+1OWSjj6cBH
1VrU7guVd7q+aCEQPhTesfbmUkdUbM/TjUMyhf5SZ7WB8A28MnyYQcFttmGDsE1H
Zx4fTz8iXxjaph33alasmFUWGtG7KyzvbYUwwUOmpPd32eT8ocx3B0Z3g+Vo2Cv2
LzRRNp15FOf/Ag5HWloyIv3xrsBA2fa4xzOm3/t1Aq0ZViXLLerKKmU/EJKo7t5g
E0wtNebicNhUJaX2C+vShjfleBBwhs33a8b7dBFX/JMwU5+09N83WHRPZjCrYuK3
Uu4qANfJ+f1k+weqh6MpRxAZ6TT0OQt1XQz7rDAlq7AopfXRXI74OCNJf8d+2CPN
VW8i5IHNFjvyEz8ms1ETdw5n3uRbYIgYuCV2Pud3MCwaMTv2E0dLAqg8uJo0JYlh
H7wbqidMm2ODRL5FaqWbb2pfW0vbpCyGfyw0RQGTJAE7xVdvzfdun9HATDm6TbJF
cejBdJImfwpZmDOBtDPIbsg5+QVE3lLF6Licq13i5tIg5yQ=
-----END CERTIFICATE-----
";

    const FORGED_SELF_SIGNED: &str = "\
    -----BEGIN CERTIFICATE-----
MIIB7TCCAZOgAwIBAgIUVZcs3IU5EGhbxeUnJQSZLrb/mwMwCgYIKoZIzj0EAwIw
GzEZMBcGA1UEBRMQZjkyMDA5ZTg1M2I2YjA0NTAeFw0yNjEwMTExNTM4MTRaFw0z
NjEwMDgxNTM4MTRaMBsxGTAXBgNVBAUTEGY5MjAwOWU4NTNiNmIwNDUwWTATBgcq
hkjOPQIBBggqhkjOPQMBBwNCAARdOwcs3ZaXfXLZUANnALfe3xc5aWKA4rOV5pTx
F/FDngb7IeDjjv1t7liROa/xUhBwPsSHUVYAkvEb/XAxCxH5o4G0MIGxMA8GA1Ud
EwEB/wQFMAMBAf8wfwYKKwYBBAHWeQIBEQRxMG8CAgEsCgEBAgIBLAoBAQQJY2hh
bGxlbmdlBAAwADBQv4VATDBKBCARERERERERERERERERERERERERERERERERERER
EREREQEB/woBAAQgIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIwHQYD
VR0OBBYEFOHk/Tt/i16LDTRg1Ew9ydGPoz0HMAoGCCqGSM49BAMCA0gAMEUCIA/9
2l1V5xwAdeJJJHj8bQm8S73yNtD/9VUwBGc7DE51AiEA8H+rhsgyGtv+MJYQbCre
oM8LisleJlfVOVob2/bEW1M=
-----END CERTIFICATE-----
";
}

#!/usr/bin/env python3
"""Check, offline, that the NEAR AI key that signed the cassettes lives in a TEE.

    python3 scripts/check-attestation.py [demo/nearai-attestation.json]

Input: NEAR AI's attestation report for the model (fetched once with
`/v1/attestation/report?model=...&signing_algo=ecdsa&nonce=...`, no tokens
spent). Checks, with no network:
  1. the signing public key hashes (keccak-256) to the signing address;
  2. the cassettes (demo/cassette.json, demo/swarm-cassette.json) were
     signed by that address;
  3. the Intel TDX quote's report_data binds the signing address and our
     nonce (so the quote was made for this key, for this request);
  4. the quote is signed by its attestation key, the attestation key is
     bound by the Quoting Enclave report, the QE report is signed by the PCK
     certificate, and the PCK chain is signed up to a self-signed
     "Intel SGX Root CA" (a pinned SHA-256 of that root is checked);
  5. the NVIDIA GPU evidence carries the same nonce.
Not checked here (needs Intel's and NVIDIA's online collateral): TCB status,
revocation lists, and the GPU certificate chain and measurements.
"""
import hashlib
import json
import struct
import sys

from cryptography import x509
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import encode_dss_signature

# --- keccak-256 (Ethereum's, not SHA3-256), small pure-Python version
RC = [0x0000000000000001, 0x0000000000008082, 0x800000000000808A, 0x8000000080008000,
      0x000000000000808B, 0x0000000080000001, 0x8000000080008081, 0x8000000000008009,
      0x000000000000008A, 0x0000000000000088, 0x0000000080008009, 0x000000008000000A,
      0x000000008000808B, 0x800000000000008B, 0x8000000000008089, 0x8000000000008003,
      0x8000000000008002, 0x8000000000000080, 0x000000000000800A, 0x800000008000000A,
      0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008]
ROT = [[0, 36, 3, 41, 18], [1, 44, 10, 45, 2], [62, 6, 43, 15, 61], [28, 55, 25, 21, 56], [27, 20, 39, 8, 14]]
M = (1 << 64) - 1


def _f(a):
    for rc in RC:
        c = [a[x][0] ^ a[x][1] ^ a[x][2] ^ a[x][3] ^ a[x][4] for x in range(5)]
        d = [c[(x - 1) % 5] ^ (((c[(x + 1) % 5] << 1) | (c[(x + 1) % 5] >> 63)) & M) for x in range(5)]
        a = [[a[x][y] ^ d[x] for y in range(5)] for x in range(5)]
        b = [[0] * 5 for _ in range(5)]
        for x in range(5):
            for y in range(5):
                r = ROT[x][y]
                b[y][(2 * x + 3 * y) % 5] = ((a[x][y] << r) | (a[x][y] >> (64 - r))) & M if r else a[x][y]
        a = [[b[x][y] ^ ((~b[(x + 1) % 5][y]) & b[(x + 2) % 5][y]) for y in range(5)] for x in range(5)]
        a[0][0] ^= rc
    return a


def keccak256(data):
    rate = 136
    p = bytearray(data) + b"\x01" + b"\x00" * ((-len(data) - 1) % rate)
    p[-1] |= 0x80
    a = [[0] * 5 for _ in range(5)]
    for off in range(0, len(p), rate):
        for i in range(rate // 8):
            a[i % 5][i // 5] ^= int.from_bytes(p[off + 8 * i: off + 8 * i + 8], "little")
        a = _f(a)
    return b"".join(a[i % 5][i // 5].to_bytes(8, "little") for i in range(4))


assert keccak256(b"").hex() == "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"

# Intel SGX Root CA, downloaded separately from Intel
# (certificates.trustedservices.intel.com/Intel_SGX_Provisioning_Certification_RootCA.cer)
# and vendored, so the root in the quote is compared with Intel's own copy.
INTEL_ROOT = "vendor/intel/Intel_SGX_Provisioning_Certification_RootCA.cer"
ok_all = True


def check(ok, what):
    global ok_all
    ok_all &= bool(ok)
    print(("ok  " if ok else "FAIL"), what)


def p256(raw64):
    return ec.EllipticCurvePublicNumbers(int.from_bytes(raw64[:32], "big"), int.from_bytes(raw64[32:], "big"), ec.SECP256R1()).public_key()


def sig(raw64):
    return encode_dss_signature(int.from_bytes(raw64[:32], "big"), int.from_bytes(raw64[32:], "big"))


def verifies(key, signature, data):
    try:
        key.verify(signature, data, ec.ECDSA(hashes.SHA256()))
        return True
    except Exception:
        return False


path = next((a for a in sys.argv[1:] if not a.startswith("--")), "demo/nearai-attestation.json")
rep = json.load(open(path))
m = rep["model_attestations"][0]
addr = m["signing_address"].lower().removeprefix("0x")
nonce = m["request_nonce"]
print(f"model {m['model_name']}, signing address {addr}, nonce {nonce}")

# 1. key -> address
pub = bytes.fromhex(m["signing_public_key"])
check(keccak256(pub)[-20:].hex() == addr, "keccak-256 of the signing public key is the signing address")

# 2. cassettes signed by it
for c in ("demo/cassette.json", "demo/swarm-cassette.json"):
    try:
        cas = json.load(open(c))
    except FileNotFoundError:
        continue
    check(cas.get("signer", "").lower().removeprefix("0x") == addr and cas.get("model") == m["model_name"],
          f"{c} was recorded with this model and this signer ({len(cas.get('entries', []))} signed calls; `make verify-cassette` re-checks each signature)")

# 3. quote binds address + nonce
q = bytes.fromhex(m["intel_quote"])
version, att_type, tee_type = struct.unpack_from("<HHI", q, 0)
check(version == 4 and tee_type == 0x81, f"Intel quote is a TDX v4 quote ({len(q)} bytes)")
rd = q[568:632]
check(rd[:20].hex() == addr and rd[20:32] == b"\0" * 12, "the quote's report_data starts with the signing address")
check(rd[32:].hex() == nonce, "the quote's report_data ends with our nonce (fresh for this request)")

# 4. signature chain
(sig_len,) = struct.unpack_from("<I", q, 632)
s = q[636:636 + sig_len]
quote_sig, att_key = s[:64], s[64:128]
check(verifies(p256(att_key), sig(quote_sig), q[:632]), "the quote is signed by its attestation key")
ctype, csize = struct.unpack_from("<HI", s, 128)
cd = s[134:134 + csize]
qe_report, qe_sig = cd[:384], cd[384:448]
(auth_len,) = struct.unpack_from("<H", cd, 448)
auth = cd[450:450 + auth_len]
inner_type, inner_size = struct.unpack_from("<HI", cd, 450 + auth_len)
pem = cd[456 + auth_len:456 + auth_len + inner_size]
check(ctype == 6 and inner_type == 5, "certification data: QE report + PCK certificate chain")
check(qe_report[320:352] == hashlib.sha256(att_key + auth).digest(), "the Quoting Enclave report binds the attestation key")
certs = x509.load_pem_x509_certificates(pem)
pck = certs[0]
check(verifies(pck.public_key(), sig(qe_sig), qe_report), "the QE report is signed by the PCK certificate")
chain_ok = True
for child, parent in zip(certs, certs[1:]):
    try:
        child.verify_directly_issued_by(parent)
    except Exception:
        chain_ok = False
root = certs[-1]
try:
    root.verify_directly_issued_by(root)
except Exception:
    chain_ok = False
check(chain_ok, "PCK chain: " + " <- ".join(c.subject.get_attributes_for_oid(x509.oid.NameOID.COMMON_NAME)[0].value for c in certs) + " (each signed by the next; root self-signed)")
root_sha = hashlib.sha256(root.public_bytes(__import__("cryptography.hazmat.primitives.serialization", fromlist=["Encoding"]).Encoding.DER)).hexdigest()
if "--print-root" in sys.argv:
    print("root sha256", root_sha)
try:
    pinned = hashlib.sha256(open(INTEL_ROOT, "rb").read()).hexdigest()
    check(root_sha == pinned, f"the chain's root is Intel's own SGX Root CA ({INTEL_ROOT}, sha256 {pinned[:16]}...)")
except FileNotFoundError:
    check(False, f"{INTEL_ROOT} missing: download it from Intel to pin the root")

# 5. GPU evidence nonce
nv = json.loads(m["nvidia_payload"])
check(nv.get("nonce") == nonce, f"NVIDIA GPU evidence carries the same nonce ({len(nv.get('evidence_list', []))} GPUs)")

print("not checked offline: Intel TCB status and revocation, NVIDIA certificate chain and measurements (need online collateral)")
sys.exit(0 if ok_all else 1)

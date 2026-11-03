# Security model

Drop It is designed for direct, local-network use. It has no account service,
analytics endpoint, cloud storage, or relay in this release.

## What is protected

- The transfer server uses TLS 1.3 with a self-signed certificate.
- Pairing codes expire after five minutes, are single-use, and lock after three
  failed attempts.
- Pairing code material has SPAKE2/HKDF/ChaCha20-Poly1305 primitives and TOFU
  certificate-fingerprint checks.
- Received paths are sanitized and transfer requests check available disk space.
- Locally persisted chat history is encrypted with Windows DPAPI on Windows.

## Important current limitations

- Direct LAN peers necessarily learn each other's network address.
- The relay-only mode is not implemented and must remain disabled.
- Chat transport integration is still under active development; do not treat the
  experimental messenger API as a replacement for a reviewed secure messenger.
- Drop It does not protect a compromised Windows account, a compromised endpoint,
  screen capture, or malicious files after a user accepts them.

## Reporting a vulnerability

Do not open a public issue with exploit details. Contact the repository owner
privately through GitHub first, including affected version, reproduction steps,
and impact. Allow reasonable time for a fix before disclosure.

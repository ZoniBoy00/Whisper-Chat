# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.1.x   | :white_check_mark: |

Whisper is currently in pre-beta. Security fixes are applied to the latest
`main` branch and will be included in the next release.

## Reporting a Vulnerability

**Please do NOT open a public GitHub issue for security vulnerabilities.**

If you discover a security vulnerability in Whisper, please report it
responsibly:

1. **Email**: Send details to the maintainer privately via GitHub's
   [private vulnerability reporting](../../security/advisories/new) feature.
2. **Include**:
   - Description of the vulnerability
   - Steps to reproduce or proof of concept
   - Potential impact assessment
   - Suggested fix (if any)
3. **Response time**: We aim to acknowledge reports within **48 hours** and
   provide an initial assessment within **7 days**.
4. **Disclosure**: We will coordinate public disclosure with you after a fix
   is available. Credit will be given unless you prefer to remain anonymous.

## Scope

### In Scope

- Cryptographic implementation flaws (X3DH, Double Ratchet, Megolm)
- Authentication bypasses or identity spoofing
- Server-side vulnerabilities (relay, WebSocket handling, rate limiting)
- Local storage encryption weaknesses (SQLCipher, backup encryption)
- Dependency vulnerabilities with demonstrable impact
- Privacy leaks (metadata exposure beyond documented threat model)

### Out of Scope

- Social engineering attacks against users
- Physical access to unlocked devices
- Vulnerabilities in upstream dependencies without a viable attack path
- Issues already known and tracked in public issues

## Threat Model

Whisper's security relies on the following assumptions:

- **The server is zero-knowledge** — it never sees plaintext, keys or message
  content. Compromising the relay exposes only routing metadata and opaque
  ciphertext envelopes.
- **Client devices are trusted** — if an attacker controls the device, all
  bets are off. Whisper protects data *in transit* and *at rest*, not against
  malware on the endpoint.
- **vodozemac is correct** — we rely on the audited vodozemac library for all
  cryptographic operations. We do not implement our own primitives.
- **TLS is secure** — transport security assumes TLS 1.2/1.3 with valid
  certificates. Certificate pinning is planned but not yet implemented.

For a detailed security model, see the [README](README.md#security-model).

## Security Best Practices for Contributors

- **Never commit secrets** — `.env`, identity files, database files and local
  test artifacts must never enter version control.
- **No hand-rolled crypto** — use vodozemac exclusively. Proposals for new
  cryptographic primitives require external audit.
- **Test before merge** — all crypto changes require tests. No exceptions.
- **Hardened builds** — release profile uses `lto="fat"`, `panic="abort"`,
  `strip=true`. Do not weaken these settings.
- **Wire compatibility** — never change serialised formats without updating
  all consumers (server, desktop, mobile, smoke tests) in the same change.

## Acknowledgements

We thank all security researchers who responsibly disclose vulnerabilities.
Contributors will be credited here (with permission) after a fix is released.

</content>
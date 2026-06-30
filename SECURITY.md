# Security Policy

Ziplark unpacks untrusted archives, so security matters. Two things are core to
the design:

- **Path-traversal / "zip slip" guard.** Every extraction path — for every
  format — is funnelled through a single `safe_join` check that rejects
  absolute paths, `..` components and drive prefixes, so no archive entry can
  ever escape the destination directory.
- **No telemetry, no network.** Ziplark does not phone home.

## Reporting a vulnerability

Please report security issues **privately** via GitHub's
[**Report a vulnerability**](https://github.com/zhitongblog/ziplark/security/advisories/new)
flow rather than opening a public issue. We aim to acknowledge reports within a
few days. Responsible disclosure is appreciated and reporters will be credited
(with permission).

## Supported versions

The latest released version receives security fixes.

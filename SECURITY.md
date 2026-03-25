# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Reporting a Vulnerability

If you discover a security vulnerability in BearWisdom, please report it responsibly.

**Do not open a public GitHub issue for security vulnerabilities.**

Instead, email the maintainer directly or use [GitHub's private vulnerability reporting](https://github.com/MariusAlbu/BearWisdom/security/advisories/new).

### What to include

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

### Response timeline

- **Acknowledgement**: within 48 hours
- **Assessment**: within 1 week
- **Fix**: targeted within 2 weeks for critical issues

## Scope

BearWisdom is a **local-only** tool. It does not expose network services by default — the web server (`bw-web`) binds to `0.0.0.0` only when explicitly started. Areas of security concern include:

- **SQL injection** via the SQLite query layer (mitigated: all queries use parameterised statements)
- **Path traversal** via the `/api/file-content` and `/api/browse` endpoints
- **Dependency vulnerabilities** in tree-sitter grammars or Rust/Node.js dependencies

## Dependencies

We track dependency advisories through `cargo audit` and Dependabot. If you notice a vulnerable dependency, please report it.

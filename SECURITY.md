# Security Policy

## Supported Versions

We release security updates for the following versions:

| Version | Supported          |
| ------- | ------------------ |
| 0.3.x   | :white_check_mark: |
| < 0.3   | :x:                |

## Reporting a Vulnerability

We take the security of pytest-language-server seriously. If you believe you have found a security vulnerability, please report it to us as described below.

### How to Report

**Please do NOT report security vulnerabilities through public GitHub issues.**

Instead, please report them via email to:

- **Email**: hackedbellini@gmail.com
- **Subject**: [SECURITY] pytest-language-server vulnerability report

Please include the following information in your report:

- Type of vulnerability
- Full paths of source file(s) related to the manifestation of the issue
- The location of the affected source code (tag/branch/commit or direct URL)
- Any special configuration required to reproduce the issue
- Step-by-step instructions to reproduce the issue
- Proof-of-concept or exploit code (if possible)
- Impact of the issue, including how an attacker might exploit it

### What to Expect

- **Acknowledgment**: We will acknowledge receipt of your vulnerability report within 48 hours.
- **Updates**: We will send you regular updates about our progress, at minimum every 7 days.
- **Disclosure Timeline**: We aim to disclose vulnerabilities within 90 days of the initial report.
- **Credit**: We will credit you in the security advisory unless you prefer to remain anonymous.

### Security Update Process

1. The security team will investigate and validate the vulnerability
2. A fix will be developed in a private repository
3. A new version will be released with the fix
4. A security advisory will be published on GitHub
5. The CVE (if applicable) will be requested and published

## Security Best Practices for Users

### Installation

- Always install from official sources (PyPI, Homebrew, or crates.io)
- Verify checksums when downloading pre-built binaries
- Use the latest stable version

### Running the Server

- Run the LSP server with the minimum required privileges
- Do not expose the LSP server to untrusted networks
- Be cautious when opening untrusted workspace directories
- Review the workspace before allowing the server to scan it

### Known Limitations

- The server scans all Python files in the workspace recursively
- The server reads contents of test files and conftest.py files
- The server may follow symlinks in the workspace
- Virtual environment scanning may access third-party code

## Security Measures

### Development

- All code changes are reviewed before merging
- We use automated security scanning in CI/CD:
  - `cargo audit` for known vulnerabilities
  - `cargo deny` for license compliance and security policies
  - `cargo clippy` for code quality and potential issues
  - Dependency review on pull requests
- GitHub Actions are pinned to specific commit SHAs
- We use GitHub's security features (Dependabot, security advisories)

### Build Process

- Builds are reproducible via Cargo.lock
- Release artifacts include build provenance attestations
- PyPI releases use trusted publishing with OIDC

### Dependencies

- We minimize the dependency tree
- Dependencies are regularly updated
- Unmaintained dependencies are monitored and replaced when necessary

## Security Auditing

We perform regular security audits:

- **Daily**: Automated dependency vulnerability scanning
- **Weekly**: Manual review of security alerts
- **Per Release**: Full security review before each release

## Responsible Disclosure

We believe in responsible disclosure and will work with security researchers to:

- Understand and reproduce the vulnerability
- Develop and test a fix
- Coordinate disclosure timing
- Provide credit in security advisories

## Contact

For security-related questions or concerns that do not relate to a vulnerability, you can:

- Open a GitHub discussion (for general security questions)
- Email: hackedbellini@gmail.com (for sensitive matters)

## Hall of Fame

We thank the following researchers for responsibly disclosing security issues:

<!-- This section will be updated as researchers report vulnerabilities -->

*No vulnerabilities reported yet. Be the first!*

---

**Last Updated**: 2025-01-15

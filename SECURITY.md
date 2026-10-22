# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do not open a public issue** for security problems.
2. Email details to the maintainer directly (check GitHub profile for contact).
3. Include: affected version, steps to reproduce, impact assessment, and any suggested fix.

We will acknowledge receipt within 72 hours and provide a timeline for resolution. Public disclosure will be coordinated after a fix is available.

## Safety Notes

- This application uses Windows `SendInput` for keyboard/mouse injection.
- It requires running at the same or higher integrity level as the target application (UIPI).
- No network access, no data collection, no telemetry.
- Game anti-cheat systems may detect input injection; use in games at your own risk.
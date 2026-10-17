# Security Policy

## Reporting a vulnerability

Please do not open a public issue for a security problem.

Report it privately in one of two ways:

- Use GitHub's private vulnerability reporting: open the
  [Security tab](https://github.com/commy-commander/commy/security) of this
  repository and choose **Report a vulnerability**.
- Email commy.cmd@commy.chat with the subject "Security".

Include what you found, the version or commit, steps to reproduce, and what an
attacker could do with it. A proof of concept helps. Please do not include real
user data.

You will get a reply within 7 days. We will keep you updated while we confirm
and fix the problem, and we are happy to credit you in the release notes if you
want that.

## Supported versions

Commy is young and moves quickly. Security fixes go into the latest release and
the `main` branch. Please reproduce a problem on the latest release before
reporting it.

## Scope

In scope:

- The desktop app (`apps/desktop`) and the Rust crates (`crates/`)
- The account server (`auth/`) and the self-hosting files (`deploy/`)
- Anything that could expose messages, files, voice, video, screen shares,
  invite links, or which servers someone belongs to

Out of scope:

- Findings that need a device or account the attacker already controls
- Denial of service through sheer traffic volume
- Problems in third-party software with no Commy-specific impact; report those
  upstream

## Good faith

If you act in good faith, stay within these rules, and give us reasonable time
to fix a problem before sharing details, we will not take action against you.
Please only test against your own accounts and servers.

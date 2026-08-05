# Security Policy

## Reporting a vulnerability

Please do not open a public issue for a security vulnerability.

Use GitHub's **Report a vulnerability** flow in the repository's Security tab
so the report stays private. Include a clear description, affected commit or
version, reproduction steps, impact, and a suggested mitigation if known.

If the private reporting flow is unavailable, contact the maintainer through
the GitHub profile for `carterlasalle` and do not disclose credentials,
identities, hostnames, or command output in public channels.

## Sensitive data

sessionwatch is an administrative observability tool. When run with elevated
permissions it can read process arguments, Tailscale SSH identities, wtmp/btmp
records, and users' shell history files. Treat its screen output and journal as
sensitive operational data. Redact those values from bug reports and logs.

The project does not promise that collected command lines or history are safe
to disclose. Operators are responsible for access control on the host and on
the journal path.

## Supported versions

The latest commit on `main` is the supported development version. Security
fixes are released through GitHub tags when applicable.

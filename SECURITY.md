# Security reporting

Do not put credentials, personal vessel tracks, private scenes, or restricted
datasets in a public issue. Report a vulnerability through the repository's
GitHub **Security → Report a vulnerability** form when available. If private
reporting is unavailable, open an issue requesting a private reporting channel
without exploit details or sensitive attachments.

Include the affected commit, platform, a minimal synthetic reproducer, impact,
and any workaround. This is an active development project; no supported stable
release or response-time guarantee is currently offered.

Store an optional AIS provider key through the application, outside the source
tree. A `--test-storage=<directory>` session uses that directory for preferences,
data caches and keys; it does not load your regular application credentials.

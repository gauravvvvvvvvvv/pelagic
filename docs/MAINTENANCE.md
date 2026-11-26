# Maintenance and triage

Pelagic source and its supported engine API work offline independently of optional
external services. No hosted service is needed to open projects or run experiments.

File a local/repository issue using the supplied templates. This checkout has no
published support mailbox or staffed response-time agreement. The project owner
controls release publication and triage; no external team or service is implied.

Priority order: corrupted/lost projects or recordings, crashes/security boundary
errors, wrong numerical results, supported-workflow failures, performance regression,
then cosmetic/features. A release is blocked by known save/recovery/integrity failures.
Issues should carry a minimal synthetic scene, expected/observed result and build ID.
Preserve a copy of the affected file before attempting migration or repair.

Each patch reruns formatting, strict Clippy, release tests, native smoke checks and
affected measurements. Format migrations need both old fixtures and round trips.
ABI version 1 stays unchanged for compatible additions; breaking ABI changes get a
new version. Same-build replay verification and portable truth playback are distinct.

Dependency updates review the lockfile diff, advisory status and licence texts,
then rerun native startup and SDK loading: a compile-only check missed the import
library problem found during this release. Assets must retain credits and licences.
Public release publication requires trusted hashes and the signing decision described
in DISTRIBUTION.md. Local CI definitions are supplied; a hosted run is not implied
until the repository is pushed and its runner reports success.

# CEP:FILE: quarantine/README.md
# CEP:WHAT: Policy for the quarantine directory holding non-compliant code awaiting repair or deletion.
# CEP:WHY: Standard sections 32.1 and 34.4.3 require quarantine to exist, be excluded from builds, and carry deadlines.
# CEP:CLASS: CC
# CEP:STATUS: complete
# CEP:FAILURE: none
# CEP:ASSUMES: entries are removed within 30 days or deleted per section 34.4.4.
# CEP:COST: none; empty directory.
# CEP:EVIDENCE: review 2026-10-30.

Quarantine entries are forbidden in builds. Each entry must carry a deadline header:
CEP:DEADLINE: YYYY-MM-DD. Expired entries are deleted per standard 34.4.4.

Currently empty.

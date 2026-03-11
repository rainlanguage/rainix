# Audit Triage — 2026-03-12-01

## Pass 0: Process Review
| ID | Severity | Title | Status |
|----|----------|-------|--------|
| A01-1-p0 | LOW | Inaccurate CI platform description in CLAUDE.md | FIXED |
| A01-2-p0 | LOW | Task path prefix varies by working directory | PENDING |
| A01-3-p0 | LOW | Subgraph tasks not runnable via `nix run` | PENDING |

## Pass 1: Security
No LOW+ findings.

## Pass 2: Test Coverage
| ID | Severity | Title | Status |
|----|----------|-------|--------|
| A01-1-p2 | LOW | No test for increment overflow behavior | PENDING |
| A01-2-p2 | LOW | No test for consecutive increments | PENDING |
| A02-1-p2 | LOW | No CI coverage for subgraph tasks | PENDING |
| A02-2-p2 | LOW | Default dev shell not tested in check-shell.yml | PENDING |

## Pass 3: Documentation
| ID | Severity | Title | Status |
|----|----------|-------|--------|
| A01-1-p3 | LOW | No README.md exists | PENDING |
| A01-2-p3 | LOW | No comments on exported reusable outputs in flake.nix | PENDING |

## Pass 4: Code Quality
| ID | Severity | Title | Status |
|----|----------|-------|--------|
| A01-1-p4 | LOW | Pragma version mismatch between source and test | PENDING |
| A02-1-p4 | LOW | Unused import: console2 | PENDING |
| A03-1-p4 | LOW | Pragma version mismatch — Deploy.sol | PENDING |
| A04-1-p4 | LOW | goldsky SHA256 identical for x86_64-darwin and aarch64-darwin | PENDING |

## Pass 5: Correctness
| ID | Severity | Title | Status |
|----|----------|-------|--------|
| A03-1-p5 | LOW | Deploy.run() broadcasts nothing | PENDING |
| A04-1-p5 | MEDIUM | rainix-rs-prelude is a no-op but CI runs it | DOCUMENTED |
| A04-2-p5 | LOW | CLAUDE.md documents no-op but flake has no comment | PENDING |

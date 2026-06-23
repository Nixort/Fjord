# Fjord — commit / patch history

This skeleton was built as an ordered patch series so the history is
reviewable step by step rather than one big drop. Apply with:

    git am patches/*.patch

## Commits (oldest first)

- 2026-06-23  chore: bootstrap Fjord workspace scaffolding
- 2026-06-23  docs: add README and full ARCHITECTURE overview
- 2026-06-23  docs: add detailed phased roadmap
- 2026-06-23  docs: add CONTRIBUTING, SECURITY policy and glossary
- 2026-06-23  feat(keel): microkernel skeleton (cap, vspace, ipc, tide, untyped)
- 2026-06-23  feat(hull): hardware abstraction layer skeleton
- 2026-06-23  feat(anchor): secure boot + DICE skeleton
- 2026-06-23  feat(helm): root supervisor + Cask launch gate
- 2026-06-23  feat(cask): tamper-evident executable format skeleton
- 2026-06-23  feat(lading): signed manifest + license model skeleton
- 2026-06-23  feat(brine): authenticated disk encryption skeleton
- 2026-06-23  feat(harbormaster): MFA authentication + authorization skeleton
- 2026-06-23  feat(logbook): transparency-log client skeleton
- 2026-06-23  feat(services): cryptd, storaged, vfs, netd, timed skeletons
- 2026-06-23  feat(fjord-rt): async runtime skeleton
- 2026-06-23  feat(libfjord): userspace capability bindings skeleton
- 2026-06-23  build(shipwright): host build orchestrator skeleton


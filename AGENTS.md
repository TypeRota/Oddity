# Oddity

## Project

Oddity is a Linux launcher for *Neverness To Everness* (NTE).

### Arch

- `oddity-platform`: Platform primitives shared by the launcher and game packages: HTTP, files, hashing, archives, diffs, INI parsing, directory trees, Wine path rewriting, audio, network metering, clock, process, locks, keyring and desktop notifications.
- `oddity-contract`: The single declaration source for commands and events, and the basis for the generated Electron allowlist, renderer TypeScript types, browser demo command keys and `launcher/CONTRACTS.md`.
- `oddity-runtime`: Runtime contracts shared by the host and game packages: command groups, tasks, install domains, launch and assembly.
- `oddity-host`: Host kernel, command dispatch, sessions and all host domains; independent of Electron and game packages.
- `oddity-backend`: Backend binary (`oddity-backend`); the backend assembly point for game packages and the process startup order; its lib target is reused by `oddity-cli`.
- `oddity-cli`: Headless CLI (binary `nte-cli`) built on `oddity-host`, `oddity-runtime` and the backend assembly; dispatches package commands through the package CLI surface.
- `oddity-recovery`: Native emergency recovery window (binary `oddity-recovery`), independent of Electron.
- `launcher/packages`: Game packages (Rust backend and Vue UI); each declares its command groups and UI contributions, and playable games also supply their install domain and launch implementation.
- `launcher/ui/electron`: Electron main process and isolated preload.
- `launcher/ui`: Vite + TypeScript + Vue 3 frontend.

Prefer reusable logic in `oddity-host` over duplication.

## Code and Repository Rules

1. Do not write implementation status, collaboration progress, milestones, decisions, reflections, or memos in code comments.
2. PRs must reference Issues using `Closes`, `Refs`, or `Fixed`, instead of **fucking** using dogshit as a prefix.
3. PR descriptions must not mention your own coverage tests or unit tests!
4. Do not fabricate author information.
5. Do not fucking introduce any explanations related to old decisions in any text. Absolutely fucking prohibited!
6. Create a worktree based on the latest `origin/main`. Do not modify files directly in the existing workspace.
7. Do not disclose any personal information, **including directory structures**!
8. Issue replies must be brief and clear, instead of treating them like a **fucking dumpster for spewing bullshit**.
9. Issue bodies must be concise and clear. Don't fucking tell other people what to do. Fuck you, nobody wants to hear your endless stream of worthless bullshit!
10. Do not push untracked directories to the remote just because you think they should be uploaded!
11. Do not write your stupid progress reports or research reports into `docs/` or `internal/`!
12. The repository is a public space. Think carefully about whether what you're writing is intended for contributors. Content intended for your user must never appear in the repository!

## Working Principles

- Understand the relevant code before making changes.
- Focus on the intended outcome rather than following a rigid procedure.
- Prefer simple, maintainable solutions consistent with the existing architecture.
- Comments should be concise, natural Chinese and explain intent rather than restating code.
- User-facing text should use formal Chinese with consistent terminology and ASCII punctuation.
- Use judgment when adding tests. Test meaningful behavior, not implementation details.
- Verify relevant changes when practical, and report anything left unverified.

## Technical Research

When encountering unfamiliar functionality, do not immediately implement it from scratch. First search GitHub for similar open-source projects that are actively maintained and have clear licensing, and compare their implementation approaches, dependencies, and known issues. Prefer proven designs; do not directly copy incompatible code or code of unknown origin.

## Implementation Rules

- All animation uses Motion For Vue. Anything involving animation choreography uses Anime.js. Never hand-roll animation in CSS.
- No filler in user-facing copy. Players do not want to read it. Every sentence must tell them something they need.
- Before writing a component, check the existing files for an identical or similar one. Do not build from scratch when one already exists.
- If the implementation uses or borrows from an open source project, record it in [THIRD_PARTY.md](THIRD_PARTY.md).

## Branches and PRs

- Before creating a branch or submitting a PR, confirm that your branch is based on the latest `origin/main`.
- Before opening a PR, follow the PR title rules in [CONTRIBUTING.md](CONTRIBUTING.md), using either the module form or the semantic form.
- Do not use PRs as a dumping ground for irrelevant remarks that are not meant for maintainers.

## Decision Management

**The current decision is the only authoritative decision.**

- Describe the current design directly. Do not narrate previous decisions, rejected alternatives, reversals, or decisions not taken.
- Do not introduce regression tests merely to demonstrate that an abandoned decision has not been restored.
- When a decision changes, remove its superseded implementation, configuration, documentation, comments, and obsolete tests.
- Do not retain legacy branches, compatibility layers, fallback paths, or historical explanations solely to preserve an abandoned decision.

## Completion

Briefly report what was implemented, what was verified, and any remaining issues.


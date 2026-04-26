## Summary

<!-- 1–3 sentences: what changed and why. -->

## Type of change

<!-- Mark with [x] -->

- [ ] Bug fix (`fix:` / `bugfix/*` branch)
- [ ] New feature (`feat:` / `feature/*` branch)
- [ ] New site extractor (`feat(extractor):` / `feature/*` branch)
- [ ] Refactor (`refactor:` / `chore/*` branch)
- [ ] Documentation (`docs:` / `chore/*` branch)
- [ ] Chore / tooling (`chore:` / `chore/*` branch)
- [ ] Performance (`perf:`)
- [ ] Test (`test:`)

## Test plan

<!-- How did you verify the change? Include commands run + results. -->

- [ ] `cargo build`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy -- -W clippy::all`
- [ ] `cargo fmt --check`
- [ ] (If desktop crate touched) `cd crates/rdlp-desktop && npx tsc --noEmit && npm test -- --run`
- [ ] Manual verification (describe the scenario)

## Checklist

- [ ] Branch name conforms to `gitflow-branch-policy` (`feature/*` / `bugfix/*` / `chore/*` / `spike/*` / `release/*` / `hotfix/*`, lowercase kebab-case after the prefix, ≤ 72 chars)
- [ ] Commits follow [Conventional Commits](https://www.conventionalcommits.org/)
- [ ] If a new extractor: legal/ToS check done, test fixtures recorded via `rdlp-probe`, no live-network dependency in tests
- [ ] If a new dependency: justified in the PR description, license compatible with `MIT OR Apache-2.0`
- [ ] If a behavior change: docs / `CLAUDE.md` updated
- [ ] (Implicit) By submitting this PR I agree my contribution is dual-licensed under `MIT OR Apache-2.0` per the repo's contribution clause

# Decisions to be made

Open design questions surfaced in conversation but not yet committed to. Each entry: context, options, current status.

## 1. Escape hatch for the reentrancy rule

**Context.** The CEI rule forbids legitimate patterns (oracle-then-write, cross-contract composability with return values, callback-driven finalization, pull-pattern post-bookkeeping). Developers will sometimes need to opt out of the check on a specific block.

**Options.**

- Per-rule attribute: `#[pvmsafe::allow(reentrancy)]` on the specific offending block. Auditable via `grep`, scales to future rules, mirrors Rust's own `#[allow(...)]` ecosystem.
- Broad keyword: `pvmsafe::unsafe { ... }`. Simpler to spell, but waives every pvmsafe rule at once and doesn't generalize as the rule set grows.

**Current status.** Agreed on per-rule allow attributes. Not implemented. Ships when a concrete use case appears.

## 2. Gradual vs total refinement-type discipline

**Context.** pvmsafe is currently opt-in: unannotated parameters have no proof obligations and FM never runs on them. A developer who forgets to annotate gets zero safety — same failure mode as TypeScript's `any`. Decision made: we should make entirely mandatory unless someone explicily skip it. The reason about it is that if someone is using my language he want those features.

**Options.**

- **Gradual (today).** Only annotated code is checked. Pitch: "the parts you annotate, we prove rigorously." Matches how Liquid Haskell, F\*, Dafny are used in practice.
- **Total with implicit `true`.** Every parameter implicitly refined by `true`. Same runtime behavior as gradual, but opens the door to a future lint/warning asking the developer to refine public entrypoints.
- **Mandatory refinements on entrypoints.** `#[pvm_contract_macros::method]`-annotated functions must have a refinement on every non-`self` parameter, even if it's `true`. Forces a social discipline at the type level.

**Current status.** Decided 2026-04-15 and implemented: mandatory on entrypoints (`#[pvm_contract_macros::{constructor, method, fallback}]`), explicit `#[pvmsafe::unchecked]` opt-out for non-refinable parameter types (e.g. `Address`). Internal helpers remain gradual.

## 3. Path-sensitive reasoning (branch conditions as assumptions)

**Context.** Today the assumption context `Γ` is exactly the declared refinements on the caller's parameters. A runtime check like `if amount > 0 { assume_positive(amount) }` is not enough to typecheck the call, because the `if amount > 0` branch condition is not threaded into `Γ`.

**Options.**

- **Add path sensitivity.** Walk the body with a mutable `Γ` that gains a constraint on entering a `then` branch and the negation on entering an `else` branch. Pure extension of the existing FM machinery, no new decision procedure.
- **Keep declaration-only.** Force the developer to refine at parameter level, never at runtime. Simpler but rejects valid patterns.

**Current status.** Implemented: if/else branch conditions threaded into Γ with snapshot/restore, early-return guards (`if cond { return; }`) persist `¬cond` for subsequent statements, mutation invalidation drops stale assumptions.

## 4. Non-linear refinements

**Context.** FM decides linear integer arithmetic. Refinements like `x * y > 0` (two unknowns multiplied), modular wraparound, `keccak(...)`, or anything quadratic are outside the decidable fragment. Translator currently rejects with `non-linear expression`.

**Options.**

- **Hard-reject (today).** Compile error, actionable message. Forces developers to rewrite or not annotate.
- **Admit with a warning.** Store the predicate, skip it during checking, emit a warning. Unsound by omission — the user might think it was checked.
- **SMT fallback.** Escalate to Z3 for non-linear cases. Breaks the "no SMT" pitch and the decidability guarantee.

**Current status.** Hard-reject. Changing this would change the project's identity; requires an explicit decision.

## 5. Refinements on return types and locals

**Context.** Only function parameters can carry refinements today. Return types (`-> {v:U256 | v > 0}`) and `let`-bound locals are unannotated.

**Options.**

- **Extend to return types.** A function can declare what it produces, callers get that as an assumption on the returned binding. Standard in Liquid Haskell / F\*.
- **Extend to locals.** `let x: {v:U256 | v > 0} = ...` with a proof obligation at the binding site.
- **Keep parameters-only.** Smallest surface, matches the DAO-style reentrancy story, defers the annotation question for produced values.

**Current status.** Implemented: `#[pvmsafe::ensures(pred)]` on functions (caller-side injection + implementer-side proof obligation at return sites), `#[pvmsafe::refine(pred)] let x = expr;` on let bindings (proof obligation at binding site, assumption injection for subsequent code). Both integrate with path sensitivity, mutation invalidation, and `?` operator.

## 6. Arithmetic safety

**Context.** Smart contracts are vulnerable to integer arithmetic bugs: subtraction underflow, addition/multiplication overflow, division/modulo by zero.

**Approach.**

- **Subtraction underflow**: for `a - b`, prove `a >= b` from Γ via Fourier-Motzkin. Dischargeable by parameter refinements, if-guards, early-return guards, `given`, or let-binding refinements.
- **Addition/multiplication overflow**: flag `a + b` and `a * b` between non-literal operands. Cannot prove `a + b <= MAX` without expressing the type's maximum in linear arithmetic. Fix: `checked_add`/`saturating_add`/`checked_mul`/`saturating_mul`. Suppressible by `given`.
- **Division/modulo by zero**: for `a / b` and `a % b`, prove `b > 0` from Γ via Fourier-Motzkin. Same discharge mechanisms as subtraction.

**Current status.** Implemented. All four operations checked. Literal-only expressions skipped (compiler handles those).

## 7. Cross-module and cross-contract checking

**Context.** The call-site checker only fires inside a single `#[pvmsafe_macros::contract]` module. Calls to other contracts (`api::call(...)`) are opaque — we don't know the callee's refinements.

**Options.**

- **Intra-module only (today).** Matches the rest of pvmsafe's scoping (reentrancy is same-method only). Honest.
- **Trait-based interface annotations.** Define a refined Rust trait for a remote contract's ABI; the caller proves obligations against the declared trait. Needs a shared schema source.
- **On-chain ABI with embedded refinements.** Out of scope for the near term.

**Current status.** Intra-module. Cross-contract is explicitly in the "what's not yet verified" list.

## 8. Test harness — `trybuild`

**Context.** Negative tests today are manual: edit a file, build, observe the error, revert. Unit tests cover the checker on hand-crafted `ItemMod` but not the full proc-macro surface.

**Options.**

- **Keep manual (today).** Fast for a live demo. Brittle as the rule set grows.
- **Add `trybuild`.** Pinned error snapshots, CI can fail on regressions. One more dev dependency.

**Current status.** Manual. Add `trybuild` when the cost of an accidental regression exceeds the setup cost.

## 9. Stable Rust support

**Context.** The example requires `#![feature(stmt_expr_attributes)]` because we put `#[pvmsafe::locally]` on statement-position blocks. Nightly-only.

**Options.**

- **Nightly (today).** The PolkaVM target already requires nightly, so the gate is free for contract authors.
- **Drop statement-block attributes.** Switch to function-like macros (`pvmsafe::locally! { ... };`). Works on stable, worse UX.
- **Wait for stabilization.** `stmt_expr_attributes` has been nightly for years; no clear timeline.

**Current status.** Nightly. Only revisit if stable Rust users complain.

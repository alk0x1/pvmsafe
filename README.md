# pvmsafe

Compile-time verification for `pallet-revive` smart contracts, as a Rust
proc-macro crate. Wraps `pvm_contract_macros::contract` from
[paritytech/cargo-pvm-contract](https://github.com/paritytech/cargo-pvm-contract)
and rejects unsafe contracts at `cargo build` time.

## What it checks

- **Refinement types** — `refine` / `ensures` / `given` predicates on
  parameters, returns, lets, and blocks. Discharged by Fourier–Motzkin
  entailment over linear integer arithmetic.
- **Arithmetic safety** — every unchecked `+`, `-`, `*`, `/`, `%` is
  rejected unless provably safe. Subtraction/division require an explicit
  proof obligation; addition/multiplication require `checked_*` /
  `saturating_*`.
- **Conservation of value** — `#[pvmsafe::invariant(conserves)]` on the
  module plus `#[pvmsafe::delta(expr)]` on each storage write. On every
  path through an entrypoint, declared deltas must sum to zero (per group).
- **Effect types & CEI ordering** — `#[pvmsafe::effect(read|write|call|emit|revert)]`
  on functions. Call-graph fixpoint infers effects for callers; a CFG
  walker rejects any `write` or `emit` after a `call`, blocking reentrancy
  by construction. Opt out per-rule with `#[pvmsafe::effect_allow(...)]`.
- **Entrypoint coverage** — every entrypoint parameter must carry
  `#[pvmsafe::refine(...)]` or `#[pvmsafe::unchecked]`. No silent holes.

## Zero runtime cost

All `pvmsafe::*` attributes are stripped before the inner
`pvm_contract_macros::contract` macro sees the module. PolkaVM bytecode is
identical to an unannotated build.

## Usage

```rust
use pvmsafe::contract;

#[pvmsafe::contract]
#[pvmsafe::invariant(conserves)]
mod token {
    #[pvmsafe::effect(write)]
    pub fn transfer(
        #[pvmsafe::refine(amount > 0)] amount: U256,
        to: Address,
    ) -> Result<(), Error> {
        let sender = load(&balance_key(&caller));
        if sender < amount { return Err(Error::Insufficient); }

        #[pvmsafe::refine(v >= amount)]
        let safe = sender;

        #[pvmsafe::delta(-amount)]
        store(&balance_key(&caller), safe - amount);

        #[pvmsafe::delta(amount)]
        store(&balance_key(&to), load(&balance_key(&to)) + amount);
        Ok(())
    }
}
```

A working end-to-end example lives in [pvmsafe-example/](pvmsafe-example/).

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

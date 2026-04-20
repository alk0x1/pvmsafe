import { useEffect, useState } from "react";
import { Highlight, themes } from "prism-react-renderer";

type Section = {
  id: string;
  title: string;
  children?: { id: string; title: string }[];
};

const TOC: Section[] = [
  {
    id: "overview",
    title: "Overview",
  },
  {
    id: "install",
    title: "Install",
  },
  {
    id: "quickstart",
    title: "Quick start",
  },
  {
    id: "attributes",
    title: "Attributes",
    children: [
      { id: "attr-contract", title: "contract" },
      { id: "attr-refine", title: "refine" },
      { id: "attr-ensures", title: "ensures" },
      { id: "attr-invariant", title: "invariant" },
      { id: "attr-delta", title: "delta" },
      { id: "attr-given", title: "given" },
      { id: "attr-unchecked", title: "unchecked" },
      { id: "attr-effect", title: "effect" },
      { id: "attr-effect-allow", title: "effect_allow" },
    ],
  },
  {
    id: "rules",
    title: "Verification rules",
    children: [
      { id: "rule-arith", title: "Arithmetic safety" },
      { id: "rule-refine", title: "Refinement discharge" },
      { id: "rule-conserve", title: "Conservation of value" },
      { id: "rule-cei", title: "CEI ordering" },
      { id: "rule-entry", title: "Entrypoint coverage" },
    ],
  },
  {
    id: "errors",
    title: "Compile errors",
  },
  {
    id: "examples",
    title: "Example contracts",
  },
];

function Code({ code, lang = "rust" }: { code: string; lang?: string }) {
  return (
    <Highlight theme={themes.vsDark} code={code} language={lang}>
      {({ className, style, tokens, getLineProps, getTokenProps }) => (
        <pre className={className} style={style}>
          {tokens.map((line, i) => (
            <div {...getLineProps({ line })} key={i}>
              {line.map((token, key) => (
                <span {...getTokenProps({ token })} key={key} />
              ))}
            </div>
          ))}
        </pre>
      )}
    </Highlight>
  );
}

function Diag({ code }: { code: string }) {
  return <pre className="diag">{code}</pre>;
}

function useActiveHeading(ids: string[]) {
  const [active, setActive] = useState<string>(ids[0] ?? "");
  useEffect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((e) => e.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
        if (visible[0]) setActive(visible[0].target.id);
      },
      { rootMargin: "-80px 0px -60% 0px", threshold: 0 },
    );
    ids.forEach((id) => {
      const el = document.getElementById(id);
      if (el) observer.observe(el);
    });
    return () => observer.disconnect();
  }, [ids]);
  return active;
}

function flattenIds(toc: Section[]): string[] {
  const out: string[] = [];
  for (const s of toc) {
    out.push(s.id);
    if (s.children) out.push(...s.children.map((c) => c.id));
  }
  return out;
}

export function Docs() {
  const active = useActiveHeading(flattenIds(TOC));

  return (
    <div className="docs">
      <aside className="docs-toc">
        <nav>
          <ul>
            {TOC.map((s) => (
              <li key={s.id}>
                <a
                  href={`#${s.id}`}
                  className={active === s.id ? "active" : ""}
                >
                  {s.title}
                </a>
                {s.children && (
                  <ul className="docs-toc-sub">
                    {s.children.map((c) => (
                      <li key={c.id}>
                        <a
                          href={`#${c.id}`}
                          className={active === c.id ? "active" : ""}
                        >
                          {c.title}
                        </a>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            ))}
          </ul>
        </nav>
      </aside>

      <main className="docs-body">
        <section id="overview">
          <span className="section-kicker">documentation</span>
          <h1>pvmsafe</h1>
          <p className="docs-lede">
            A Rust proc-macro crate that wraps{" "}
            <code>pvm_contract_macros::contract</code> and refuses to compile
            smart contracts whose safety properties cannot be proven. Targets
            pallet-revive / PolkaVM.
          </p>
          <p>
            Every check is discharged by Fourier–Motzkin elimination over
            linear integer arithmetic. No SMT solver, no runtime checks — the
            annotations are stripped before codegen, so the emitted PolkaVM
            bytecode is identical to the unannotated version.
          </p>
        </section>

        <section id="install">
          <h2>Install</h2>
          <p>
            Add <code>pvmsafe</code> alongside the pallet-revive contract
            macros in your contract crate's <code>Cargo.toml</code>:
          </p>
          <Code
            lang="toml"
            code={`[dependencies]
pvm-contract-macros = { git = "https://github.com/paritytech/cargo-pvm-contract" }
pvmsafe = "0.1"`}
          />
        </section>

        <section id="quickstart">
          <h2>Quick start</h2>
          <p>
            Wrap your contract module with <code>#[pvmsafe::contract]</code>{" "}
            (as the outer attribute) and the existing{" "}
            <code>#[pvm_contract_macros::contract(...)]</code> underneath:
          </p>
          <Code
            code={`#[pvmsafe::contract]
#[pvmsafe::invariant(conserves)]
#[pvm_contract_macros::contract("ERC20.sol", allocator = "pico")]
mod erc20 {
    #[pvm_contract_macros::method]
    pub fn transfer(
        #[pvmsafe::refine(amount > 0)] amount: U256,
        #[pvmsafe::unchecked] to: Address,
    ) -> Result<(), Error> {
        let sender_bal = load(&balance_key(&caller));
        if sender_bal < amount {
            return Err(Error::InsufficientBalance);
        }
        #[pvmsafe::refine(v >= amount)]
        let safe_bal = sender_bal;

        #[pvmsafe::delta(-amount)]
        store(&balance_key(&caller), safe_bal - amount);

        let to_bal = load(&balance_key(&to));
        #[pvmsafe::delta(amount)]
        store(&balance_key(&to), to_bal.saturating_add(amount));
        Ok(())
    }
}`}
          />
          <p>
            Run <code>cargo build</code>. If anything can't be proven safe, the
            build fails with a diagnostic pointing at the offending line.
          </p>
        </section>

        <section id="attributes">
          <h2>Attributes</h2>
          <p>
            Nine attributes drive the checker. All live under the{" "}
            <code>pvmsafe::</code> prefix, including the outer{" "}
            <code>pvmsafe::contract</code>, and are stripped from the
            expansion before the inner contract macro runs.
          </p>

          <h3 id="attr-contract">
            <code>#[pvmsafe::contract]</code>
          </h3>
          <p>
            Module-level. The entry point — enables all checks on the wrapped
            module and hands the stripped code to{" "}
            <code>pvm_contract_macros::contract</code>.
          </p>

          <h3 id="attr-refine">
            <code>#[pvmsafe::refine(predicate)]</code>
          </h3>
          <p>
            On function parameters or <code>let</code> bindings. Asserts that a
            linear arithmetic predicate holds for the bound value. On a{" "}
            <code>let</code>, <code>v</code> stands for the initializer; on a
            parameter, the parameter name is used directly.
          </p>
          <Code
            code={`fn transfer(#[pvmsafe::refine(amount > 0)] amount: u64) { /* ... */ }

#[pvmsafe::refine(v >= amount)]
let safe_balance = sender_balance;`}
          />

          <h3 id="attr-ensures">
            <code>#[pvmsafe::ensures(predicate)]</code>
          </h3>
          <p>
            On function definitions. Postcondition on the return value. Inside
            the predicate, <code>v</code> refers to the returned expression.
          </p>
          <Code
            code={`#[pvmsafe::ensures(v > 0)]
fn mint(amount: u64) -> u64 { amount }`}
          />

          <h3 id="attr-invariant">
            <code>#[pvmsafe::invariant(conserves[(groups)])]</code>
          </h3>
          <p>
            Module-level. Declares one or more conservation groups. Every
            entrypoint in the module must have its declared{" "}
            <code>delta</code> contributions sum to zero on every exit path.
          </p>
          <Code
            code={`// single default group
#[pvmsafe::invariant(conserves)]
mod erc20 { /* ... */ }

// explicit named groups
#[pvmsafe::invariant(conserves(balances, shares))]
mod vault { /* ... */ }`}
          />

          <h3 id="attr-delta">
            <code>#[pvmsafe::delta(expr)]</code>
          </h3>
          <p>
            On expression statements inside an entrypoint. Contributes a signed
            linear term to the function's delta accumulator. If the module
            declares named groups, use <code>group = expr</code>.
          </p>
          <Code
            code={`#[pvmsafe::delta(-amount)]
store(&balance_key(&caller), new_sender);

#[pvmsafe::delta(amount)]
store(&balance_key(&to), new_recipient);`}
          />

          <h3 id="attr-given">
            <code>#[pvmsafe::given(predicate)]</code>
          </h3>
          <p>
            On expressions. Adds a temporary assumption to the proof context
            for that expression only. Useful when a fact is known from an
            external invariant but isn't expressible via{" "}
            <code>refine</code>.
          </p>
          <Code
            code={`#[pvmsafe::given(x > 0)]
let result = risky_op(x);`}
          />

          <h3 id="attr-unchecked">
            <code>#[pvmsafe::unchecked]</code>
          </h3>
          <p>
            On entrypoint parameters. Explicit opt-out — marks the parameter as
            intentionally unverified. Required on every entrypoint parameter
            that does not carry a <code>refine</code>.
          </p>
          <Code
            code={`#[pvm_contract_macros::method]
pub fn balance_of(#[pvmsafe::unchecked] account: Address) -> U256 { /* ... */ }`}
          />

          <h3 id="attr-effect">
            <code>#[pvmsafe::effect(...)]</code>
          </h3>
          <p>
            On function items. Declares the effect set of the function over
            the five atoms{" "}
            <code>read</code>, <code>write</code>, <code>call</code>,{" "}
            <code>emit</code>, <code>revert</code>. Use <code>pure</code> or
            empty parens to assert no effects. Declared sets are authoritative
            at callsites; undeclared functions have their effect set inferred
            bottom-up via a call-graph fixpoint. The CEI check rejects any
            entrypoint whose CFG has a <code>write</code> or <code>emit</code>{" "}
            after a <code>call</code>.
          </p>
          <Code
            code={`#[pvmsafe::effect(write)]
fn set_balance(addr: &[u8; 20], amount: U256) { /* ... */ }

#[pvmsafe::effect(call)]
fn send_value(to: &[u8; 20], amount: U256) { /* ... */ }

#[pvmsafe::effect(read)]
fn load_u256(key: &[u8; 32]) -> U256 { /* ... */ }`}
          />

          <h3 id="attr-effect-allow">
            <code>#[pvmsafe::effect_allow(...)]</code>
          </h3>
          <p>
            On function items. Per-rule escape hatch for the CEI check.{" "}
            <code>effect_allow(write_after_call)</code> disables the
            write-after-call rejection;{" "}
            <code>effect_allow(emit_after_call)</code> disables the
            emit-after-call rejection. Auditable via <code>grep</code> — the
            whole-function <code>unsafe</code> hammer is deliberately avoided.
          </p>
        </section>

        <section id="rules">
          <h2>Verification rules</h2>

          <h3 id="rule-arith">Arithmetic safety</h3>
          <p>
            Every binary arithmetic operator on an unsigned integer type is
            required to be provably safe: addition and multiplication must not
            overflow; subtraction must not underflow; division and modulo must
            have a non-zero divisor. The proof obligation is discharged against
            the current assumption set via Fourier–Motzkin elimination.
          </p>

          <h3 id="rule-refine">Refinement discharge</h3>
          <p>
            At each call site, the arguments passed to parameters with a{" "}
            <code>refine</code> predicate must entail that predicate. The
            checker substitutes the argument expressions into the predicate and
            asks the FM engine whether the negation is contradictory.
          </p>

          <h3 id="rule-conserve">Conservation of value</h3>
          <p>
            When a module declares <code>#[pvmsafe::invariant(conserves)]</code>
            , every entrypoint's <code>delta</code> contributions must sum to
            exactly zero on every control-flow path. The checker tracks the
            accumulated symbolic sum and emits an error when a return site has
            a non-zero net delta that isn't provably balanced.
          </p>

          <h3 id="rule-cei">CEI ordering</h3>
          <p>
            Each function declares its effect set via{" "}
            <code>#[pvmsafe::effect(...)]</code> over the atoms{" "}
            <code>read</code>, <code>write</code>, <code>call</code>,{" "}
            <code>emit</code>, <code>revert</code>. The macro infers undeclared
            sets bottom-up via a call-graph fixpoint. The CEI check walks the
            CFG of every entrypoint with branch merging and pessimistic loop
            treatment, and rejects any <code>write</code> or <code>emit</code>{" "}
            that appears after a <code>call</code> on any path. This is the
            classic Checks-Effects-Interactions rule — state changes before
            calls out, never the other way round. Opt out per-rule with{" "}
            <code>#[pvmsafe::effect_allow(write_after_call)]</code> or{" "}
            <code>effect_allow(emit_after_call)</code> for callback handlers
            that genuinely need the reversed order.
          </p>

          <h3 id="rule-entry">Entrypoint coverage</h3>
          <p>
            Every parameter on a{" "}
            <code>#[pvm_contract_macros::method]</code>,{" "}
            <code>constructor</code>, or <code>fallback</code> function must
            carry either a <code>refine</code> or{" "}
            <code>#[pvmsafe::unchecked]</code>. This forces an explicit decision
            at the trust boundary; no silent holes.
          </p>
        </section>

        <section id="errors">
          <h2>Compile errors</h2>
          <p>
            Every rejection comes with a span pointing at the offending
            expression and a hint about how to satisfy the checker. A
            representative sample:
          </p>

          <h3>Underflow on an unguarded subtraction</h3>
          <Code
            code={`fn f(a: u64, b: u64) {
    let _ = a - b;
}`}
          />
          <Diag
            code={`error: pvmsafe: subtraction \`a - b\` may underflow;
       not provable from caller's assumptions
  --> src/lib.rs:2:13
   |
 2 |     let _ = a - b;
   |             ^^^^^`}
          />

          <h3>Division by zero</h3>
          <Code
            code={`fn f(x: u64, y: u64) {
    let _ = x / y;
}`}
          />
          <Diag
            code={`error: pvmsafe: \`x / y\` may divide by zero;
       divisor not provably non-zero
  --> src/lib.rs:2:13
   |
 2 |     let _ = x / y;
   |             ^^^^^`}
          />

          <h3>Refinement not proven at call site</h3>
          <Code
            code={`fn caller(x: u64) { callee(x); }
fn callee(#[pvmsafe::refine(x > 0)] x: u64) {}`}
          />
          <Diag
            code={`error: pvmsafe: refinement \`x > 0\` not provable
       from caller's assumptions
  --> src/lib.rs:1:21
   |
 1 | fn caller(x: u64) { callee(x); }
   |                     ^^^^^^^^^`}
          />

          <h3>CEI violation (reentrancy risk)</h3>
          <Code
            code={`#[pvmsafe::effect(call)]
fn send_value(to: &[u8; 20], amount: U256) { /* ... */ }

#[pvmsafe::effect(write)]
fn set_balance(addr: &[u8; 20], amount: U256) { /* ... */ }

fn withdraw(amount: U256) {
    let caller = get_caller();
    send_value(&caller, amount);
    set_balance(&caller, U256::ZERO);
}`}
          />
          <Diag
            code={`error: pvmsafe: state write after external call; reentrancy risk
  --> src/lib.rs:10:5
   |
10 |     set_balance(&caller, U256::ZERO);
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
note: earlier call effect here
  --> src/lib.rs:9:5
   |
 9 |     send_value(&caller, amount);
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^`}
          />

          <h3>Entrypoint parameter uncovered</h3>
          <Code
            code={`#[pvm_contract_macros::method]
pub fn transfer(amount: u64) {}`}
          />
          <Diag
            code={`error: pvmsafe: parameter \`amount\` on entrypoint \`transfer\`
       must carry \`#[pvmsafe::refine(...)]\` or
       \`#[pvmsafe::unchecked]\``}
          />

          <h3>Conservation invariant violated</h3>
          <Code
            code={`#[pvmsafe::invariant(conserves)]
mod m {
    #[pvm_contract_macros::method]
    pub fn f(#[pvmsafe::refine(amount > 0)] amount: u64) {
        #[pvmsafe::delta(-amount)]
        a(amount);
        // no matching positive delta — invariant breaks
    }
}`}
          />
          <Diag
            code={`error: pvmsafe: conservation invariant violated in \`f\`;
       declared deltas do not sum to zero on this path`}
          />
        </section>

        <section id="examples">
          <h2>Example contracts</h2>
          <p>
            The <code>pvmsafe-example</code> crate ships five contracts that
            demonstrate each rule end-to-end:
          </p>
          <ul className="docs-list">
            <li>
              <strong>erc20-full</strong> — full ERC-20 with parameter
              refinements, conservation deltas, and CEI annotations.
            </li>
            <li>
              <strong>erc20-vulnerable</strong> — the same contract without
              pvmsafe annotations; the ERC-20 transfer underflows when
              exploited at runtime.
            </li>
            <li>
              <strong>vault</strong> — a share-based vault with conservation
              between <code>balances</code> and <code>shares</code> groups.
            </li>
            <li>
              <strong>vault-vulnerable</strong> — the broken vault that forgets
              to decrement the caller's share slot; drains on withdraw.
            </li>
            <li>
              <strong>macro-pico-alloc</strong> — minimal allocator fixture
              showing the inner macro composition.
            </li>
          </ul>
          <p>
            The two verified/vulnerable pairs (erc20 and vault) drive the{" "}
            <a href="/demo">live demo</a>.
          </p>
        </section>
      </main>
    </div>
  );
}

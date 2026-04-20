import { Link } from "react-router-dom";
import { Highlight, themes } from "prism-react-renderer";

const VULN_CODE = `// ❌ ships vulnerable — cargo build succeeds
mod erc20 {
    pub fn transfer(to: Address, amount: U256) {
        let sender = load(&balance_key(&caller));
        let new_sender = sender - amount;
        //               ^^^^^^^^^^^^^^^^
        // no guard — underflows to 2^256 - 1
        // when sender < amount
        store(&balance_key(&caller), new_sender);
        store(&balance_key(&to), load(&balance_key(&to)) + amount);
    }
}`;

const SAFE_CODE = `// ✓ verified — cargo build rejects unguarded subtraction
#[pvmsafe::contract]
mod erc20 {
    pub fn transfer(
        #[pvmsafe::refine(amount > 0)] amount: U256,
        to: Address,
    ) {
        let sender = load(&balance_key(&caller));
        if sender < amount { return Err(Insufficient); }

        #[pvmsafe::refine(v >= amount)]
        let safe = sender;
        // ✓ proved by Fourier–Motzkin
        let new_sender = safe - amount;
        store(&balance_key(&caller), new_sender);
    }
}`;

const COMPILE_ERROR = `error: pvmsafe: subtraction \`sender - amount\` may underflow; not provable from caller's assumptions
  --> src/erc20.rs:5:26
   |
 5 |         let new_sender = sender - amount;
   |                          ^^^^^^^^^^^^^^^^`;

type Feature = {
  title: string;
  blurb: string;
  tag: string;
};

const FEATURES: Feature[] = [
  {
    title: "Arithmetic safety",
    tag: "refine",
    blurb:
      "Every unchecked subtraction, addition, multiplication, division, and modulo is rejected at build time unless proven safe via linear arithmetic.",
  },
  {
    title: "Refinement types",
    tag: "refine / ensures",
    blurb:
      "Annotate parameters, locals, and return values with predicates. Call-site arguments must discharge the callee's refinements via Fourier–Motzkin elimination.",
  },
  {
    title: "Conservation of value",
    tag: "invariant / delta",
    blurb:
      "Declare a module `conserves` and annotate each storage write with its delta. The sum must be zero on every path — or it will not compile.",
  },
  {
    title: "CEI ordering",
    tag: "effect(read|write|call|emit|revert)",
    blurb:
      "Each function declares its effect set; pvmsafe infers callers via call-graph fixpoint and rejects any `write` or `emit` that follows a `call` in the CFG. Opt out per-rule with `effect_allow(write_after_call)`.",
  },
  {
    title: "Entrypoint coverage",
    tag: "refine / unchecked",
    blurb:
      "Every entrypoint parameter must carry an explicit refinement or an `#[pvmsafe::unchecked]` marker. No silent holes.",
  },
  {
    title: "Zero runtime cost",
    tag: "proc-macro",
    blurb:
      "All verification is compile-time. The annotations are stripped before codegen — the PolkaVM bytecode is identical to the unannotated version.",
  },
];

export function Home() {
  return (
    <div className="home">
      <section className="hero">
        <p className="hero-eyebrow">
          <span className="brand-dot" />
          compile-time verification for pallet-revive contracts
        </p>
        <h1 className="hero-title">
          Your contract won't ship
          <br />
          if it can't be <em>proven safe</em>.
        </h1>
        <p className="hero-lede">
          <strong>pvmsafe</strong> is a Rust proc-macro crate that verifies
          smart-contract safety properties - arithmetic overflow, reentrancy,
          conservation of value - at <code>cargo build</code> time. The
          vulnerable version you'd normally ship simply <em>will not compile</em>.
        </p>
        <div className="hero-ctas">
          <Link to="/docs" className="btn btn-primary">
            Read the docs
          </Link>
          <Link to="/demo" className="btn btn-ghost">
            See the runtime demo →
          </Link>
        </div>
      </section>

      <section className="hero-code">
        <div className="hero-code-header">
          <div>
            <h2>The value is in the error you never see ship.</h2>
            <p>
              Identical runtime semantics. The only difference: whether{" "}
              <code>cargo build</code> accepts the code.
            </p>
          </div>
        </div>
        <div className="hero-code-grid">
          <div className="hero-code-col hero-code-vuln">
            <div className="hero-code-head">
              <span className="tag tag-danger">without pvmsafe</span>
              <span className="hero-code-note">
                compiles clean — ships exploitable
              </span>
            </div>
            <Highlight theme={themes.vsDark} code={VULN_CODE} language="rust">
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
          </div>
          <div className="hero-code-col hero-code-safe">
            <div className="hero-code-head">
              <span className="tag tag-safe">with pvmsafe</span>
              <span className="hero-code-note">
                refinement proven — build succeeds
              </span>
            </div>
            <Highlight theme={themes.vsDark} code={SAFE_CODE} language="rust">
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
          </div>
        </div>

        <div className="terminal">
          <div className="terminal-head">
            <span className="terminal-dots">
              <span /> <span /> <span />
            </span>
            <span className="terminal-title">
              $ cargo build --release  ·  strip the pvmsafe annotations and
              this is what you'd get
            </span>
          </div>
          <pre className="terminal-body">{COMPILE_ERROR}</pre>
        </div>
      </section>

      <section className="features">
        <div className="features-intro">
          <span className="section-kicker">what it checks</span>
          <h2>Six properties, enforced by the compiler.</h2>
          <p>
            Each property is discharged by Fourier–Motzkin elimination over
            linear integer arithmetic. No SMT solver, no runtime instrumentation,
            no theorem-prover incantations — just proc-macros and entailment.
          </p>
        </div>
        <div className="features-grid">
          {FEATURES.map((f) => (
            <article key={f.title} className="feature-card">
              <div className="feature-card-head">
                <h3>{f.title}</h3>
                <code className="feature-tag">{f.tag}</code>
              </div>
              <p>{f.blurb}</p>
            </article>
          ))}
        </div>
      </section>

      <section className="cta-row">
        <div className="cta-card">
          <h3>Reference</h3>
          <p>
            Every attribute, every verification rule, every shape of compile
            error — with the exact diagnostic text you'll see in your terminal.
          </p>
          <Link to="/docs" className="btn btn-primary">
            Open the documentation
          </Link>
        </div>
        <div className="cta-card cta-card-demo">
          <h3>Live demo</h3>
          <p>
            Two contracts on a real pallet-revive dev-node — one verified by
            pvmsafe, one not. Run the same attack on both and watch only one
            succeed.
          </p>
          <Link to="/demo" className="btn btn-ghost">
            Launch the demo
          </Link>
        </div>
      </section>
    </div>
  );
}
